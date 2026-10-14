# Report schema

The production JSON report is the complete local workflow's machine-readable evidence. CLI 0.1.0 uses `schema_version: 1` and `workflow_version: 1` for completed evaluated runs.

## Top-level fields

| Field | Meaning |
| --- | --- |
| `schema_version`, `engine_version`, `workflow_version` | Report and implementation identity |
| `manifest_schema_version`, `legacy_unversioned_manifest` | Explicit input version/legacy status |
| `run_id`, `project` | Content-derived run identity and project name |
| `result`, `exit_code` | PASS/FAIL/ERROR and production process status |
| `analysis` | Normalized static findings, hints and suggested runtime tests |
| `execution` | Local transaction, program swap, BEFORE/AFTER observations |
| `preservation` | Evaluations, failed/error findings and counts |
| `relationships` | Declared graph edges and statuses |
| `lifecycle` | Fresh-world scenario evidence and counts |
| `policy` | Coverage, expected changes, warnings, blockers and decisions |
| `artifacts` | Canonical paths and SHA-256 identities |
| `source` | Fixture or imported snapshot provenance |
| Execution/evaluation flags | What ran locally and what was evaluated |
| `remote_transactions_sent`, `rpc_write_calls` | False and zero for this local workflow |

Configuration/early runtime ERROR reports can instead contain a smaller envelope with `schema_version`, `result`, `exit_code` and `diagnostics`. Do not assume `execution` exists on every ERROR. Portable capture requires evaluated artifact identities.

## BEFORE and AFTER

`execution.before_accounts` and `execution.after_accounts` contain logical names/bindings, optional raw snapshots, optional decoded state and optional decode errors. Raw evidence includes address, owner, lamports, data length and data bytes. Version-specific IDLs decode the two states. A missing account or decode failure remains explicit.

`execution.transaction` retains instruction, success/error, logs and compute units. `program_swap` records loading V2 into the existing local world.

## Preservation example

The real broken Vault finding includes:

```json
{
  "id": "PRESERVE-balance-field",
  "account": "vault",
  "field": "balance",
  "rule": "equal",
  "before": {"state": "found", "value": {"kind": "integer", "value": "1000000"}},
  "after": {"state": "found", "value": {"kind": "integer", "value": "950000"}},
  "category": "FIELD_NOT_PRESERVED",
  "status": "FAIL"
}
```

This is a selected excerpt. Production findings flatten the evaluation fields; there is no nested `evaluation` property in this report shape. `preservation.evaluations` includes passing checks too; `findings` is not the complete rule list.

## Inspect without extra dependencies

```bash
python3 - <<'PYCODE'
import json
from pathlib import Path
r = json.loads(Path('runs/vault-fail/report.json').read_text())
print('Verdict:', r['result'], 'exit:', r['exit_code'])
print('Blocking:', r['policy']['blocking'])
for f in r['preservation']['findings']:
    print(f['id'], f['status'], f['before'], f['after'])
PYCODE
```

Use JSON consumers that preserve decimal-string integers. Report hashes identify tested inputs; do not change artifacts and then attribute the old result to the new binary.

## HTML and Markdown

`report.html` displays the decision table and expandable full evidence. `summary.md` summarizes the verdict. Both are derived from the same JSON run. The hosted API's derived owner-facing report is a separate boundary that removes private paths; see [prepared-project API](/docs/backend).
