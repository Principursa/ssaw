# SSAW

Shark's Secure Agent Wallet.

SSAW is a local Ethereum wallet intended for agent use. It keeps wallet material encrypted on disk, exposes a small CLI, and can run a line-oriented JSON stdio server for agent tooling.

## Current Status

Implemented today:

- dedicated SSAW age identity at `~/.config/ssaw/identity.txt`
- encrypted wallet seed at `~/.ssaw/seed.age`
- address derivation from a BIP-39 mnemonic
- message signing
- EIP-712 typed data signing
- named chain configuration
- native transaction sending
- runtime ABI contract reads
- runtime ABI contract writes
- stdio JSON server with wallet methods

Planned but not implemented yet:

- project-scoped wallets
- address aliases like `deployer` or `oracle`
- hardened MCP/server mode
- spending policy controls
- Foundry integration

The authoritative design notes are in [SPEC.md](./SPEC.md), but this README reflects what the code can actually do right now.

## Setup

Enter the dev shell:

```sh
nix develop
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

Note:

- a freshly initialized wallet will not be funded on Anvil by default
- fund the derived address first, or import a funded mnemonic

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

## Stdio Server

Run the server:

```sh
cargo run -- serve
```

Each request is one JSON line on stdin. Each response is one JSON line on stdout.

Currently supported methods:

- `get_address`
- `list_addresses`
- `sign_message`
- `sign_typed_data`
- `send_transaction`
- `read_contract`
- `write_contract`

Examples:

```sh
printf '%s\n' '{"id":1,"method":"get_address","params":{"index":0}}' | cargo run -- serve
```

```sh
printf '%s\n' '{"id":2,"method":"send_transaction","params":{"chain":"local","to":"0x000000000000000000000000000000000000dead","value_wei":"1","index":0}}' | cargo run -- serve
```

```sh
printf '%s\n' '{"id":3,"method":"read_contract","params":{"chain":"local","address":"0xYourContract","function":"balanceOf","abi":[{"type":"function","name":"balanceOf","stateMutability":"view","inputs":[{"name":"owner","type":"address"}],"outputs":[{"name":"","type":"uint256"}]}],"args":["0x000000000000000000000000000000000000dead"]}}' | cargo run -- serve
```

## Testing

Run the test suite:

```sh
nix develop --no-update-lock-file -c cargo test
```

For isolated local testing, use a temporary home directory:

```sh
export HOME=/tmp/ssaw-test
rm -rf "$HOME"
mkdir -p "$HOME"
nix develop
```

## Documentation Rule

When code changes add, remove, or materially change SSAW behavior, update this README in the same pass so the documented CLI and server surface stays accurate.
