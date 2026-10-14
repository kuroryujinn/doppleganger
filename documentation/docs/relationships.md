# Account relationships

Declare the semantic edges that must exist between the observed accounts and public bindings. Relationships live in the versioned policy, not in the manifest's preservation array.

## Vault example

```toml
[[relationship]]
id = "vault-authority"
kind = "authority"
account = "vault"
field = "owner"
target = "$owner"

[[relationship]]
id = "vault-program"
kind = "program_owner"
account = "vault"
target = "$program"

[[relationship]]
id = "vault-pda"
kind = "pda"
account = "vault"
target = "$vault"
```

The first checks a decoded public-key field. `program_owner` checks metadata rather than a field named owner. `pda` requires a manifest PDA binding and checks the resolver's derivation.

## Kinds and evidence requirements

| Kind | Field required? | Target must be observed? |
| --- | --- | --- |
| `authority`, `owner` | Yes | No; literal public key or resolved binding allowed |
| `reference`, `parent`, `mint` | Yes | Yes, existing observed account in both checked states |
| `program_owner` | No | No; account metadata comparison |
| `pda` | No | Configured PDA binding required |

Use `before_field` for a renamed field. References/parents/mints need matching observation entries, not just an address binding. Optional `v1_type`/`v2_type` bindings support raw-only observations where appropriate.

## Redirected or newly added edges

Defaults are `check_before = true` and `preserve = true`. Explicitly declaring a new edge requires `check_before = false` and `preserve = false`; the rule then checks its AFTER target. This does not approve a static field/interface change.

Unknown IDs, duplicate edge IDs, invalid field paths and inconsistent flags fail validation. Missing/ambiguous/undecodable source evidence produces an evaluation error; absent/dangling/changed links produce violations.

## Inspect a run

```bash
doppelganger test runs/vault-project --ci --output runs/relationship-check
```

Read `relationships.edges` and `policy.decisions`. The tutorial declares three edges and all pass for the good candidate. Graph evaluation is pure and local; it does not discover undeclared account links or fetch accounts remotely.
