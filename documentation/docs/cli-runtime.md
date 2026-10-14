# Runtime commands

Run an individual local engine stage to inspect evidence. These commands do not evaluate the final policy and retain evidence-only exit behavior.

## Syntax

```text
doppelganger runtime build|migrate|verify|lifecycle MANIFEST_OR_DIRECTORY
                     [--output REPORT_JSON]
```

The project path is required. `--output` is the only supported option. Without it, human-readable output is printed and no report file is written. The output file's parent directories are created; unlike production `test` directories, existing report files may be overwritten.

## Stages

| Action | Behavior | JSON artifact |
| --- | --- | --- |
| `build` | Load V1, fund signers, materialize/initialize/setup state, observe accounts | V1 actions and snapshots; `migration_executed: false` |
| `migrate` | Build V1, load V2 and execute one configured transaction | Transaction and BEFORE/AFTER accounts |
| `verify` | Migrate then evaluate preservation rules | Execution plus preservation; policy/lifecycle false |
| `lifecycle` | Run configured scenarios in fresh worlds | Scenario outcomes, diagnostics and counts |

## Copyable examples

```bash
doppelganger runtime build runs/vault-project --output runs/v1-world.json
doppelganger runtime migrate runs/vault-project --output runs/migration.json
doppelganger runtime verify runs/vault-project --output runs/preservation.json
doppelganger runtime lifecycle runs/vault-project --output runs/lifecycle.json
```

These were executed against the prepared tutorial example. `build` observes a 48-byte VaultV1; `migrate` observes a 56-byte VaultV2. `verify` reports seven passing rules; `lifecycle` reports seven passing scenarios.

## Exit behavior and mistakes

Completed evidence generation can exit 0 even when the transaction or an evaluated check failed. Configuration/setup/filesystem errors fail through the retained error path; there is no production 0–4 verdict contract here. Use `test --ci` for CI gating.

A runtime world report is not a sealed Devnet snapshot. To restore a frozen snapshot, use production `test --snapshot` or [snapshot replay](/docs/cli-snapshot); these runtime commands do not accept a `--snapshot` option.
