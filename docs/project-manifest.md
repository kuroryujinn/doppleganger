# Project manifest

The production input is a prepared directory containing `doppelganger.toml` and `doppelganger-policy.toml`. IDLs and SBF binaries are referenced by the manifest. Start from the complete Vault, Escrow or Record example rather than an incomplete template.

```toml
schema_version = 1
[project]
name = "my-project"
framework = "anchor"
[versions.v1]
program_id = "YOUR_VALID_BASE58_PROGRAM_ID"
program_path = "artifacts/v1.so"
idl = "artifacts/v1.json"
[versions.v2]
program_id = "THE_SAME_PROGRAM_ID"
program_path = "artifacts/v2.so"
idl = "artifacts/v2.json"
[migration]
instruction = "migrate"
account_mapping = { position = "position", authority = "authority" }
args = { version = "u8:2" }
[[migration_targets]]
name = "position"
address = "$position"
v1_type = "PositionV1"
v2_type = "PositionV2"
```

The sample program IDs are placeholders; replace them before validation. Paths resolve relative to the manifest directory. Existing files are required and IDL addresses must match the configured program ID. Absolute paths and references outside the directory are accepted for this local tool. A future upload service must constrain them in an isolated workspace.

`addresses` defines named public bindings with `type = "signer"`, `"literal"`, `"program"`, `"reference"` or `"pda"`. Signers require a 32-byte `test_seed` in production tests and configured lamports. Use `$binding` references in values; account_mapping uses binding names. PDA seeds support constant bytes/string, public keys and typed integer seeds. Program selection is `v1` or `v2`. Circular/unknown references fail. TOML duplicate keys, duplicate targets and duplicate observation identities fail.

`fixture` contains `name`, optional `payer`, `account`, `initialize` and `setup`. `fixtures` contains named alternate fixtures for scenarios. Initialization/setup definitions use the same instruction/account_mapping/args shape as migration. An observed account without fields is expected to be created by setup; an account with fields creates explicitly synthetic test state. See [engine reference](engine-reference.md).

`preservation` and `scenario` are arrays. `snapshot` configures read-only acquisition and imported program/account identities. Policies and `relationship` definitions live in the separate profile. Unknown dangerous fields and unsupported versions fail rather than being ignored. Resource budgets are in [supported formats](supported-anchor.md).

Pre-versioned manifests remain a distinct frozen compatibility format and are reported with `legacy_unversioned_manifest: true`; they are not automatically reinterpreted as a future schema. Upgrade using the [migration guide](migration-guide.md).

## Registry

Registration is optional: direct manifest paths work everywhere. `project add DIR` records its canonical manifest under its validated project name. Duplicate names from another path and renamed entries pointing to the same manifest are rejected. `project list` reads the versioned map. A shared OS file lock covers reads and entire update transactions; updates use temporary file, sync and rename with the previous validated state saved as `.doppelganger/projects.json.bak`.

Corrupt/unsupported registries block normal operations. `project recover` explicitly restores a validated backup and preserves the damaged file as `projects.json.corrupt.<pid>`. Recovery restores the previous successful update, so the most recent addition may need registering again. Registry files and its directory must not be symlinks. Registry use is qualified on Linux; it is not a multi-host/distributed registry.
