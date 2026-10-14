# Quickstart

Install CLI 0.1.0, prepare the checked-in Vault artifacts and run one complete local migration test. The commands below assume a source checkout and a Linux/WSL terminal.

::: warning Distribution status
The GitHub repository has no published source yet. Follow [installation](/docs/installation) using the supplied local checkout until publication.
:::

## 1. Install the CLI

Follow [prerequisites](/docs/prerequisites) for Rust 1.98.0 and the system linker. From the repository root:

```bash
cargo +1.98.0 install --path crates/dg-cli --locked
export PATH="$HOME/.cargo/bin:$PATH"
doppelganger --version
```

No Anchor, Solana CLI, validator, RPC endpoint, API token or funded wallet is needed for the remaining steps.

## 2. Prepare a standalone Vault

This documentation includes a small helper that copies the real checked-in Vault binaries, IDLs, manifest and reviewed policy. It changes only artifact paths in the copied manifest. It does not build programs or modify the source fixtures.

```bash
python3 documentation/scripts/prepare-example.py --output runs/vault-project
```

The output must be a new directory. You now have:

```text
runs/vault-project/
├── doppelganger.toml
├── doppelganger-policy.toml
├── v1.bin
├── v2.bin
├── v1.json
└── v2.json
```

The binary contents are SBF ELF programs; the `.bin` suffix is not a different execution format. The manifest embeds fixture initialization and setup. No separate fixture file or snapshot is required for this run.

```bash
doppelganger doctor runs/vault-project --json
```

Expect `passed: true`. Missing optional build tools may produce WARN entries.

## 3. Inspect the static changes

```bash
doppelganger analyze --from runs/vault-project/v1.json \
  --to runs/vault-project/v2.json --account VaultV1:VaultV2
```

`--from` and `--to` select the two Anchor IDLs. `--account` pairs the renamed account types. The Vault grows from 48 to 56 serialized bytes and adds a `u64` withdrawal limit. Its discriminator and interface also change. These differences require review; they do not alone prove a failed migration.

## 4. Run the complete test

```bash
doppelganger test runs/vault-project --ci --output runs/vault-pass
```

Verified output:

```text
DOPPELGANGER MIGRATION TEST
Result: PASS
Exit code: 0
Report: runs/vault-pass/report.json
```

LiteSVM initializes and deposits into the V1 Vault, loads V2 at the same program ID and runs the configured migration. The engine then evaluates preservation, three declared relationships and seven fresh-world lifecycle scenarios. The reviewed policy handles the exact static changes.

The output directory must not already exist. For a second run, choose `runs/vault-pass-2`.

## 5. Read the reports

```bash
cat runs/vault-pass/summary.md
python3 -m json.tool runs/vault-pass/report.json
python3 -m http.server 8765 --bind 127.0.0.1 --directory runs/vault-pass
```

Open [http://127.0.0.1:8765/report.html](http://127.0.0.1:8765/report.html) while the server runs. Stop it with Ctrl+C. Python's HTTP server is a local report viewer, not the API worker.

A PASS covers the declared test world and checks. Continue with [your first migration test](/docs/first-test) to see a real balance regression and reproduce it offline.
