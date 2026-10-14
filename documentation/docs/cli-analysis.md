# Static analysis

Compare two supported Anchor IDLs without building a world or executing a migration.

## Syntax and arguments

```text
doppelganger analyze --from V1_IDL_OR_DIRECTORY --to V2_IDL_OR_DIRECTORY
                    [--account OLD:NEW] [--format text|json|html]
```

| Option | Meaning |
| --- | --- |
| `--from` | Required V1 IDL file or supported IDL directory |
| `--to` | Required V2 IDL file or supported IDL directory |
| `--account` | Pair renamed account types; repeat for multiple mappings |
| `--format` | Default `text`; `json` prints machine-readable evidence; `html` also writes a browser report |

A directory may contain `idl.json` or one JSON file under `target/idl`. Ambiguous/missing files fail. Identically named account types pair automatically; provide an explicit mapping for `VaultV1:VaultV2`.

## Example

After the [quickstart](/docs/quickstart):

```bash
doppelganger analyze --from runs/vault-project/v1.json \
  --to runs/vault-project/v2.json --account VaultV1:VaultV2 --format html
```

Verified header:

```text
DOPPELGANGER STATIC PREFLIGHT ANALYSIS
Project: direct
Upgrade: 0.1.0 -> 0.1.0
Findings: 9
Migration hints: 9
Suggested runtime tests: 9
```

The verified output includes account schema/size changes, discriminator changes, custom errors and instructions. The new withdrawal limit increases the exact account size from 48 to 56 bytes. See the full retained output in the verification record for this build; labels and counts are specific to these IDLs.

## Output and exits

Every successful analysis writes `.doppelganger/analysis/<stable-run-id>/analysis.json`. `--format html` additionally writes `analysis.html`. The command prints the actual paths. JSON format still writes its artifact.

A successful analysis exits 0 even when there are structural differences. Unsupported/invalid input fails through the legacy command error path, commonly exit 1. The production `test` command evaluates these findings under policy and owns the explicit 0–4 exit contract.

## Common mistakes

A static difference is not a migration failure verdict. Static analysis cannot prove preserved balances, correct authorities or runtime rollback. Do not apply the production `test` exit-code table to the standalone analysis command. Registered `project analyze` requires a legacy registry manifest; direct analysis works with the two IDLs from a current prepared project.
