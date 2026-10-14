# R8: Generic Lifecycle Scenarios

`runtime lifecycle` orchestrates R5 world creation and R6 migration/candidate
loading. It does not call R7 or infer economic requirements. Every independent
scenario builds one fresh world. Repeat and post-migration steps keep that same
world; they do not reload the candidate or rerun initialization/setup.

## Architecture

- `dg-config::lifecycle` validates eight fixed recipes and their expectations.
- `dg-runtime::lifecycle::LifecycleEngine` executes them in manifest order.
- `MigrationExecutor::execute_loaded` reuses R6's IDL encoder, account/signature
  binding, transaction executor, and snapshot machinery without replacing V2.
- `ScenarioStepRecord` retains setup transactions, program swaps, immutable raw
  and decoded account evidence, logs, compute units, errors, and expectations.
- IDs default to SHA-256 of parsed scenario content. Explicit IDs are preferred;
  duplicate explicit/generated IDs are rejected. Ordering is never sorted by ID.

## Configuration

The default `[fixture]` remains unchanged. Optional `[fixtures.NAME]` entries are
complete existing `FixtureDefinition` values, not partial merge patches. They
support the same `account`, `initialize`, `setup`, and `payer` fields. A selected
variant runs through R5, never through manual scenario-specific byte mutation.

```toml
[[scenario]]
id = "normal"
type = "standard_migration"
expected = "success"

[[scenario]]
id = "wrong-authority"
type = "unauthorized_migration"
signer_account = "authority" # Exact signer-account name in the V2 IDL.
signer_override = "stranger" # Existing configured test identity or alias.
expected = "error"
expected_error = 2003        # Exact numeric custom/Anchor code, not a log name.
state_unchanged = true

[[scenario]]
id = "repeat"
type = "repeat_migration"
first = "success"
second = "error" # "success" is also supported; no_op is not a distinct outcome.

[[scenario]]
id = "operate"
type = "post_migration_instruction"
instruction = "operate"
account_mapping = { record = "$record", authority = "$owner" }
args = { value = "u64:99", fail = false }
expected = "success"
```

Operation mappings default to the configured migration mapping if omitted; supply
an explicit mapping when account sets differ. Instruction order, signer/writable
flags, discriminator, and argument types always come from the V2 IDL. `args` and
`arguments` are aliases. Signer overrides replace only the explicitly named IDL
signer account, not the payer or other signers; no new identity is generated.

| Type | Steps after fresh R5 build | Expectations |
|---|---|---|
| `standard_migration` | R6 migration | `expected` success/error |
| `unauthorized_migration` | Replace named signer binding, migrate | `expected`, optional exact error |
| `repeat_migration` | Migrate, execute same instruction on loaded V2 | Explicit `first`, `second` (alias `expected_second`) |
| `failed_migration_rollback` | Configured failing migration | Error and raw target equality |
| `unmigrated_state_against_v2` | Load V2, skip migration, execute instruction | Success or error, no assumed rejection |
| `post_migration_instruction` | Successful migration, configured operation | Expected operation outcome |
| `zero_value_state` | Select named fixture, migrate | Expected outcome |
| `boundary_value_state` | Select named fixture, migrate | Expected outcome |

Boundary scenarios require `fixture = "NAME"`. No boundary values are invented.
For example, a `[fixtures.maximum]` with an `initialize` action can set
`args = { value = "u64:18446744073709551615" }`; `[fixtures.zero]` can use zero.
Both still need their ordinary account bindings and payer. R8 does not assert that
arbitrary developer-named fixtures actually represent a domain boundary.

`[scenario.migration]` optionally replaces the complete migration definition for
that scenario, e.g. a `mode = 2` argument that triggers an atomic failure. This is
not permitted for the unmigrated recipe, which skips migration. The other ordinary
migration schema, account mapping, and typed argument rules remain unchanged.

`state_unchanged = true` compares selected R6 migration targets across the final
transaction: addresses, presence, owner, lamports, executable flag, length, and raw
bytes. It does not compare decoded schema representations. Equality is automatic
for rollback and a repeat step expected to reject. Fee payers are not exempt: if
selected, their transaction fee changes can cause a state-equality failure. Empty
observation sets cannot satisfy equality. Each changed account is diagnosed.

## Errors and Reports

`PASS` means a configured expectation matched. `FAIL` means runtime behavior or
selected raw state contradicted it. `ERROR` means configuration, world creation,
loading, instruction preparation/submission, or evaluation could not complete.
An engine error never satisfies expected program rejection. Without a numeric
`expected_error`, any program-level rejection can match. Names/log substrings are
not supported. A failed prerequisite stops dependent steps, not other scenarios.

JSON includes `lifecycle_evaluated: true`, `preservation_evaluated: false`, and
`policy_evaluated: false`. It includes public signer keys, not keypairs. With fixed
test seeds, the entire serialized report is deterministic, including signatures;
no evidence fields are discarded. Unseeded signers remain intentionally random.
Only use public test seeds for local fixtures, never production credentials.

An empty suite reports zero scenarios; it is not a safety verdict. Shape/ID errors
are fatal configuration errors. Scenario execution errors appear in the report.
Like R7, successfully writing a report returns command success even with failed
scenarios; no CI exit policy is added.

## Acceptance

The isolated non-Vault Record fixture covers all eight types, real rollback after
write/reallocation/transfer, exact error identity, raw equality, zero/max-u64
fixtures, world isolation, engine failures, and deterministic IDs/reports.

The seven configured Vault cases match an independent typed recipe retained from
the old demo: migration, unauthorized signer, repeat, rollback, unmigrated operation,
post-migration withdrawal, and limit rejection. The Vault rollback case is its old
unauthorized rejection with unchanged target state; the non-Vault fixture proves
rollback after actual attempted mutation. No new Vault boundary semantics are
invented. The old hardcoded demo and its economic checks remain unchanged.

The same scenario definitions pass 7/7 for good V2 and 6/7 for the acceptance-only
`r8-repeat-accepted` candidate. Its repeated migration is accidentally accepted;
normal migration still produces exactly the good candidate's raw state and decodes.
Only `repeat-migration` fails. See the [candidate build instructions](../examples/vault-runtime-r8-broken/README.md).

```bash
# Rebuild the non-Vault V2 fixture with its new operation/no-op instructions.
NO_DNA=1 cargo build-sbf --tools-version v1.57 --offline --skip-tools-install \
  --manifest-path programs/dg-r6-fixture-v2/Cargo.toml --sbf-out-dir target/deploy/r6-v2
cargo test -p dg-runtime --test runtime_r8 --offline
cargo test -p dg-runtime --test vault_r8_acceptance --test vault_r8_detection_acceptance --offline
cargo test -p dg-cli --test runtime_lifecycle_cli --offline
cargo run -p dg-cli --offline -- runtime lifecycle examples/vault-runtime --output report/lifecycle-good.json
cargo run -p dg-cli --offline -- runtime lifecycle examples/vault-runtime-r8-broken --output report/lifecycle-broken.json
cargo test --workspace --offline
cargo clippy --workspace --all-targets --no-deps --offline -- -D warnings
cargo fmt --all --check
```

Missing artifacts fail tests, never skip. The existing R5/R6 artifact build steps
remain required. Lifecycle evaluation does not prove multi-project generalization,
Devnet compatibility, relationship preservation, economic invariants, regression
replay, or CI deployment policy. No Escrow or later milestone is implemented here.

## Verified Closure

**R8 ACCEPTANCE GATE: PASS** (2026-09-26).

- 44 isolated R8 tests; all eight recipe types covered.
- Seven independent typed Vault equivalence tests, three candidate-discrimination
  tests, and two lifecycle CLI tests pass.
- Good candidate: 7/7 PASS. R8-broken candidate: 6/7 PASS, one expected
  `repeat-migration` failure, zero engine errors. Both reporting commands exit 0.
- Complete deterministic reports match across repeated runs for both candidates.
- 311 workspace tests pass, including all R5/R6/R7 regressions.
- Workspace Clippy with warnings denied and formatting checks pass.
- Runtime source scan finds no project-specific names or branches.
- Original demo smoke test remains intact: good 22/22, original backing-defect
  candidate has only its two expected backing failures; registration smoke passes.

R8 can generically orchestrate and evaluate configured migration lifecycle
scenarios without project-specific runtime logic. R8 is frozen at this boundary;
second-project acceptance has not begun.
