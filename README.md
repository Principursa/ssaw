# SSAW

Shark's Secure Agent Wallet.

SSAW is a local Ethereum wallet built for AI-assisted workflows. It is one binary with two roles:

- `ssaw ...`: a direct CLI for humans, scripts, and local debugging
- `ssaw serve`: a stdio MCP server that exposes wallet tools to an agent

The main goal is to keep wallet secrets out of the normal AI interaction stream. Mnemonics are stored encrypted on disk, and normal usage does not require putting secrets into chat logs, shell arguments, or environment variables.

SSAW is not trying to be a hardened enclave. It is trying to be a practical local wallet boundary for agent workflows.


## What SSAW Does

- dedicated SSAW `age` identity at `~/.config/ssaw/identity.txt`
- encrypted seed storage per project
- project-aware wallet selection
- address derivation from a BIP-39 mnemonic
- project-local address aliases
- plain message signing
- EIP-712 typed data signing
- project-local chain configuration
- native ETH transaction sending
- runtime ABI contract reads
- runtime ABI contract writes
- stdio MCP server with wallet tools
- cross-process write locking for transaction submission

## When To Use Which Mode

Use the CLI when you want to:

- initialize or import a wallet
- inspect local state
- switch projects
- debug a chain or contract call by hand
- run wallet commands from scripts

Use MCP mode when you want to:

- give an agent controlled wallet capabilities through stdio
- let an agent derive addresses, sign payloads, or submit transactions
- keep the same wallet logic and project model, but expose it as tools instead of shell commands

The important point is that both modes use the same underlying wallet state and the same project layout.

## Quick Start

If you are using a local checkout, run SSAW with Cargo:

```sh
cargo run -- --help
```

Initialize a wallet:

```sh
cargo run -- init
```

Create and select a project:

```sh
cargo run -- project init dex
```

Add a chain:

```sh
printf '%s' 'http://127.0.0.1:8545' | cargo run -- add-chain local 31337 --rpc-url-stdin
```

Start the MCP server for that project:

```sh
cargo run -- --project dex serve
```

If you want a built binary instead of `cargo run`, build the crate and run the executable directly:

```sh
cargo build --release
./target/release/ssaw --help
```

## First-Time Setup

Initialize a new wallet in the current project:

```sh
cargo run -- init
```

That will:

- create the SSAW identity if needed
- generate a mnemonic
- encrypt it to disk
- print the mnemonic once for backup
- print address index `0`

Import an existing mnemonic from stdin:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- import
```

Import a mnemonic that uses a BIP-39 passphrase:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- import --prompt-passphrase
```

The passphrase is not persisted with the seed. Prompt for it on signer-dependent CLI commands, or unlock `serve` at startup for a smoother agent workflow.

Check where SSAW is storing state:

```sh
cargo run -- doctor
```

## Nix

If you use Nix, SSAW also ships with a dev shell and package entrypoint.

Enter the dev shell:

```sh
nix develop
```

Inside that shell:

- `ssaw` is available on `PATH`
- `anvil` is available for local testing
- the shell `ssaw` command tracks your local checkout

Run the packaged app directly:

```sh
nix run
```

## Project Model

Projects are the main wallet boundary in SSAW.

Each project has its own:

- encrypted seed
- chain configuration
- alias metadata
- wallet lock file

Current layout:

```text
~/.ssaw/
  current-project
  seed.age                  # default project
  chains.toml               # default project
  addresses.toml            # default project
  wallet.lock               # default project
  projects/
    dex/
      seed.age
      chains.toml
      addresses.toml
      wallet.lock

~/.config/ssaw/
  config.toml
  identity.txt
```

The `default` project lives directly under `~/.ssaw/`. Named projects live under `~/.ssaw/projects/<name>/`.

Create and select a new project:

```sh
cargo run -- project init dex
```

Import directly into a named project:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- project import dex
```

If that project uses a BIP-39 passphrase, add `--prompt-passphrase`.

Switch projects:

```sh
cargo run -- project use default
cargo run -- project use dex
```

Inspect project state:

```sh
cargo run -- project list
cargo run -- project current
```

Override the active project for a single command:

```sh
cargo run -- --project dex address
cargo run -- --project dex serve
```

Recommended daily flow:

- choose the active project with `project use`
- run normal commands against that project
- use `--project` only for explicit overrides and scripts

## CLI Usage

### Addresses

Derive the default address:

```sh
cargo run -- address
```

If the project requires a BIP-39 passphrase:

```sh
cargo run -- address --prompt-passphrase
```

Derive another index:

```sh
cargo run -- address --index 1
```

### Aliases

Aliases are project-local names for derivation indices.

Create an alias:

```sh
cargo run -- alias set deployer --index 0 --label deployer --label admin
```

List aliases:

```sh
cargo run -- alias list
```

Show one alias:

```sh
cargo run -- alias show deployer
```

Use an alias anywhere an address target is accepted:

```sh
cargo run -- address --alias deployer
cargo run -- sign-message "hello" --alias deployer
```

### Signing

Sign a plain message:

```sh
cargo run -- sign-message "hello"
```

Sign EIP-712 typed data from stdin:

```sh
printf '%s' '{"types":{"EIP712Domain":[{"name":"name","type":"string"}],"Mail":[{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"SSAW"},"message":{"contents":"hello"}}' | cargo run -- sign-typed-data
```

### Chains

Add a project-local chain:

```sh
printf '%s' 'http://127.0.0.1:8545' | cargo run -- add-chain local 31337 --rpc-url-stdin
```

List configured chains:

```sh
cargo run -- list-chains
```

`list-chains` prints chain names and ids without echoing stored RPC endpoints.

Chain config is project-local. If `dex` and `launchpad` both use Anvil, each project still needs its own `add-chain` entry.

### Transactions

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

Current behavior:

- by default, `send-transaction` prints only the transaction hash
- with `--wait`, it prints JSON including confirmation state and receipt summary
- writes take an exclusive project-local lock so concurrent processes serialize instead of racing

Notes:

- a freshly initialized wallet will not be funded on Anvil by default
- fund the derived address first, or import a funded mnemonic
- transactions use the active project unless `--project` is passed

### Contract Reads And Writes

SSAW reads ABI JSON from stdin and accepts repeated `--arg` values as Solidity-like string literals.

Read a contract function:

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

Wait for a contract write receipt:

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
  --function deposit \
  --abi-stdin \
  --value-wei 1000000000000000
```

Current behavior:

- function resolution is by name and uses the first ABI match
- outputs are printed as JSON values
- integers are rendered as decimal strings
- byte values are rendered as `0x` hex strings
- `write-contract` returns only `tx_hash` by default
- with `--wait`, it returns JSON including confirmation state and receipt summary

## MCP Server Usage

Run the stdio MCP server:

```sh
cargo run -- serve
```

Run it against a specific project:

```sh
cargo run -- --project dex serve
```

Unlock a passphrase-protected project for the lifetime of the server process:

```sh
cargo run -- --project dex serve --prompt-passphrase
```

`ssaw serve` speaks line-oriented JSON-RPC 2.0 over stdio and implements the MCP handshake. Each server process is scoped to its selected project, and requests are handled sequentially.

When started with `--prompt-passphrase`, the server keeps that passphrase only in process memory. It is not written back to disk with the seed.

Across multiple CLI and server processes sharing the same project, write operations are serialized by that project's wallet lock file.

Current MCP methods:

- `initialize`
- `notifications/initialized`
- `ping`
- `tools/list`
- `tools/call`

Current wallet tools:

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

Address-targeting tools accept either:

- `index`
- `alias`

Server tool calls act on the project selected when `serve` started. If you need another project, start another server with a different `--project` selection.

Signer-targeting tool responses include alias/index metadata when available. Chain-management responses do not echo stored RPC endpoints. `doctor` reports `passphrase_required`, `signer_unlocked`, and `server_project_scope` so an agent can tell whether signer-dependent operations are currently available.

### Minimal Handshake Example

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | cargo run -- serve
```

### MCP `get_address` Example

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_address","arguments":{"alias":"deployer"}}}' \
  | cargo run -- serve
```

### MCP `send_transaction` Example

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_transaction","arguments":{"chain":"local","to":"0x000000000000000000000000000000000000dead","value_wei":"1","index":0}}}' \
  | cargo run -- serve
```

### MCP `read_contract` Example

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_contract","arguments":{"chain":"local","address":"0xYourContract","function":"balanceOf","abi":[{"type":"function","name":"balanceOf","stateMutability":"view","inputs":[{"name":"owner","type":"address"}],"outputs":[{"name":"","type":"uint256"}]}],"args":["0x000000000000000000000000000000000000dead"]}}}' \
  | cargo run -- serve
```

## Common Workflow

For a new user, the easiest path is:

1. `nix develop`
2. `cargo run -- project init <name>`
3. `cargo run -- add-chain <chain-name> <chain-id> --rpc-url-stdin`
4. Use CLI commands directly while you validate addresses, aliases, signing, and chain setup
5. Once that works, point your agent at `cargo run -- --project <name> serve`

That keeps the initial debugging human-readable and then shifts to MCP once the wallet state is known-good.

## Testing

Run the full test suite:

```sh
nix develop --no-update-lock-file -c cargo test
```

The test suite includes:

- CLI, project, alias, and MCP integration coverage in `tests/cli_flow.rs`
- Anvil-backed transaction and contract coverage in `tests/anvil_flow.rs`

For isolated local testing, use a temporary home directory:

```sh
export HOME=/tmp/ssaw-test
rm -rf "$HOME"
mkdir -p "$HOME"
nix develop
```

## Security Notes

SSAW makes a narrower claim than a hardware wallet or a hardened signing service.

It is designed to reduce accidental secret exposure in:

- chat transcripts
- copied commands
- shell history
- environment variables
- normal MCP tool responses

It does not claim to defend against:

- a malicious agent that intentionally misuses the wallet
- root on the host
- same-user process introspection
- full host compromise
