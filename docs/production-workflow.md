# Production CLI workflow

This milestone joins the implemented engines into one policy-controlled local test.
It adds declared account relationships and upgrade decisions before APIs or frontend
development. Release hardening and qualification are recorded in MVP acceptance.

## Run a prepared project

The directory contains `doppelganger.toml`, V1/V2 artifacts and IDLs at the configured
paths, and a separate versioned `doppelganger-policy.toml`.

```bash
cargo run -p dg-cli --offline -- migration test examples/vault-runtime --ci
cargo run -p dg-cli --offline -- test examples/escrow-runtime --json
```

Both spellings execute the same pipeline:

```text
Validate manifest and policy → hash inputs → analyze IDLs
    → build V1 state → execute V2 migration → preservation
    → account relationships → fresh-world lifecycle scenarios
    → policy → JSON / HTML / Markdown reports → process exit
```

The default report directory is `.doppelganger/runs/<run-id>-<process-id>/`.
`--output NEW_DIRECTORY` selects a fresh output directory; existing directories
are never overwritten. Successful and policy-failing evaluations contain complete
before/after evidence. Configuration/runtime failures save structured diagnostics;
argument parsing and output-write failures print diagnostics to stdout.

`--json` prints the complete report as JSON. Normal output prints the verdict and
report path. `--ci` documents CI intent; the exit-code contract applies with or
without that flag. A report being written successfully no longer implies success.

| Exit | Meaning |
| --- | --- |
| 0 | All required checks pass under the supplied policy; warnings may remain |
| 1 | Policy violation, migration rejection, failed check, or missing required coverage |
| 2 | Invalid arguments, missing/invalid configuration or artifacts, invalid snapshot |
| 3 | Runtime setup/execution failure or check evaluation error |
| 4 | Internal serialization, input mutation, or output-write error |

There is no safety score. The verdict covers the supplied fixture/snapshot, rules,
relationships and scenarios. Existing `runtime verify/lifecycle` commands retain
their evidence-only behavior; use `test` for policy-controlled process exits.

## Account relationships

Policy schema version 1 requires explicit declarations:

```toml
schema_version = 1

[[relationship]]
id = "vault-authority"
kind = "authority"
account = "vault"
field = "owner"
target = "$owner"

[[relationship]]
id = "vault-owner-program"
kind = "program_owner"
account = "vault"
target = "$program"

[[relationship]]
id = "vault-address"
kind = "pda"
account = "vault"
target = "$vault"
```

Supported kinds: `authority`, `owner`, `reference`, `parent`, `mint`,
`program_owner`, and `pda`. `owner` checks a decoded field; `program_owner`
checks account metadata. Field edges require a public-key value, support nested
struct paths, and can specify `before_field` for a renamed field.

References, parents, and mints must point to an existing observed account in both
states. Include those accounts in the manifest's `migration_targets`; optional
type bindings permit raw-only observations. An address binding alone does not
prove that a referenced account exists.

Authority/owner targets may be literal public keys or resolved `$bindings` without
requiring an observed identity account. PDA checks require a `$binding` declared
as a PDA in the manifest; a literal binding cannot masquerade as a derivation.
The existing generic address resolver supplies the actual derived address.

By default edges are checked before and after migration and must be preserved.
For an explicitly added/redirected edge, `check_before = false` and
`preserve = false` check the declared AFTER target only. This does not approve
the corresponding static layout change; policy still handles it independently.

Missing source evidence, decoding problems, wrong types and ambiguous accounts
produce evaluation errors. Absent accounts, dangling links and changed values
produce violations. Neither can yield PASS. `dg-policy` evaluates immutable
account records and public bindings without executing LiteSVM or making RPC calls.
This is a declared graph, not automatic discovery of every semantic relationship.

## Expected changes and required coverage

All unexpected static findings block by default. Approvals require exact
classification, path, BEFORE and AFTER JSON plus a reason:

```toml
[[expected_change]]
classification = "ACCOUNT_SCHEMA_CHANGE"
path = "account.Record.limit"
before_json = "null"
after_json = '{"kind":"primitive","value":"u64"}'
reason = "Approved new per-account limit"
```

Copy the exact path and values from the static report after reviewing the change.
No wildcard matching or generated automatic approval is used during a run.
An unobserved/stale expected-change declaration blocks, as does an unexpected
new value at an otherwise approved path.

If a project deliberately wants advisory static analysis, it can set:

```toml
[policy]
unexpected_static = "warn" # fail (default), warn, or ignore
```

This setting cannot downgrade preservation, relationship, lifecycle or migration
failures, and ignored/warning findings remain counted in the report.

The default policy requires nonempty preservation, relationship and lifecycle
coverage. A developer can explicitly set `require_preservation`,
`require_relationships` or `require_lifecycle` to false when that area is outside
the intended check; configured failures still block. Configuration typos,
unknown fields and unsupported profile versions fail early.

Vault and Escrow profiles approve their specific known fixture upgrades and
declare authority, owner and PDA expectations. They are examples to review,
not blanket production policies. The same profiles apply to the deliberately
broken candidates:

```bash
cargo run -p dg-cli --offline -- test examples/escrow-runtime-broken \
  --policy examples/escrow-runtime/doppelganger-policy.toml --ci
# Expected exit: 1, sole amount-preservation defect.
```

## Frozen Devnet state

```bash
cargo run -p dg-cli --offline -- test examples/vault-runtime-devnet \
  --policy examples/vault-runtime/doppelganger-policy.toml \
  --snapshot fixtures/devnet-snapshots/vault-devnet --ci
```

This uses the saved snapshot. No RPC client, recapture, remote signing,
deployment or remote migration is invoked by the workflow. The combined report
retains the R11 snapshot contract so R10 can create snapshot-derived regressions.
Signer-dependent operations still require explicitly controlled matching local
test identities. The workflow rejects random signer generation to keep repeated
reports deterministic.

## Integrity and regression compatibility

Every evaluated report identifies SHA-256 hashes for V1/V2 binaries, IDLs,
manifest, policy and optional snapshot. Inputs are hashed before the run and
rechecked afterwards; changed inputs invalidate the result. The run ID derives
from those content hashes and workflow schema version. Reports do not serialize
private signing material or the manifest's public fixture seed arrays.

Preservation and lifecycle failures from combined reports can be passed to the
existing `regression create` command, followed by `regression reproduce` and an
optional corrected candidate override. Relationship and static-policy findings
are supported by portable `regression capture/replay` whole-workflow cases; native individual cases remain preservation/lifecycle only. Source artifacts for native cases remain local
path references rather than portable embedded binaries.

## Local CI acceptance and GitHub workflow

After provisioning Rust, cached locked dependencies, Solana `cargo-build-sbf`
with platform-tools v1.57 and Anchor 1.2.0:

```bash
bash scripts/build-production-fixtures.sh
bash scripts/check-production-mvp.sh
```

On the current machine use `ANCHOR_BIN=/home/kuro/.avm/bin/anchor-1.2.0` for the
build script. It builds all required acceptance candidates and generates IDLs.
The check script runs formatting, Clippy, workspace tests and full good/broken
policy acceptance, retaining JSON/HTML/Markdown evidence under `report/`.
The broken fixture must exit 1; an unexpected PASS or runtime/configuration error
fails the acceptance script.

`.github/workflows/production-mvp.yml` supports trusted same-repository PRs,
main/master pushes, manual dispatch and reusable workflow calls. Fork PR code is
excluded from the provisioned Linux self-hosted runner with the custom `doppelganger`
label. Provision the pinned build tools, cached dependencies and cargo-audit 0.22.2.
It builds fixtures, audits dependencies, runs acceptance, captures/replays a portable
failure case and uploads reports even when acceptance fails. Remote Devnet tests
remain gated. No hosted GitHub run is claimed until it executes there.

The runner-label and action configuration follows [GitHub runner guidance](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/use-in-a-workflow),
[checkout documentation](https://github.com/actions/checkout/blob/v4/README.md),
and [artifact upload documentation](https://github.com/actions/upload-artifact/blob/v4/README.md).
Third-party actions are pinned to verified release commit hashes.

## Generic loader correction

Full-project validation exposed a generic configuration bug: `ProjectManifest::load`
encoded public-key arguments before resolving `$address` references, while runtime
execution resolved them correctly. The loader now applies the shared resolver before
encoding. A dedicated test checks the resolved public-key bytes. The historical R9
source audit explicitly classifies this generic fix and strips only its recorded
preprocessing addition before comparing the earlier core baseline; no application
names or fixture branches were added to production code.

## Release qualification and next phase

The local 0.1.0 MVP qualification closes registry/versioning, compatibility, resource
limits, seeded parser stress, dependency audit, scale checks, diagnostics, installation,
portable reproduction and standalone example packaging. See MVP acceptance
for exact evidence and operational limitations. APIs, uploaded-source compilation,
folder transport and frontend are the next product phase.
