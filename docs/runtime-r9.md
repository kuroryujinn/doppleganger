# R9: Second Executable Project Generalization

DG Escrow runs through the same Phase 0 and R5/R6/R7/R8 implementation as Vault.
This milestone adds application programs, artifacts, manifests, tests and evidence,
not a new runtime adapter or project registration.

## Application Difference

V1 initialization requires consent from two different signers and fixes a positive
escrow amount. The initializer then funds that amount once; the enum changes from
`Created` to `Funded`. The PDA uses a distinct namespace plus both actors' keys.
V2 migration preserves the actors, amount and status, inserts an `i64` deadline
and arbitrator public key, and resizes the existing account from 81 to 121 bytes.
The `Disputed` enum tag is appended without renumbering existing tags. A real V2
release requires the beneficiary's signature, a funded state, and an unexpired
deadline, then transfers the recorded amount and changes status to `Released`.

This differs from Vault in its two-party initialization, fixed-amount contract,
explicit state transitions, enum field, signed deadline, additional public key,
and different authority/account set for the post-migration operation. It is still
a deliberately small local acceptance application, not a complete escrow product.
Refund/arbitration operations, expiry recovery and new V2 initialization are not
implemented. No zero fixture is claimed: this application rejects zero amounts.

The manifest fixes local test identities, amount `1000000`, deadline `4102444800`,
and arbitrator `3qbR1eZRqXUWroWKKYhbDmR3FfqTHfqSU8zZSxtANzYh`. No production
credentials, cluster connection, RPC import or deployment is involved.

## Core Baseline Audit

Before adding Escrow, SHA-256 hashes of all 21 source files under the five core
crates' `src` directories were recorded in [r9-core-baseline.sha256](r9-core-baseline.sha256).
This includes the decoder, analyzer, config/IDL encoder, address resolver, world
builder, migration executor, preservation engine, lifecycle engine, and CLI source.
The `core_sources_match_pre_r9_baseline_without_project_registration` test checks
every hash and scans for Escrow-specific names. A separate source inventory check
also confirms no additional production source files have been introduced.

Generic runtime/source modifications required: **NONE**. No generic bug fix,
new capability, or project-specific workaround was needed. In particular there
is no Escrow-specific decoder, instruction encoder, PDA resolver or dispatch.

Non-behavioral build metadata changes are limited to two workspace members,
their local program-ID entries in `Anchor.toml`, local package lockfile entries,
and two `dg-runtime` **dev-dependencies** for independent typed acceptance tests.
Those dependencies are not linked into the runtime library's production dependency
graph. All production core source hashes remain at the pre-R9 baseline.

Escrow support was added entirely through program artifacts, configuration,
fixtures, and tests. Documentation and build bookkeeping accompany those additions.

## Actual Static Analysis

The unmodified analyzer processes Anchor-generated IDLs using the account mapping
`EscrowV1:EscrowV2`. The recorded CLI report is
[`report/escrow-analysis.json`](../report/escrow-analysis.json), also stored by the
normal analyzer under `.doppelganger/analysis/64efd312132c4b0a/analysis.json`.

It reports 16 findings: two added account fields, one changed discriminator,
seven error definition/message changes, four instruction additions/removals,
one appended enum variant, and one exact size change from 81 to 121 bytes.
V2 intentionally supports migration and release, not V1's initialize/fund API;
the instruction-removal findings are genuine and are not suppressed.

No standalone PDA-change finding is claimed: the old and new instruction sets
do not overlap, and the analyzer does not infer cross-instruction equivalence.
The configured two-party PDA is independently derived in the runtime tests;
actual before/after address equality is tested separately. No topology proof or
economic safety conclusion is inferred from this report.

## Runtime Evidence

R5 executes real `initialize_escrow` and `fund` transactions. It captures an
81-byte account with `Funded` status, amount 1000000 and 2454640 lamports, owned
by the fixture program. Tests compare its bytes to independently constructed
Anchor serialization and compare all decoded fields to known fixture values.
Initialization uses both parties' signatures; funding uses the initializer.

R6 loads V2 without altering the existing account, resolves `migrate_escrow` and
typed arguments from the generated IDL, and executes with the configured signer.
Tests compare data and metas to Anchor-generated instruction encoding. Both
candidates succeed, grow to 121 bytes, decode as V2, and retain account identity.
The retained V1 snapshots stay unchanged after migration.

R7 uses 11 existing primitives: initializer, beneficiary, amount and status equality;
deadline initialization/default; arbitrator initialization/default; program owner,
address continuity, and minimum size. No new preservation semantics are added.

| Candidate | R6 migration | R7 preservation | R8 lifecycle |
|---|---|---|---|
| Good V2 | Success | 11/11 PASS, zero errors | 6/6 PASS, zero errors |
| Broken V2 | Success | 10/11 PASS, one targeted failure, zero errors | 6/6 PASS, zero errors |

The acceptance-only `r9-broken-amount` feature subtracts 100000 from the recorded
amount after ordinary rent funding. Both candidate manifests differ only in name
and binary path. The normalized preservation rules and lifecycle definitions are
identical. Raw V2 snapshots differ only in the eight amount bytes, not lamports,
owner, address, length, enum, deadline, or arbitrator.

The genuine finding is `PRESERVE-amount-retained`, category `FIELD_NOT_PRESERVED`,
account `escrow`, field `amount`, BEFORE `1000000`, AFTER `900000`, status `FAIL`.
All other rules pass. The same lifecycle suite still passes: release executes with
the corrupted recorded amount. This intentionally proves that successful operations
do not establish state preservation or economic correctness.

## Lifecycle Coverage

Each of the six configured scenarios is tested independently against both binaries:

- Standard migration: successful transaction and expected typed V2 state.
- Unauthorized migration: configured stranger rejected with custom code 6003.
- Repeat migration: first succeeds, second rejects with 3002; no reload or state loss.
- Rollback: a controlled migration argument causes a real write after resize/rent
  top-up, logs the attempted mutation, and returns 6007; raw target state is unchanged.
- Unmigrated release: V2 loads, migration is skipped, and old bytes reject with 3002.
- Post-migration release: beneficiary signs the actual operation; enum becomes `Released`.

Configured ordering, fresh V1 worlds for independent scenarios, and continuous
state within dependent steps are checked. Reversing the suite produces the same
per-scenario evidence. Repeated runs compare complete V1 snapshots, migration
evidence, preservation and lifecycle serialization, including stable finding IDs.
No runtime fields are stripped to make equality pass.

## Files and Reproduction

- `programs/dg-escrow-v1` and `programs/dg-escrow-v2`: small Anchor application.
- `fixtures/escrow/idl`: IDLs generated from the actual program sources.
- `examples/escrow-runtime` and `examples/escrow-runtime-broken`: same fixture,
  eleven preservation rules, and six lifecycle scenarios.
- `scripts/build-escrow.sh`: offline builds for all three binaries and both IDLs.
- `crates/dg-runtime/tests/escrow_r9.rs`: 19 acceptance tests covering the layers.
- `crates/dg-cli/tests/escrow_r9_cli.rs`: six tests for the existing commands.
- `docs/r9-core-baseline.sha256`: pre-implementation core hashes.

See the [Escrow fixture README](../examples/escrow-runtime/README.md) for the exact
build command, local Anchor override and demo sequence. All runtime CLI paths
(`build`, `migrate`, `verify`, `lifecycle`) were executed, including both candidates'
preservation and lifecycle reports. Successfully produced reports return exit 0,
including the broken candidate's preservation failure; no CI policy was added.

```bash
cargo test -p dg-runtime --test escrow_r9 --offline
cargo test -p dg-cli --test escrow_r9_cli --offline
cargo test --workspace --offline
cargo clippy --workspace --all-targets --no-deps --offline -- -D warnings
cargo fmt --all --check
sha256sum --check docs/r9-core-baseline.sha256
bash scripts/check-mvp.sh
```

Required binaries/IDLs must exist; tests fail with build instructions rather than
skipping. All transactions are local LiteSVM transactions with public test seeds.
Never deploy the fixture or fund those identities on a real cluster. R9 does not
implement Devnet compatibility, economic invariants, relationship graphs, replay,
CI gating, fuzzing, or a production-readiness verdict.

## Acceptance Closure

**R9 ACCEPTANCE GATE: PASS** (2026-09-27).

- Static analysis, R5 construction, R6 migration, R7 preservation and R8 lifecycle:
  all verified on DG Escrow through unchanged production core code.
- 19 Escrow runtime acceptance tests and six CLI tests pass.
- Good: migration succeeds, preservation 11/11, lifecycle 6/6.
- Broken: migration succeeds, preservation 10/11 with the sole amount violation,
  lifecycle 6/6. Both candidates use the same rules and expectations.
- Complete deterministic evidence matches across repeated runs.
- Core inventory and all 21 source hashes match the recorded pre-R9 baseline.
- All 336 workspace tests pass, including the full R5/R6/R7/R8 Vault regressions.
- Workspace Clippy with warnings denied and formatting checks pass.
- Original demo smoke passes: good 22/22, backing-defect candidate retains exactly
  its two expected failures; project registration and both coverage modes pass.
- The offline Escrow build script was run successfully, generating all three
  binaries and both IDLs. No cluster activity was performed.

Doppelganger has now demonstrated the same generic runtime migration, preservation,
and lifecycle pipeline on two materially different Anchor applications without
project-specific runtime logic. The generic runtime core is frozen for this demo
milestone; later productionization work has not begun.
