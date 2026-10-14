# Preservation rules

Declare properties that must hold for observed accounts after migration. The manifest owns these rules; policy decides whether their coverage and outcomes are acceptable.

## Rule syntax

```toml
[[preservation]]
id = "balance-field"
account = "vault"
field = "balance"
rule = "equal"
```

`account` is a logical observation name, not `$address`. Stable IDs make failures easy to capture. Nested struct fields use dot-separated paths. Values are typed through the declared IDL; missing/decode errors are retained as errors, not passes.

## Supported rules

| Rule | Field | Value | Condition |
| --- | --- | --- | --- |
| `equal`, `not_equal` | Required | Forbidden | Compare typed BEFORE and AFTER |
| `initialized` | Required | Forbidden | Field absent before, present after |
| `default` | Required | Required | Exact AFTER value |
| `greater_or_equal`, `less_or_equal` | Required | Required | Typed numeric AFTER bound |
| `exists`, `not_exists` | Optional | Forbidden | Account or field presence/absence |
| `owner_preserved` | Forbidden | Forbidden | Account program owner unchanged |
| `lamports_preserved` | Forbidden | Forbidden | Account lamports unchanged |
| `account_size_minimum` | Forbidden | Required | Minimum AFTER data length |
| `pda_preserved` | Forbidden | Forbidden | Observed account address continuity |

`pda_preserved` checks continuity; the policy's `pda` relationship additionally verifies configured derivation. A lamport-preservation rule does not model rent/backing or full token conservation.

## Vault additions

```toml
[[preservation]]
id = "limit-default"
account = "vault"
field = "withdrawal_limit"
rule = "default"
value = "u64:500000"

[[preservation]]
id = "account-size"
account = "vault"
rule = "account_size_minimum"
value = 56
```

These examples are excerpted from the real Vault manifest. To enforce both introduction and exact value of a new field, declare `initialized` and `default` separately.

## Evaluate and inspect

```bash
doppelganger test runs/vault-project --json --output runs/rules-check
```

Read `preservation.evaluations` for all rules and `preservation.findings` for failures/errors. The broken Vault's `PRESERVE-balance-field` contains BEFORE 1,000,000 and AFTER 950,000. `runtime verify` also generates evidence but does not apply final policy exits.

Only declared rules over observed state are tested. Custom expressions, sums and token-conservation packs are not implemented.
