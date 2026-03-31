# SSAW

Shark's Secure Agent Wallet.

SSAW is a local Ethereum wallet intended for agent use. It keeps wallet material encrypted on disk, exposes a small CLI, and can run a stdio MCP server for agent tooling.

## Current Status

Implemented today:

- dedicated SSAW age identity at `~/.config/ssaw/identity.txt`
- project-aware wallet storage
- address derivation from a BIP-39 mnemonic
- message signing
- EIP-712 typed data signing
- named chain configuration
- native transaction sending
- runtime ABI contract reads
- runtime ABI contract writes
- stdio MCP server with wallet tools
- cross-process write locking for transaction submission

Planned but not implemented yet:

- hardened MCP/server mode
- spending policy controls
- Foundry integration

The authoritative design notes are in [SPEC.md](./SPEC.md), but this README reflects what the code can actually do right now.

The intended daily flow is:

- use `project ...` to create or switch the active project
- run normal wallet commands against that active project
- use `--project` only as an explicit override when scripting or comparing projects side by side

## Setup

Enter the dev shell:

```sh
nix develop
```

Inside the dev shell, `ssaw` and `anvil` are available on `PATH`.
The in-shell `ssaw` command is a thin wrapper around `cargo run`, so it tracks your local checkout without requiring a separate install step.

You can also run the packaged app directly:

```sh
nix run
```

Initialize a new wallet:

```sh
cargo run -- init
```

Import an existing mnemonic from stdin:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- import
```

Check local state paths:

```sh
cargo run -- doctor
```

## Projects

SSAW now supports project selection.

Current layout:

- the legacy root wallet is the `default` project
- named projects live under `~/.ssaw/projects/<name>/`
- the selected project is stored in `~/.ssaw/current-project`

Create and switch to a project:

```sh
cargo run -- project init dex
```

That creates the project, initializes its wallet, and selects it.

Import directly into a project:

```sh
printf '%s\n' 'test test test test test test test test test test test junk' | cargo run -- project import imported
```

Switch between projects:

```sh
cargo run -- project use default
cargo run -- project use dex
```

List projects:

```sh
cargo run -- project list
```

Show the current project:

```sh
cargo run -- project current
```

You can also override the selected project per command, but that is meant more for scripts and one-off targeting than normal interactive use:

```sh
cargo run -- --project dex address
cargo run -- --project dex serve
```

## Aliases

Aliases are project-local names for derived address indices.

Set an alias:

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

Use an alias anywhere an address index matters:

```sh
cargo run -- address --alias deployer
cargo run -- sign-message "hello" --alias deployer
```

`list_addresses` and `get_address` responses also include alias metadata when available.

## Wallet Commands

Derive addresses:

```sh
cargo run -- address
cargo run -- address --index 1
```

Sign a plain message:

```sh
cargo run -- sign-message "hello"
```

Sign EIP-712 typed data from stdin:

```sh
printf '%s' '{"types":{"EIP712Domain":[{"name":"name","type":"string"}],"Mail":[{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"SSAW"},"message":{"contents":"hello"}}' | cargo run -- sign-typed-data
```

## Chain Configuration

Add a chain:

```sh
printf '%s' 'http://127.0.0.1:8545' | cargo run -- add-chain local 31337 --rpc-url-stdin
```

List configured chains:

```sh
cargo run -- list-chains
```

## Sending Transactions

Send native ETH:

```sh
cargo run -- send-transaction \
  --chain local \
  --to 0x000000000000000000000000000000000000dead \
  --value-wei 1
```

Wait for a receipt instead of returning only the transaction hash:

```sh
cargo run -- send-transaction \
  --chain local \
  --to 0x000000000000000000000000000000000000dead \
  --value-wei 1 \
  --wait \
  --timeout-secs 60
```

Note:

- a freshly initialized wallet will not be funded on Anvil by default
- fund the derived address first, or import a funded mnemonic
- transactions use the currently selected project unless `--project` is passed

## Contract Calls

SSAW accepts contract ABI JSON on stdin and repeated `--arg` values as Solidity literals.

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

- function resolution is by name, using the first ABI match
- outputs are printed as JSON values
- integers are rendered as decimal strings
- byte values are rendered as `0x` hex strings
- `send-transaction` and `write-contract` return only `tx_hash` by default
- with `--wait`, they return JSON including confirmation state and receipt summary
- write operations take an exclusive wallet lock, so concurrent processes serialize sends instead of racing

## MCP Server

Run the server:

```sh
cargo run -- serve
```

`ssaw serve` now speaks MCP over stdio using JSON-RPC 2.0 messages, one JSON message per line.

Within one `ssaw serve` process, requests are handled sequentially. Across multiple CLI/server processes sharing the same wallet, write operations are serialized by a project-local wallet lock file:

- `default`: `~/.ssaw/wallet.lock`
- named project: `~/.ssaw/projects/<name>/wallet.lock`

If you want a separate stdio wallet for a separate workstream, start `serve` with a different project:

```sh
cargo run -- --project dex serve
```

Tool arguments may include `"project": "<name>"` to target a project explicitly without starting a separate process for each one.

Current MCP methods:

- `initialize`
- `notifications/initialized`
- `ping`
- `tools/list`
- `tools/call`

Current wallet tools:

- `get_address`
- `list_addresses`
- `sign_message`
- `sign_typed_data`
- `send_transaction`
- `read_contract`
- `write_contract`

Address-targeting methods also accept `alias` in place of `index` for project-local alias lookup.

Minimal handshake example:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | cargo run -- serve
```

Call `get_address` through MCP:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_address","arguments":{"project":"dex","alias":"deployer"}}}' \
  | cargo run -- serve
```

Call `send_transaction` through MCP:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_transaction","arguments":{"chain":"local","to":"0x000000000000000000000000000000000000dead","value_wei":"1","index":0}}}' \
  | cargo run -- serve
```

Call `read_contract` through MCP:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_contract","arguments":{"chain":"local","address":"0xYourContract","function":"balanceOf","abi":[{"type":"function","name":"balanceOf","stateMutability":"view","inputs":[{"name":"owner","type":"address"}],"outputs":[{"name":"","type":"uint256"}]}],"args":["0x000000000000000000000000000000000000dead"]}}}' \
  | cargo run -- serve
```

## Testing

Run the test suite:

```sh
nix develop --no-update-lock-file -c cargo test
```

The test suite now includes:

- CLI/project/alias integration coverage in `tests/cli_flow.rs`
- Anvil-backed transaction and contract flow coverage in `tests/anvil_flow.rs`

For isolated local testing, use a temporary home directory:

```sh
export HOME=/tmp/ssaw-test
rm -rf "$HOME"
mkdir -p "$HOME"
nix develop
```

## Documentation Rule

When code changes add, remove, or materially change SSAW behavior, update this README in the same pass so the documented CLI and server surface stays accurate.
