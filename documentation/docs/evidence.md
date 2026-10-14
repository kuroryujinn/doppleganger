# Understanding migration evidence

Read the verdict together with the transaction and check evidence. Process exit 0 from the complete `test` workflow means policy accepted the supplied run, not that every possible state is safe.

## Evidence layers

| Report section | Question |
| --- | --- |
| `analysis.findings` | What changed structurally between the IDLs? |
| `execution.transaction` | Did the configured local migration transaction succeed? |
| `execution.before_accounts` / `after_accounts` | What raw and decoded state was observed? |
| `preservation.evaluations` | Did each declared field/metadata rule pass? |
| `preservation.findings` | Which preservation checks failed or errored? |
| `relationships.edges` | Did declared authority/reference/PDA edges hold? |
| `lifecycle.scenarios` | Did fresh-world operational expectations hold? |
| `policy.decisions` | Which findings were expected, warnings, ignored or blocking? |

The broken Vault succeeds as a transaction but changes its balance from 1,000,000 to 950,000. The `equal` preservation check fails and policy returns FAIL. Static compatibility alone cannot detect that runtime loss.

## Three report formats

`test` writes `report.json`, `report.html` and `summary.md` in a new output directory. JSON holds full machine-readable evidence, HTML provides a policy table and expandable evidence, and Markdown summarizes the result.

```bash
cat runs/vault-pass/summary.md
python3 -m json.tool runs/vault-pass/report.json
```

Serve the output directory with Python's local HTTP server to view HTML, as shown in the [quickstart](/docs/quickstart).

## Check statuses and verdicts

Checks use PASS, FAIL, ERROR and NOT_APPLICABLE. Missing required evidence does not become a pass. ERROR indicates invalid configuration, unavailable state or inability to evaluate/execute; its process exit distinguishes the category. FAIL means the observed behavior violated policy. See [exit codes](/docs/exit-codes).

## Integrity and numeric values

Artifacts are hashed before the run and rechecked afterwards. `artifacts` records canonical paths and SHA-256 identities. Run identity derives from input content and workflow version. Typed integers serialize as decimal strings to avoid JavaScript precision loss. Raw bytes remain available when typed decoding fails.

Preservation/relationship guarantees cover declared rules and observed accounts only. Evidence does not certify undeclared economics, conservation, authorization paths or every possible account combination.
