# Lifecycle testing

Migration success alone does not show that V2 operates correctly or rejects invalid transitions. Lifecycle scenarios build fresh worlds and compare declared outcomes.

## Supported scenario types

| Type | Purpose |
| --- | --- |
| `standard_migration` | Expected primary migration result |
| `unauthorized_migration` | Alternate signer must fail as declared |
| `repeat_migration` | Separate first/second migration outcomes |
| `failed_migration_rollback` | Expected failure and rollback evidence |
| `post_migration_instruction` | V2 operation after migration |
| `unmigrated_state_against_v2` | V2 operation on original V1 state |
| `zero_value_state` | Migration under named zero-value fixture |
| `boundary_value_state` | Migration under named boundary fixture |

Each scenario has optional stable `id`, required `type`, and type-specific expected outcomes. `expected_error` matches an exact numeric custom/Anchor error, not a log substring.

## Repeat example

```toml
[[scenario]]
id = "repeat-migration"
type = "repeat_migration"
first = "success"
second = "error"
expected_error = 3002
```

Repeat scenarios require `first`/`second` rather than `expected`. Other types use `expected = "success"` or `"error"`. `expected_second` remains an alias for `second` in the schema.

## Post-migration example

```toml
[[scenario]]
id = "post-migration-limit"
type = "post_migration_instruction"
instruction = "withdraw"
expected = "error"
expected_error = 6002
state_unchanged = true
args = { amount = "u64:500001" }
```

This real Vault case rejects an over-limit withdrawal and checks unchanged state. Instruction scenarios can supply account mappings/arguments; migration overrides use a nested `scenario.migration` definition. Unauthorized scenarios choose `signer_account` and `signer_override` bindings.

## Execute

```bash
doppelganger runtime lifecycle runs/vault-project --output runs/lifecycle-evidence.json
doppelganger test runs/vault-project --ci --output runs/lifecycle-policy
```

The first produces evidence only. The second applies required coverage and policy exits. The good tutorial Vault has seven configured scenarios, all passing. Supported types are eight; a project need not instantiate every type automatically.

Read `lifecycle.scenarios`, their `steps` and diagnostics. A failed expected-error match is a failure even if some program error occurred. The scenarios cover declared operations/state worlds, not every execution path.
