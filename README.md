# comdirect-ynab

Sync comdirect Girokonto transactions to YNAB using a local CLI.

## Requirements

- Rust toolchain
- 1Password CLI (`op`) with a Service Account token
- comdirect REST API credentials
- YNAB personal access token

## Build

```sh
cargo build --release
```

The binary is located at `target/release/comdirect-ynab`.

## Setup

1. Export the 1Password Service Account token for the current shell:

```sh
export OP_SERVICE_ACCOUNT_TOKEN="..."
```

2. Run the initializer to create the config and select your YNAB account:

```sh
./target/release/comdirect-ynab init
```

The config is stored at `~/Library/Application Support/comdirect-ynab/config.toml` and only contains
`op://` references, not secrets.

3. Run a one-off sync (you will be prompted for a TAN if needed):

```sh
./target/release/comdirect-ynab sync
```

## Notes

- The sync uses a 30 day lookback window.
- If the session TAN expires, `sync` will guide you through a TAN challenge before importing.
