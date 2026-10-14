# Regression commands

There are two separate case formats: portable whole-workflow cases and native single-finding/scenario cases. They have different file inventories and replay semantics.

## Portable capture

```text
doppelganger regression capture --from-report REPORT_JSON --store NEW_DIRECTORY
```

`--from-report` and `--store` are required. Capture accepts complete version-1 production failures with artifact identities and original exit 1 or 3. It copies binaries, IDLs, manifest, policy and optional snapshot, revalidates input hashes and reruns the normalized failure before sealing `case.json` and `expected.json`.

```bash
doppelganger regression capture --from-report runs/vault-fail/report.json --store runs/vault-case
```

The directory must be new. No `--output` flag is accepted. Successful capture returns `CAPTURED` and exits 0; invalid capture inputs exit 2.

## Portable replay

```text
doppelganger regression replay CASE_DIRECTORY --output NEW_DIRECTORY
```

Both positional case path and `--output` are required. No `--store`, `--candidate` or `--candidate-idl` is accepted.

```bash
doppelganger regression replay runs/vault-case --output runs/vault-replay
```

| Process exit | Replay meaning |
| --- | --- |
| 0 | `REPRODUCED`: normalized evidence matched the stored failure |
| 1 | `MISMATCH`: evaluation differs from the recorded case |
| 2 | Invalid/tampered case, arguments or output failure |

Replay writes production report formats to a fresh output directory. `migration_exit_code` is the failing migration's exit, separate from reproduction success.

## Native create

```text
doppelganger regression create --from-report FILE --manifest FILE_OR_DIRECTORY
  (--finding ID | --scenario ID) [--store DIRECTORY]
```

Exactly one finding/scenario selector is required. Only preservation/lifecycle cases are supported. The default store is `.doppelganger/regressions`. Output includes the content-derived case ID/directory and retains original artifact references.

```bash
doppelganger regression create --from-report runs/vault-fail/report.json \
  --manifest runs/vault-broken-project --finding PRESERVE-balance-field --store runs/native-cases
```

## Native list, inspect and reproduce

```text
doppelganger regression list [--store DIRECTORY]
doppelganger regression inspect CASE_ID_OR_DIRECTORY [--store DIRECTORY]
doppelganger regression reproduce CASE_ID_OR_DIRECTORY
  [--candidate BINARY] [--candidate-idl IDL] [--store DIRECTORY]
```

```bash
doppelganger regression list --store runs/native-cases
doppelganger regression inspect CASE_DIRECTORY
doppelganger regression reproduce CASE_DIRECTORY
doppelganger regression reproduce CASE_DIRECTORY \
  --candidate runs/vault-project/v2.bin --candidate-idl runs/vault-project/v2.json
```

Use the directory returned by `create` for `CASE_DIRECTORY`. Inspection checks the sealed native case. Reproduction replays its recorded state and finding, optionally with corrected V2 inputs. Existing artifact paths must remain valid.

Native reproduction returns `REPRODUCED` or `NOT_REPRODUCED` in JSON; both normal evaluations can exit 0. Invalid native cases propagate a command error, commonly exit 1. Check JSON rather than treating process success as migration qualification.

[Your first migration test](/docs/first-test) executes both formats end to end.
