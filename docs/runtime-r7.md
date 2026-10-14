# R7: Generic Preservation Rules

R7 evaluates developer-declared preservation rules against generic R6 before/after
migration evidence. It does not yet run lifecycle scenarios, second-project
generalization, Devnet verification, regression generation, or CI policy.

## Architecture

- `dg-config::preservation` defines and validates optional `[[preservation]]` entries.
- `dg-runtime::preservation::evaluate(&execution, &rules, &v2_model)` is a pure
  evaluator. It borrows evidence and performs no VM, filesystem or RPC calls.
- Expected values use the V2 field schema, existing `dg_config::encode`, and
  `dg_state::decode_value`. This exposes the same bounded Borsh reader used for
  accounts. Integers remain lossless `BigInt` values and JSON decimal strings.
- `runtime verify` composes R5, R6, then R7 once. R6 evidence stays unchanged.

The explicit V2 model is needed because decoded integers do not retain original
signedness/width. Expected prefixes must match that schema and remain within range.
The model must correspond to the candidate that produced the evidence.

## Configuration

```toml
[[preservation]]
id = "amount-retained"
account = "record"
field = "amount"
rule = "equal"

[[preservation]]
account = "record"
field = "config.version"
rule = "default"
value = "u64:2"
```

Targets are R6 observation names, not account types or newly derived addresses.
Dot-separated struct fields are supported; empty segments, indexing and expressions
are rejected. Explicit IDs are preserved; absent IDs use SHA-256 of parsed rule
content. TOML formatting does not affect IDs. Duplicate IDs, including identical
unnamed rules, are rejected; repeated rules need distinct explicit IDs.

## Semantics

| Rule | Condition |
|---|---|
| `equal` | BEFORE field equals AFTER field |
| `not_equal` | BEFORE field differs from AFTER field |
| `initialized` | Field absent before, present after; zero and `None` are valid values |
| `default` | AFTER field equals configured value |
| `greater_or_equal` | AFTER integer is at least configured integer |
| `less_or_equal` | AFTER integer is at most configured integer |
| `exists` | AFTER account or decoded field is present |
| `not_exists` | AFTER account or decoded field is absent |
| `owner_preserved` | Raw account owner program IDs match |
| `lamports_preserved` | Raw lamports match exactly, without rent adjustment |
| `account_size_minimum` | AFTER length meets configured nonnegative minimum |
| `pda_preserved` | BEFORE and AFTER logical binding addresses match |

Equality uses existing typed values, including structs, enums/payloads, options,
and sequences. Arrays, tuples, bytes and vectors use sequence equality. Equal
decoded integers compare equal regardless of source width; width drift remains
a static-analysis concern. Numeric ordering never coerces strings or public keys.

Absence and error semantics:

- A missing observation is `TARGET_MISSING` or `UNKNOWN_ACCOUNT`, not an absent
  account. Duplicate logical names are `AMBIGUOUS_ACCOUNT`.
- A null snapshot means account absent. Account-level presence is evaluable.
- Field existence/absence on an absent or undecodable account returns `ERROR`.
  Missing fields inside decoded accounts are distinguishable and evaluable.
- `initialized` accepts a missing BEFORE field or absent BEFORE account. Missing
  AFTER account/field fails; a field already present BEFORE is `NOT_APPLICABLE`.
  Unavailable decoding is an error, never proof of field absence.
- Metadata comparisons needing snapshots return `ERROR` for absent accounts.
  Address continuity compares bindings even when an account is absent; require
  `exists` separately for presence. It does not prove PDA seed stability.
- Default/minimum/maximum need only AFTER values. Decode failures do not stop
  independent metadata rules or later rules.

## Reports and CLI

Reports contain normalized rules, ordered evaluations/findings, counts for `PASS`,
`FAIL`, `ERROR`, `NOT_APPLICABLE`, project name, migration signature/outcome and
scope markers. Violations generate findings; evaluation errors generate diagnostics
with precise codes. Finding IDs derive from rule IDs, not their failure index.
Findings retain expected and side-specific values/resolution errors and presence.
Configuration-shape/ID errors are fatal; malformed expected values and unresolved
targets become per-rule errors. No configured rules means no inferred requirements.

```bash
cargo run -p dg-cli --offline -- runtime verify examples/vault-runtime \
  --output report/preservation.json
```

CLI JSON contains original R6 evidence (including raw snapshots) plus the R7
report and `preservation_evaluated: true`, `lifecycle_evaluated: false`,
`policy_evaluated: false`. The nested R6 marker remains false, accurately describing
R6's scope. Reports contain public keys only, not signer secrets.

Rule failures/errors do not introduce CI exit-code policy: a produced report is a
successful command. Fatal configuration/execution/output errors return command
errors. Check migration outcome separately: unchanged state after a failed
migration can satisfy equality rules without implying migration succeeded.

## Vault Acceptance and Closure

The sample declares seven rules: owner field, balance field, new-field initialization,
default, raw owner, minimum size and address continuity. The good Vault passes all
seven, with overlapping outcomes matching the original demo's typed predicates.

**The existing faulty Vault's SOL-backing defect is not expressible using the R7
primitives.** It skips a top-up but retains recorded balance, owner, default and
address and reallocates correctly. Both binaries pass the seven applicable rules.
Exact lamport preservation fails the good binary's top-up and passes the faulty
binary, so it is deliberately absent from the sample rules.

Tests verify this limitation explicitly. A separate negative control declares an
expected default of 1000 instead of 500000 and obtains `DEFAULT_VALUE_MISMATCH`
on the faulty binary. This is **not** detection of the backing defect; the same
expectation would also fail on the good binary. No economic rule is invented.

The acceptance gap is now closed by the separately authorized
[`vault-runtime-r7-broken`](../examples/vault-runtime-r7-broken/README.md) candidate.
It uses the existing V2 program's acceptance-only `r7-broken-balance` build feature,
stored in `target/deploy/r7-broken`, and the same seven correct rules as the good
candidate. The feature changes only serialized balance after the normal top-up.
Neither generic R6/R7 implementation nor preservation semantics were changed.

| Candidate | R6 migration | R7 passed | R7 failed | R7 errors |
|---|---|---:|---:|---:|
| Good V2 | Success | 7 | 0 | 0 |
| R7-broken V2 | Success | 6 | 1 | 0 |

The genuine finding is `PRESERVE-balance-field`, category `FIELD_NOT_PRESERVED`,
account `vault`, field `balance`, rule `equal`, status `FAIL`, BEFORE `1000000`,
AFTER `950000`. V2 decoding, owner, size, address, and new-field initialization
remain valid. Raw V2 snapshots match the good candidate except for balance bytes.

`vault_r7_detection_acceptance` verifies that manifests differ only in project
name and candidate binary, normalized rule IDs/order/contents match, and deterministic
V1 snapshots match. Repeated deterministic broken runs produce identical execution
evidence and serialized preservation reports. The CLI test verifies that this
preservation failure still returns a successful reporting-command exit status.

**R7 acceptance gate: PASS.** This proves candidate discrimination under declared
R7 primitives, not detection of the original SOL-backing defect. That fixture and
its limitation tests remain unchanged. No economic or lifecycle rule was added.

No scripts, cross-account arithmetic, PDA rederivation, relationship graph,
static-hint enforcement, lifecycle framework or deployment verdict is introduced.

## Verification

```bash
cargo test -p dg-runtime --test runtime_r7 --offline
cargo test -p dg-runtime --test vault_r7_acceptance --offline
cargo test -p dg-runtime --test vault_r7_detection_acceptance --offline
cargo test -p dg-cli --test runtime_verify_cli --offline
cargo test --workspace --offline
cargo clippy --workspace --all-targets --no-deps --offline -- -D warnings
cargo fmt --all --check
```

The isolated suite uses synthetic evidence without constructing LiteSVM worlds.
Vault acceptance and CLI tests require good, backing-defect, and R7-broken SBF
binaries and IDLs; missing artifacts fail rather than skip. Build the R7-broken
variant using its linked fixture README before running workspace tests. R5/R6
remain regression checks. No final integrated runtime matrix is introduced.

Verified locally: 96 isolated R7 tests, four existing Vault acceptance tests,
three closure tests and two R7 CLI tests pass; all 255 workspace tests pass.
Workspace-wide Clippy with warnings denied and formatting checks pass. Both CLI
commands succeed and produce `report/r7-good.json` and `report/r7-broken.json`
with the counts above. The static analyzer's pre-existing
eight-argument finding helper received a local Clippy allowance, with no behavior
change, to allow the requested workspace-wide lint command.
