# Project manifest

`doppelganger.toml` selects the program artifacts, V1 state construction, migration transaction and observed account rules. Schema version 1 is the current contract.

## Program and project identity

```toml
schema_version = 1

[project]
name = "vault-runtime"
framework = "anchor"

[versions.v1]
program_path = "v1.bin"
idl = "v1.json"
program_id = "DTbPK89xmJH65ay3YyJTZnPnDWsyXU3KNLiEvXQ7bjoT"

[versions.v2]
program_path = "v2.bin"
idl = "v2.json"
program_id = "DTbPK89xmJH65ay3YyJTZnPnDWsyXU3KNLiEvXQ7bjoT"
```

This is an excerpt from the prepared tutorial copy. Both IDLs must agree with the configured program ID; V1 and V2 must share it. The binary path must exist. Full VM artifact validation occurs during execution.

## Address bindings

The original manifest defines deterministic `owner` and `stranger` signer seeds plus their funding. It also includes:

```toml
[addresses.program]
type = "program"
version = "v1"

[addresses.system_program]
type = "literal"
address = "11111111111111111111111111111111"

[addresses.vault]
type = "pda"
program = "v1"
seeds = [{ const = "vault" }, { pubkey = "$owner" }]
```

`$owner` resolves a named binding. Instruction `account_mapping` values use binding names such as `owner`, without `$`. The resolver derives the actual PDA and rejects unknown/circular bindings. Public deterministic fixture seeds are test inputs, never production signing credentials.

## Observe and migrate

```toml
[[migration_targets]]
name = "vault"
address = "$vault"
v1_type = "VaultV1"
v2_type = "VaultV2"

[migration]
instruction = "migrate"
account_mapping = { vault = "vault", owner = "owner", system_program = "system_program" }
```

The target selects the account whose BEFORE/AFTER state is decoded under the two type names. The migration selects one V2 instruction. IDL account order and encoding determine the submitted local transaction.

## Typed arguments

The actual fixture deposits with `args = { amount = "u64:1000000" }`. Decimal strings allow integer boundaries beyond TOML's signed 64-bit literals. Prefixes must match the IDL type. Public-key arguments may resolve `$bindings`. Unsupported shapes fail validation rather than being inferred.

## Versioning and validation

Pre-versioned manifests remain a distinct frozen legacy format. Add `schema_version = 1` at the top of a newly maintained project, before any table. Do not edit hashed snapshot/regression inputs. Unsupported explicit versions and invalid bindings fail.

See [configuration reference](/docs/configuration) for all top-level sections, [fixtures](/docs/state) for state construction and [preservation](/docs/preservation) for rule syntax.
