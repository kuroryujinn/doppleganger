# Complete test workflow

Use `test` for the complete policy-controlled migration workflow. `migration test` is an exact alias. Both use the same arguments, defaults, outputs and exits.

## Syntax

```text
doppelganger test PROJECT [--policy FILE] [--snapshot FILE_OR_DIRECTORY]
  [--output NEW_DIRECTORY] [--ci] [--json | --quiet] [--verbose]
```

| Argument/option | Behavior and default |
| --- | --- |
| `PROJECT` | Required current manifest file or prepared directory; no registry-name lookup |
| `--policy FILE` | Explicit profile; default is sibling `doppelganger-policy.toml` |
| `--snapshot FILE_OR_DIRECTORY` | Restore sealed V1 state instead of default fixture construction |
| `--output NEW_DIRECTORY` | Default `.doppelganger/runs/<run-id>-<process-id>/`; never overwrite |
| `--ci` | Express CI intent; exit contract applies with or without it |
| `--json` | Print complete report JSON to stdout |
| `--quiet` | Suppress normal summary; mutually exclusive with `--json` |
| `--verbose` | Print additional stage/input/output diagnostics to stderr |

Duplicate/unknown options or missing paths fail with exit 2. `test --help` is not a dedicated help path in this implementation; use top-level `--help` or this page.

## Execute

```bash
doppelganger test runs/vault-project --ci --output runs/vault-pass
```

The engine validates and hashes inputs, compares IDLs, builds/restores V1, loads V2 at the same program ID, executes the configured migration, evaluates preservation and declared relationships, runs fresh-world lifecycle scenarios and applies policy.

## Output

A completed test writes `report.json`, `report.html` and `summary.md`. Normal terminal output contains the verdict, exit code and JSON path. Configuration/runtime failures retain structured diagnostics where output is possible. Argument parsing and output-write failures print diagnostics even when a report directory cannot be written.

To inspect a report:

```bash
python3 -m json.tool runs/vault-pass/report.json
cat runs/vault-pass/summary.md
```

## Exit contract

See [exit codes](/docs/exit-codes) for 0–4. Exit 1 is an expected outcome when the broken Vault loses balance; it does not mean the CLI crashed. Exit 4 for an already existing output directory preserves previous reports. Choose a fresh path on every run.

## CI

After toolchain/cache provisioning, `scripts/check-production-mvp.sh` runs the repository's qualified good/broken workflow checks. The existing GitHub workflow expects a self-hosted Linux runner; this documentation does not claim a hosted Actions run.

For your own CI, retain report artifacts even on exit 1 and use `test`, rather than standalone `runtime verify`, as the gate. Do not approve unexpected findings automatically.
