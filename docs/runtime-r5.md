# R5: Generic V1 World Builder

R5 builds the configured V1 world and captures state.
It does not yet execute the V2 migration.

## Implementation

The new `dg-runtime` crate has no dependency on either demo program in its normal
dependencies. Program-specific dependencies are confined to tests.

- `RuntimeWorldBuilder::build_project` reads a manifest, its V1 IDL and binary,
  resolves addresses, compiles fixture actions, then builds a fresh world.
- `build_manifest` accepts a parsed manifest and its base directory.
- `build` accepts the existing `ProgramArtifact`, normalized model, fixture,
  `ResolvedAddresses`, and `FixtureInstructions` directly.
- `RuntimeWorld` owns an isolated LiteSVM, resolved signing identities, V1 artifact,
  and a count of funded identities. It is not serialized. The existing
  `dg-state::RuntimeWorld` remains a data-only fixture snapshot collection.
- `WorldBuildResult` returns the live world, attempted action records, and
  observations in fixture declaration order.
- `RuntimeWorld::observe` reads actual VM accounts. Each `ObservedAccount` includes
  its expected binding, a raw snapshot, decoded fields when available, and a
  separate structured decode error. Expected owner/lamports never overwrite the
  observed values. R5 does not decide whether an ownership difference is correct.

The builder funds each signer once, inserts only explicitly synthetic accounts,
executes all initialization before setup, and stops at the first failed action.
Each action is a separate transaction. Its record retains phase, index, name,
program/account metadata, public signing keys, instruction bytes, signature,
success/error, logs, fees, and compute units. No private key is serialized.

`BuildError` carries a category, context, message, attempted action records, and
any observations collected before a capture error. Earlier successful actions are
not described as rolled back when a later action fails. A failed build does not
return a usable world. Programs report their own errors; the builder interprets
no protocol-specific instruction or error name.

## Configuration

Use the existing [Vault fixture](../examples/vault-runtime/doppelganger.toml).
Set `fixture.payer` to a logical signer or alias. When omitted, the builder chooses
the first funded signer in name order, falling back to the first signer if all
are zero-funded. It never invents a hidden fee payer or extra funding.

`fixture.account.fields` means direct synthetic state creation using the existing
encoder. Without fields, the account is only an observation target; R5 does not
pre-create it or apply its declared lamports/owner. Existing runtime accounts
cannot be overwritten by synthetic definitions.

`test_seed` preserves deterministic identities across builds. Without it, fresh
random signer keys are generated. Test keys must never hold production assets.

## Run

Build the existing V1 binary and IDL using the root README, then:

```bash
cargo run -p dg-cli -- runtime build examples/vault-runtime --output report/v1-world.json
```

The JSON output contains the V1 artifact reference, execution records, raw and
decoded observations, and `migration_executed: false`. Without `--output`, the
command prints its summary without writing a file. Failure diagnostics are
printed as structured JSON on stderr.

Verified output (public addresses omitted here because the example uses random keys):

```text
DOPPELGANGER RUNTIME WORLD
Project: examples/vault-runtime
Program: Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS (V1 loaded)
Signers: 2 funded identities
Initialization actions: 1/1 passed
Setup actions: 1/1 passed
Observed accounts: 1
vault
  owner: Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS
  lamports: 2224960
  data length: 48
  snapshot: captured
  decode: VaultV1
No migration executed.
```

## Verification

The dedicated `dg-r5-fixture` program creates records, performs order-sensitive
updates, and can deliberately corrupt account data for observation tests. It has
no migration or preservation behavior. Build it before running runtime tests:

```bash
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-r5-fixture/Cargo.toml --sbf-out-dir target/deploy/r5
cargo test -p dg-state -p dg-config
cargo test -p dg-runtime --test runtime_r5
cargo test -p dg-runtime --test vault_equivalence
cargo test --workspace
cargo clippy -p dg-state -p dg-config -p dg-runtime -p dg-cli -p dg-r5-fixture --all-targets --no-deps -- -D warnings
```

Use `--offline --skip-tools-install` with `build-sbf` when platform tools v1.57
are already installed. `bash scripts/check-r5.sh` runs the gates in that order.
The Vault acceptance test requires the existing V1 binary and IDL; missing
artifacts fail tests rather than silently skipping them.

There are 32 isolated R5 tests and one separate Vault V1 acceptance test. The
acceptance test independently reproduces the existing typed, hardcoded V1 recipe
with deterministic keys, then compares address, owner, lamports, data length,
raw bytes, and decoded fields against the generic configuration-driven build.
It supplies nonexistent V2 files and an invalid migration instruction to verify
that neither is consumed. This is not the final runtime integration suite.

## Limits

- V1 only. The shared manifest syntax still requires V2/migration sections, but
  R5 does not read V2 files or resolve/execute the migration instruction.
- One configured V1 program plus LiteSVM's built-in/default programs. Arbitrary
  additional CPI program configuration is not implemented.
- No migration validation, preservation rules, lifecycle scenarios, or upgrade
  report. Decode errors are evidence, not migration findings.
- No general VM reset/restore API. Build another world for a fresh environment.
- LiteSVM's normal account setter deletes zero-lamport accounts. R5 uses the pinned
  version's `persistence-internal` direct insertion API only for explicitly
  configured, non-executable zero-lamport state, after collision/encoding checks.
  This preserves local boundary fixtures; normal transaction cleanup can still
  remove them. It does not imply such accounts persist on a cluster.
- Decoder/account limits apply; complete hostile-input, process-resource, artifact
  integrity, and multi-account scale qualification remain future work.

The old hardcoded migration demo remains the reference. R5 ends here; R6 is next.
