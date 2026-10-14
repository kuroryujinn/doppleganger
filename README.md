# Doppelganger

The independently hosted [developer documentation website](documentation/README.md)
includes a verified local CLI quickstart, a good/broken migration walkthrough,
command references and offline regression reproduction. Its source lives in
`documentation/`; build and static-host settings are documented there.

R11 adds [read-only Devnet snapshots and offline local replay](docs/runtime-r11.md).
Genuine Devnet application acceptance passed on 2026-10-05.

## Production CLI workflow

The [policy-controlled workflow](docs/production-workflow.md) now combines static
analysis, local migration, preservation, declared account relationships, lifecycle
testing, and upgrade policy in one command:

```bash
cargo run -p dg-cli --offline -- migration test examples/vault-runtime --ci
cargo run -p dg-cli --offline -- test examples/escrow-runtime --json
```

Prepared projects supply the existing runtime manifest plus a versioned
`doppelganger-policy.toml`. Runs write complete JSON, HTML and Markdown evidence.
Exit codes distinguish success (0), policy violations (1), configuration errors (2),
runtime/evaluation errors (3), and internal/output errors (4). Default policy
blocks missing coverage and unexpected changes. Older runtime commands retain
their evidence-only exit behavior.

Build acceptance prerequisites with `bash scripts/build-production-fixtures.sh`,
then run `bash scripts/check-production-mvp.sh`. An opt-in GitHub workflow is
provided for a provisioned runner. See the workflow guide for toolchain requirements,
profile examples and frozen snapshots. Start with the [getting-started guide](docs/getting-started.md) and MVP acceptance for the qualified 0.1.0 release.

The backend implementation plan defines the next phase:
isolated migration jobs, prepared-project uploads, supported source-folder builds,
and the API contract for frontend integration. The backend foundation,
[durable worker](docs/backend-worker.md) and [prepared-project API](docs/backend-api.md)
are implemented and locally tested. Live rootless-container qualification and source-folder compilation remain open.

Solana state migration testing prototype for the Colosseum hackathon. The current demo runs two Anchor program versions at the same program ID in LiteSVM, migrates a SOL-backed vault account, and writes compatibility reports.

## Static preflight (Phase 0)

The independent `dg-analysis` crate normalizes two Anchor IDLs, compares instructions, account layouts, enums, PDA definitions, discriminators, events, custom types, and errors, then emits migration hints and a suggested runtime test matrix. It does not execute a migration or depend on LiteSVM.

```bash
cargo run -p dg-cli -- analyze --from target/idl/v1.json --to target/idl/v2.json --account VaultV1:VaultV2
cargo run -p dg-cli -- project analyze vault-demo --format html
```

`--from` and `--to` accept IDL JSON files or directories containing `idl.json` or a single JSON file in `target/idl`. Repeat `--account OLD:NEW` for renamed account types; identical names pair automatically. `project analyze` uses the registered manifest's account mappings. `--format json` prints the machine-readable report, while `--format html` also writes a browser report. Every run writes `.doppelganger/analysis/<stable-run-id>/analysis.json`. Findings describe structural differences and recommended tests, not a safety verdict. Exact account sizes are shown only when statically derivable; dynamic layouts carry a minimum size. Run isolated tests with `cargo test -p dg-analysis`.

## Demo

Requirements: Rust, Solana `cargo-build-sbf` with platform tools v1.57, Anchor CLI 1.2, and the dependencies in `Cargo.lock`.

```bash
mkdir -p target/idl
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-vault-v1/Cargo.toml --sbf-out-dir target/deploy/v1
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-vault-v2/Cargo.toml --sbf-out-dir target/deploy/v2
NO_DNA=1 anchor idl build -p dg_vault_v1 -o target/idl/v1.json
NO_DNA=1 anchor idl build -p dg_vault_v2 -o target/idl/v2.json
cargo run -p dg-cli -- demo target/deploy/v1/dg_vault_v1.so target/deploy/v2/dg_vault_v2.so target/idl/v1.json target/idl/v2.json report/good.json
```

On the current development machine, invoke `/home/kuro/.avm/bin/anchor-1.2.0` in place of `anchor`: the `anchor` wrapper attempts to activate an unavailable Solana CLI version. The platform tools are already installed here, so `--offline --skip-tools-install` can be added to each `cargo build-sbf` command.

The runner prints each check and writes `report/good.json` and `report/good.html`. It exits nonzero when any check fails. Open the HTML in a browser. Codex's file panel shows its source; for a browser preview, run `python3 -m http.server 8765 --bind 127.0.0.1 --directory report` and visit `http://127.0.0.1:8765/good.html` or `/bad.html`.

## Failure demonstration

The `skip-backing-top-up` feature compiles a deliberately flawed V2 migration. It reallocates the account without adding SOL for the larger rent reserve. The recorded vault balance survives, but its SOL backing falls short by 55,680 lamports.

```bash
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-vault-v2/Cargo.toml --features skip-backing-top-up --sbf-out-dir target/deploy/bad
cargo run -p dg-cli -- demo target/deploy/v1/dg_vault_v1.so target/deploy/bad/dg_vault_v2.so target/idl/v1.json target/idl/v2.json report/bad.json
```

The second command should exit with status 1 and report failures for `sol_backing_preserved` and `post_migration_sol_backing`.

After building both binaries and IDLs, run `bash scripts/check-mvp.sh` to check both outcomes and their JSON reports together. The script uses Python 3 from the standard library to verify the expected failure names.

## Add a Project

Create a TOML manifest like [the vault example](examples/vault-project.toml), then register and check it from this workspace:

```bash
cargo run -p dg-cli -- project add examples/vault-project.toml
cargo run -p dg-cli -- project list
cargo run -p dg-cli -- project check vault-demo
```

Registration is stored in `.doppelganger/projects.json` in the current working directory. Paths in a manifest are resolved relative to that manifest. A check writes `report/<name>.json` and `report/<name>.html`.

For another Anchor program, set `name`, `program_id`, V1/V2 IDL paths, and one or more `[[accounts]]` mappings. Optionally add `[accounts.pda]` to compare a named PDA account across the listed instructions. The [schema-only example](examples/vault-schema-only.toml) omits a runtime scenario; its report explicitly says migration was not executed. `project check` exits successfully when analysis completes, even if runtime coverage is absent. This legacy `project check` interface retains its Vault demo restriction. The generic production `test` workflow executes Vault, Escrow and configured supported projects.

## Generic Migration Configuration

[The Phase 1 example](examples/vault-migration/doppelganger.toml) uses `doppelganger.toml` to name both program binaries and IDLs, the V2 migration instruction, account source bindings, and typed instruction arguments. Paths are relative to the manifest. Validation checks the files, program IDs, instruction, account names, argument names, and argument values before execution.

```bash
cargo run -p dg-cli -- project add examples/vault-migration
cargo run -p dg-cli -- migration inspect vault-migration
cargo run -p dg-cli -- migration check vault-migration
```

`migration inspect` works for any supported Anchor IDL and prints the resolved account order, signer/writable flags, discriminator, and argument sizes without running LiteSVM. `migration check` currently executes only the bundled vault state fixture; other programs receive a clear unsupported-fixture error after their binding validates. The migration transaction itself is built from IDL and manifest data by the standalone `dg-config` crate, with no vault-specific branch. This restriction belongs to the legacy demo command; generic fixture setup and preservation are available through production `test`. Supported argument values include booleans, integers through u128/i128, strings, byte arrays, public keys, arrays, vectors, options, tuples, and defined struct/enum values. Use decimal strings such as `"u64:18446744073709551615"` or `"i128:-170141183460469231731687303715884105728"` outside TOML's signed 64-bit literal range. Typed prefixes must match the IDL. Unsupported shapes fail during configuration validation.

## Generic State Foundation (Phase 2)

The standalone `dg-state` crate implements generic snapshots, declarative fixture materialization, and IDL-driven Borsh decoding. This is a library milestone, not a generic migration CLI yet. See [the state interface and acceptance gate](crates/dg-state/README.md).

```bash
cargo test -p dg-state
```

Snapshots retain raw bytes, decoded fields, addresses, ownership, lossless lamports, data length, and metadata. Decoded integers use arbitrary-precision `BigInt` and serialize as decimal strings. Tests cover numeric boundaries, nested/dynamic layouts, malformed bytes, resource limits, snapshot round-trips, and an independent Borsh wire-format check. The existing runner rejects configured generic accounts instead of silently substituting its bundled fixture.

## What is tested

The runtime foundation work package adds raw snapshot
restoration, generic signer/PDA resolution, and initialization/setup definitions.
[R5's generic LiteSVM builder](docs/runtime-r5.md) now executes configured V1 setup
and captures actual runtime state:

```bash
cargo run -p dg-cli -- runtime build examples/vault-runtime --output report/v1-world.json
```

This command loads V1 only. It does not execute migration or evaluate preservation.
`migration inspect` remains a non-executing configuration check, and the existing
hardcoded migration demo remains available unchanged.

[R6's generic migration executor](docs/runtime-r6.md) now loads the configured V2
candidate into the existing world and submits one configured migration transaction:

```bash
cargo run -p dg-cli -- runtime migrate examples/vault-runtime --output report/migration-execution.json
```

Its report contains version-specific before/after snapshots, absent accounts,
decode errors, transaction errors, logs, compute units, fees, and public signer
keys. `preservation_evaluated` is explicitly false. The checks below still belong
to the separate hardcoded Vault demo:

- V1 initializes and funds a vault PDA with SOL.
- V2 rejects direct operations on unmigrated V1 bytes.
- An unrelated signer cannot migrate the vault.
- Migration preserves owner, recorded balance, account address, and SOL backing after rent growth.
- V2 reads the migrated bytes and executes a withdrawal.
- A repeated migration and an over-limit withdrawal fail without changing vault state.
- Anchor IDLs provide the account field diff, discriminator change, and PDA seed comparison.

V1 setup, single V1-to-V2 execution, and [R7 developer-declared preservation rules](docs/runtime-r7.md) now run through generic configuration:

```bash
cargo run -p dg-cli -- runtime verify examples/vault-runtime --output report/preservation.json
```

R7 evaluates twelve field/metadata rule primitives, not deployment safety. The sample's seven rules pass for the corrected Vault. They also pass for the existing faulty Vault because its SOL-backing defect requires economics outside these primitives; exact lamport preservation is not a substitute. The original demo retains its backing and lifecycle checks.

R7 acceptance is closed with a separate [broken-balance candidate](examples/vault-runtime-r7-broken/README.md): the same seven rules yield six passes and one genuine balance-preservation failure while migration still succeeds. Its README includes the required build command for the additional test artifact. The original SOL-backing-defect fixture is unchanged.

[R8's generic lifecycle engine](docs/runtime-r8.md) now runs eight configured scenario types using fresh R5 worlds and shared R6 migration/instruction execution. Seven Vault lifecycle cases match the retained typed demo reference. The same suite passes the good candidate and detects exactly one repeat-migration failure in the separate [R8-broken candidate](examples/vault-runtime-r8-broken/README.md).

```bash
cargo run -p dg-cli --offline -- runtime lifecycle examples/vault-runtime --output report/lifecycle-good.json
```

Lifecycle results remain separate from preservation and deployment policy.

## Second Application: DG Escrow (R9)

[DG Escrow](examples/escrow-runtime/README.md) now exercises the same unmodified
runtime stack with two-party initialization, an enum state machine, a new signed
deadline/arbitrator field, and beneficiary-signed release. Build its artifacts with
`bash scripts/build-escrow.sh` (see the fixture README for the local Anchor override),
then run the existing `runtime build`, `migrate`, `verify`, and `lifecycle` commands
against `examples/escrow-runtime`.

Good Escrow passes 11/11 preservation rules and 6/6 lifecycle scenarios. The same
configuration against `examples/escrow-runtime-broken` migrates successfully, passes
10/11 rules, and reports the sole amount regression from 1000000 to 900000. Its
lifecycle remains 6/6, demonstrating the separation between operational success
and preservation. [R9's evidence and core baseline audit](docs/runtime-r9.md) record
the proof; no production core source changes or project-specific adapters were needed.

Native preservation/lifecycle regression replay, genuine Devnet snapshot verification,
and the initial policy-controlled CLI workflow are implemented. SPL Token/economic
invariant packs, partial migrations, portable regression export and broader production
hardening remain. The bundled applications are acceptance fixtures, not deployment-ready
protocols. Testing and frozen-state replay execute locally.
