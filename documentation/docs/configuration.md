# Configuration reference

Current production manifests and policies are TOML. Unknown fields, unsupported versions, invalid bindings and resource overruns fail validation. Use complete working examples rather than assembling only the excerpts below.

## Manifest top-level contract

| Section/key | Fields | Role |
| --- | --- | --- |
| `schema_version` | Optional integer; current explicit value `1` | Missing means frozen legacy format |
| `project` | `name`, `framework` | Validated project identity; supported framework `anchor` |
| `versions.v1`, `versions.v2` | `program_path`, `idl`, `program_id` | Required artifacts and matched program IDs |
| `migration` | `instruction`, `account_mapping`, `arguments` (alias `args`) | Required V2 transaction definition |
| `addresses` | Named address definitions | Logical public/signing bindings |
| `fixture` | `name`, optional `payer`, `account`, `initialize`, `setup` | Default V1 world |
| `fixtures` | Named alternate fixture definitions | Lifecycle state variants |
| `migration_targets` | Array: `name`, `address`, optional `v1_type`, `v2_type` | BEFORE/AFTER observations |
| `preservation` | Array: optional `id`, `account`, `rule`, optional `field`, `value` | Typed field/metadata checks |
| `scenario` | Array of lifecycle definitions | Operational expectations |
| `snapshot` | Devnet acquisition configuration | Optional capture/replay inputs |

The migration definition is also used by fixture initialization/setup and scenario migration overrides. Account mappings match IDL logical names to named address sources. `$binding` references are accepted where public keys/state values are resolved. Paths are relative to the manifest directory.

## Address definitions

| `type` | Required fields | Meaning |
| --- | --- | --- |
| `literal` | `address` | Base58 public key |
| `signer` | `lamports`; `test_seed` required by production tests | Local deterministic identity with 32-byte seed |
| `reference` | `account` | Resolve another named address |
| `program` | `version` (`v1`/`v2`) | Configured program ID |
| `pda` | `program`, `seeds` | Derived address for configured version |

PDA seed entries are exactly one supported shape: `{ const = "vault" }`, `{ pubkey = "$owner" }`, `{ integer = "u64:1" }` or `{ bytes = [1, 2] }`. The resolver checks unknown/circular references and seed constraints. Fixture identities are public test inputs, not an authorization/key-storage system.

## Fixture accounts

Each `fixture.account` has `name`, `type`, `address`, `owner`, `lamports`, optional `executable` (default false), and optional `fields`. Omit fields for accounts created by initialization/setup. Supply all supported typed fields for explicit synthetic state. Duplicate observation identities and mismatched ownership/address fail.

`fixture.initialize` and `fixture.setup` are ordered arrays of instruction definitions. Named alternates have the same structure. [Fixtures and snapshots](/docs/state) distinguishes provenance.

## Lifecycle fields

Each scenario has `id` (optional), `type`, `expected` for non-repeat cases or `first`/`second` for repeats, optional exact `expected_error`, `signer_account`/`signer_override`, `fixture`, `instruction`, `account_mapping`, `arguments`/`args`, nested `migration` and `state_unchanged` (default false). See [lifecycle testing](/docs/lifecycle) for required combinations.

## Snapshot fields

| Field | Behavior |
| --- | --- |
| `source` | Required `devnet` |
| `rpc_url` / `rpc_url_env` | Optional mutually exclusive connection source; default public Devnet |
| `min_context_slot` | Optional acquisition floor |
| `max_account_bytes` | Per-account limit, default 1 MiB |
| `accounts` | Required configured entries: `name`, `address`, optional `owner`, `account_type`, `optional` (default false) |
| `programs` | Optional local entries: `program_id`, `program_path` |

Capture is additionally bounded by engine limits. Connection settings do not appear in evidence. Snapshot-backed local execution requires matching program/IDL/account identities and controlled local signer state.

## Policy contract

Required `schema_version = 1`; optional `[policy]`, `[[relationship]]` and `[[expected_change]]` arrays.

| Policy field | Default |
| --- | --- |
| `unexpected_static` | `fail`; also `warn`/`ignore` |
| `require_preservation` | true |
| `require_relationships` | true |
| `require_lifecycle` | true |

Relationships require unique `id`, `kind`, `account`, `target`; field kinds require `field`, with optional `before_field`. `check_before` and `preserve` default true. Expected changes require `classification`, `path`, `before_json`, `after_json`, `reason`, each nonempty/valid as applicable.

See [preservation](/docs/preservation), [relationships](/docs/relationships) and [expected changes](/docs/coverage) for exact semantics.

## Validate before execution

```bash
doppelganger doctor runs/vault-project --json
doppelganger migration inspect runs/vault-project
doppelganger test runs/vault-project --json --output runs/configuration-check
```

Doctor checks current inputs without executing. Inspect resolves transaction bindings. Only the complete test supplies runtime and policy evidence. The tutorial snippets were checked against the real copied project and executed through these commands.
