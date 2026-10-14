# Snapshot commands

Acquire explicitly configured Devnet accounts, inspect immutable snapshots or replay their V1 state locally. Only capture needs RPC.

## Capture

```text
doppelganger snapshot capture PROJECT [--output DIRECTORY]
```

The project must contain valid `[snapshot]` configuration, matching local program artifacts/IDLs and explicit controlled addresses. Default output is `.doppelganger/snapshots/<snapshot-id>/snapshot.json`. An identical existing immutable capture is reused; changed content is not silently overwritten.

```bash
export DG_DEVNET_RPC_URL=https://api.devnet.solana.com
doppelganger snapshot capture examples/vault-runtime-devnet --output runs/devnet-capture
```

This is an opt-in live command requiring the expected V1 accounts to exist on Devnet at the configured addresses. Historical fixtures are not a promise that those accounts still exist. Capture syntax was inspected; live recapture was not run for this documentation. See [Devnet capture](/docs/snapshots).

## Inspect

```text
doppelganger snapshot inspect SNAPSHOT_FILE_OR_DIRECTORY_OR_ID [--store DIRECTORY]
```

A non-path ID resolves under `.doppelganger/snapshots` or `--store`. The command validates and prints the snapshot JSON; it does not execute migration.

```bash
doppelganger snapshot inspect fixtures/devnet-snapshots/vault-devnet
```

## Replay

```text
doppelganger snapshot replay SNAPSHOT_FILE_OR_DIRECTORY_OR_ID --project PROJECT
  [--candidate BINARY] [--candidate-idl IDL] [--lifecycle]
  [--output REPORT_JSON] [--store DIRECTORY]
```

`--project` is required. Optional candidate paths override V2 inputs. `--lifecycle` evaluates lifecycle in addition to migration/preservation. JSON is printed; `--output` additionally saves it. A replay output may not overwrite a valid immutable snapshot, but ordinary existing report files can be replaced.

```bash
doppelganger snapshot replay fixtures/devnet-snapshots/vault-devnet \
  --project examples/vault-runtime-devnet --lifecycle --output runs/frozen-evidence.json
```

This was verified with `DG_DEVNET_RPC_URL=http://127.0.0.1:1` to confirm no usable RPC endpoint was needed. It is evidence-only; a failed preservation rule does not necessarily produce a failed process exit. Use production `test --snapshot` to apply policy.

## Exits and errors

Valid capture/inspect/replay completion exits 0. Configuration, integrity, decoding or transport failures print structured snapshot diagnostics and propagate a command error, commonly exit 1. Do not apply the production exit table to these lower-level commands. Duplicate/unknown flags and missing values fail; there is no capture `--store` option.
