# Versioned policy

`doppelganger-policy.toml` determines whether observed evidence is acceptable. It does not execute transactions. A policy is required by the production `test` workflow.

## Defaults

```toml
schema_version = 1

[policy]
unexpected_static = "fail"
require_preservation = true
require_relationships = true
require_lifecycle = true
```

These are the actual default settings; the Vault example omits this table because the defaults apply. With no declared rules, relationships or lifecycle scenarios, the default policy blocks for missing coverage.

`unexpected_static` supports `fail`, `warn` and `ignore`. Advisory static settings do not downgrade migration rejection or failed runtime checks. Warnings remain in evidence and may coexist with a PASS.

## Relationships

```toml
[[relationship]]
id = "vault-authority"
kind = "authority"
account = "vault"
target = "$owner"
field = "owner"
```

Declare semantic links explicitly. The engine does not discover every reference in every account. See [account relationships](/docs/relationships).

## Exact expected changes

A real Vault approval:

```toml
[[expected_change]]
classification = "ACCOUNT_SCHEMA_CHANGE"
path = "account.VaultV1 -> VaultV2.withdrawal_limit"
before_json = "null"
after_json = '{"kind":"primitive","value":"u64"}'
reason = "Reviewed fixture upgrade: Field or argument added"
```

Matching requires classification, path and parsed BEFORE/AFTER JSON to agree exactly. A stale approval for a change that was not observed blocks. Wildcards and automatic approval generation are not supported.

Review the structural report before copying selectors into a profile. Runtime violations such as a changed balance cannot be approved as structural changes. [Coverage and expected changes](/docs/coverage) explains the decision process.
