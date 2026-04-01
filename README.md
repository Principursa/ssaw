# SSAW

SSAW is a local Ethereum wallet for agent workflows.

It keeps wallet seed material encrypted on disk, exposes a focused CLI, and can run as a stdio MCP server so agents can derive addresses, sign payloads, and submit transactions without handling raw private keys or mnemonics directly.

## Scope

SSAW is designed to keep wallet secrets out of:

- chat transcripts
- shell history
- copied command lines
- MCP tool responses

This is not a hardened enclave or host-compromise defense. It is a pragmatic local wallet boundary for AI-assisted development.

The design notes live in [SPEC.md](./SPEC.md). This README documents current behavior.

## Current Capabilities

- dedicated SSAW age identity at `~/.config/ssaw/identity.txt`
- encrypted wallet seed per project
- active project selection plus explicit `--project` overrides
- address derivation by index or alias
- project-local address aliases with labels
- message signing
- EIP-712 typed data signing
- project-local named chain configuration
- native transaction sending
- ABI-driven contract reads
- ABI-driven contract writes
- stdio MCP server with wallet and chain-management tools
- cross-process locking for transaction submission

## Project Model

Projects are the main separation boundary in SSAW.

Each project has its own:

- encrypted seed
- chain configuration
- alias metadata

By default, CLI commands use the currently selected project. You can override that with `--project` when scripting or comparing projects side by side.

Disk layout:

```text
~/.ssaw/
  projects/
    <project>/
      seed.age
      chains.toml
      addresses.toml
  current-project

~/.config/ssaw/
  config.toml
  identity.txt
```

The `default` project uses `~/.ssaw/` directly for backward compatibility.

## Development Environment

Enter the dev shell:

```sh
nix develop
```

Inside the shell:

- `cargo`, `rustc`, `rustfmt`, and `clippy` are available
- `anvil` is available for local chain testing
- `ssaw` is a thin wrapper around `cargo run --`

Run the packaged application directly:

```sh
nix run
```

## Quick Start

Initialize a new wallet in the current project:

```sh
cargo run -- init
```

Import an existing mnemonic from stdin:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- import
```

Inspect resolved paths and file presence:

```sh
cargo run -- doctor
```

Show the first derived address:

```sh
cargo run -- address
```

## Working With Projects

Create and switch to a project:

```sh
cargo run -- project init dex
```

Import directly into a named project:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- project import dex
```

Switch projects:

```sh
cargo run -- project use default
cargo run -- project use dex
```

List projects and show the active one:

```sh
cargo run -- project list
cargo run -- project current
```

Override the selected project for a single command:

```sh
cargo run -- --project dex address
```

## Address Aliases

Aliases are project-local names for derivation indices.

Create an alias:

```sh
cargo run -- alias set deployer --index 0 --label deployer --label admin
```

Inspect aliases:

```sh
cargo run -- alias list
cargo run -- alias show deployer
```

Use an alias anywhere an address index is accepted:

```sh
cargo run -- address --alias deployer
cargo run -- sign-message "hello" --alias deployer
```

## Chain Configuration

Chains are stored per project. Transaction and contract commands require the target chain to be configured in that project first.

Add a chain:

```sh
printf '%s' 'http://127.0.0.1:8545' | cargo run -- add-chain local 31337 --rpc-url-stdin
```

List configured chains:

```sh
cargo run -- list-chains
```

## Signing

Sign a plain message:

```sh
cargo run -- sign-message "hello"
```

Sign EIP-712 typed data from stdin:

```sh
printf '%s' '{"types":{"EIP712Domain":[{"name":"name","type":"string"}],"Mail":[{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"SSAW"},"message":{"contents":"hello"}}' | cargo run -- sign-typed-data
```

## Sending Transactions

Send native ETH:

```sh
cargo run -- send-transaction \
  --chain local \
  --to 0x000000000000000000000000000000000000dead \
  --value-wei 1
```

Wait for a receipt:

```sh
cargo run -- send-transaction \
  --chain local \
  --to 0x000000000000000000000000000000000000dead \
  --value-wei 1 \
  --wait \
  --timeout-secs 60
```

Notes:

- a newly initialized wallet is not funded automatically
- on Anvil, fund the derived address first or import a funded mnemonic
- chain configuration is project-local

## Contract Calls

Read a contract function using ABI JSON from stdin:

```sh
cat abi.json | cargo run -- read-contract \
  --chain local \
  --address 0xYourContract \
  --function balanceOf \
  --abi-stdin \
  --arg 0x000000000000000000000000000000000000dead
```

Write to a contract:

```sh
cat abi.json | cargo run -- write-contract \
  --chain local \
  --address 0xYourContract \
  --function transfer \
  --abi-stdin \
  --arg 0x000000000000000000000000000000000000dead \
  --arg 1
```

Wait for a write receipt:

```sh
cat abi.json | cargo run -- write-contract \
  --chain local \
  --address 0xYourContract \
  --function transfer \
  --abi-stdin \
  --arg 0x000000000000000000000000000000000000dead \
  --arg 1 \
  --wait \
  --timeout-secs 60
```

Attach native value to a payable call:

```sh
cat abi.json | cargo run -- write-contract \
  --chain local \
  --address 0xYourContract \
  --function mint \
  --abi-stdin \
  --value-wei 10000000000000000
```

## MCP Server

Start the stdio MCP server:

```sh
cargo run -- serve
```

The server speaks JSON-RPC 2.0 over stdio and supports the MCP handshake:

- `initialize`
- `notifications/initialized`
- `ping`
- `tools/list`
- `tools/call`

Current tool surface:

- `get_address`
- `list_addresses`
- `list_chains`
- `add_chain`
- `doctor`
- `sign_message`
- `sign_typed_data`
- `send_transaction`
- `read_contract`
- `write_contract`

Operational notes:

- most tools accept an optional `project` override
- chain configuration is project-local, including MCP calls
- `send_transaction`, `read_contract`, and `write_contract` require prior chain configuration
- `doctor` is intended for agent debugging and returns resolved paths, file presence, aliases, and configured chains

## Testing

Run the full test suite inside the dev shell:

```sh
cargo test
```

The integration tests cover:

- CLI project and alias flows
- MCP tool discovery and responses
- local-chain transaction and contract flows against Anvil
