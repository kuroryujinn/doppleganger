# Command overview

The installed executable is `doppelganger`; the Cargo package is `dg-cli`. Use the complete `test` workflow for policy-controlled results and the lower-level commands for evidence or diagnosis.

## Command families

| Command | Purpose | Policy-controlled exit? |
| --- | --- | --- |
| `--version`, `--help`, `-h` | Version and top-level usage | No evaluation |
| `doctor [PROJECT] [--json]` | Non-executing preflight | 0 on no FAIL checks; 2 on failed checks |
| `analyze` | IDL structural comparison | No; structural findings alone do not fail the command |
| `project` | Registry and legacy project interface | Legacy behavior |
| `migration inspect` | Resolve transaction bindings without execution | No |
| `migration check` | Retained Vault demo fixture | Demo check result |
| `migration test` | Alias of production `test` | Yes |
| `runtime` | Build, migrate, preservation or lifecycle evidence | Evidence only |
| `snapshot` | Capture, inspect or replay sealed V1 state | Evidence only |
| `regression` | Capture/replay or native create/reproduce | Reproduction semantics |
| `test` | Complete migration + policy workflow | Yes, exits 0–4 |

## Help behavior

```bash
doppelganger --help
doppelganger snapshot --help
doppelganger regression --help
```

The latter two print JSON usage. Not every family implements `--help`: for `test`, `runtime`, `project` and `migration`, use this reference and top-level help. Passing an unsupported help flag can itself return an argument error. No implicit command opens an interactive menu.

## Working directory

The registry and default reports live under the current working directory. Manifest-relative artifact paths resolve from the manifest file. The walkthrough assumes the repository root, with prepared inputs under `runs/vault-project`.

Production `test` and runtime/snapshot commands take project paths, not registered project names. `migration inspect/check` can resolve a registered name. See [project commands](/docs/cli-project) for the retained schema distinction.

## Retained demo

```text
doppelganger demo V1_SBF V2_SBF V1_IDL V2_IDL [REPORT_JSON]
```

This hardcoded Vault demonstration defaults to `report/compatibility.json` and also writes HTML. It requires the original bundled Vault identity/artifacts, exits 1 when demo checks fail and is not the recommended generic qualification workflow. Use `test` for your configured project.
