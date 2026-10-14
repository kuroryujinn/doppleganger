# Expected changes and coverage

Policy separates intentional structural changes from runtime violations and requires evidence in declared areas.

## Review an exact finding

```bash
doppelganger analyze --from runs/vault-project/v1.json \
  --to runs/vault-project/v2.json --account VaultV1:VaultV2 --format json
```

Inspect `classification`, `path`, `before` and `after` on each finding. An approval for the new Vault field is:

```toml
[[expected_change]]
classification = "ACCOUNT_SCHEMA_CHANGE"
path = "account.VaultV1 -> VaultV2.withdrawal_limit"
before_json = "null"
after_json = '{"kind":"primitive","value":"u64"}'
reason = "Reviewed fixture upgrade: Field or argument added"
```

This is a real selector from the bundled policy. Match the values exactly after review. A changed value under the same path is not approved; an unused/stale approval blocks too.

## Coverage defaults

The policy requires nonempty preservation, relationship and lifecycle coverage. A deliberately narrower project can explicitly set `require_preservation`, `require_relationships` or `require_lifecycle` to false in `[policy]`. Configured failing checks still block regardless of coverage settings.

Setting `unexpected_static = "warn"` makes unexpected structural findings advisory. It cannot hide a rejected migration or a failed preservation/relationship/lifecycle check.

## Diagnose policy output

Read `policy.coverage`, `policy.expected`, `warnings`, `blocking`, `evaluation_errors` and `decisions`. The reviewed Vault policy accepts its known layout/discriminator/interface/error changes. It still rejects the broken candidate's balance loss.

Do not generate automatic blanket approvals to obtain PASS. Record reasons with selectors that explain the actual reviewed change, then rerun in a new output directory.
