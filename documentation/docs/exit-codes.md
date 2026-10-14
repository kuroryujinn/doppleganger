# CLI exit codes

The following 0–4 contract belongs to production `test` and its `migration test` alias. Lower-level and legacy commands have different semantics.

## Production tests

| Exit | Verdict/category | Meaning |
| --- | --- | --- |
| 0 | PASS | Required checks accepted under supplied policy; warnings can remain |
| 1 | FAIL | Policy violation, transaction rejection, failed check or missing coverage |
| 2 | ERROR / configuration | Invalid arguments, manifest/policy, artifacts or snapshot |
| 3 | ERROR / runtime or evaluation | State setup/execution or check evaluation failed |
| 4 | ERROR / internal/output | Serialization, mutated inputs or output write failed |

Verified good output:

```text
DOPPELGANGER MIGRATION TEST
Result: PASS
Exit code: 0
```

Verified broken output:

```text
DOPPELGANGER MIGRATION TEST
Result: FAIL
Exit code: 1
```

To produce a configuration ERROR intentionally, use a nonexistent input and a new output directory:

```bash
doppelganger test runs/missing-project --json --output runs/config-error
printf 'Exit: %s\n' "$?"
```

The JSON records `result: "ERROR"`, `exit_code: 2` and a `CONFIG_ERROR` diagnostic. It contains no successful migration evidence. Do not capture this incomplete argument/artifact failure as a portable migration regression.

## Reproduction is a different question

Portable `regression replay` exits 0 when the known failure reproduces, 1 for mismatch and 2 for an invalid case. Read `matched` and `migration_exit_code` separately.

Native `regression reproduce`, snapshot replay and runtime evidence commands can exit 0 while reporting failed checks or `NOT_REPRODUCED`. They are not production policy gates. `doctor` returns 2 for explicit failed checks; missing optional build tools produce warnings rather than failure.
