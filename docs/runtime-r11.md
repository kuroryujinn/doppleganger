# R11: Devnet Snapshot and Local Replay

**Current acceptance:** genuine Devnet application capture and offline replay
passed on 2026-10-05; see final acceptance. The historical
mock-only boundary below has been superseded by the retained real Vault fixture.
The [production workflow](production-workflow.md) can now add relationship and
policy decisions to a frozen snapshot replay.

R11 adds a read-only source to the existing R5-R8 runtime. The adapter exposes
only `getGenesisHash` and one bounded `getMultipleAccounts` batch. Its private
transport accepts an enum of these reads, not arbitrary method names. It has no
transaction, airdrop, deployment, or upgrade interface.

## Capture and inspect

Add explicit inputs to a project manifest:

```toml
[snapshot]
source = "devnet"
rpc_url_env = "SOLANA_DEVNET_RPC_URL" # optional; default is public Solana Devnet
min_context_slot = 123456           # optional
max_account_bytes = 1048576        # default; hard ceiling is 10 MiB

[[snapshot.accounts]]
name = "vault"
address = "$vault"
# Migration targets default to the configured V1 program owner.

[[snapshot.accounts]]
name = "owner"
address = "$owner"
owner = "$system_program"
optional = false
```

`rpc_url` and `rpc_url_env` are mutually exclusive. Only Devnet is supported;
genesis identity must match, or capture stops. Accounts are captured in one
finalized batch, with a shared context slot, up to 100 explicit addresses and
16 MiB of total decoded data. There is no account crawler or fallback fetch.
Additional CPI programs can be declared as `[[snapshot.programs]]` with
`program_id` and a local `program_path`; their binary hashes are pinned at
capture. Built-in programs should use LiteSVM's existing accounts.

```bash
cargo run -p dg-cli -- snapshot capture examples/vault-runtime-devnet
cargo run -p dg-cli -- snapshot capture examples/vault-runtime-devnet --output .doppelganger/snapshots/my-capture
cargo run -p dg-cli -- snapshot inspect .doppelganger/snapshots/my-capture
```

The single `snapshot.json` file has schema version 1, exact raw bytes and account
metadata (including rent epoch), data and record SHA-256 hashes, decoder status,
provenance, and a complete snapshot hash. Decoder errors retain raw bytes.
Required absent accounts stop capture; optional absent accounts are explicit
`exists=false` records. RPC errors never become absent accounts.

The `SNAP-<sha256>` identity excludes capture time. The complete hash includes
capture time and provenance. Matching identities reuse the original immutable
file. Different state cannot overwrite it. Scheme, host, and port are retained
as the origin; user information, URL paths, query strings, and fragments are
excluded from evidence. Connection settings are also removed from R10 bundles.
Hashes detect changes relative to the recorded hashes; they are not signatures
or independent proof that an RPC provider returned authentic chain state.

## Offline replay and candidate comparison

```bash
cargo run -p dg-cli -- snapshot replay .doppelganger/snapshots/my-capture \
  --project examples/vault-runtime-devnet --lifecycle --output report/replay.json
cargo run -p dg-cli -- snapshot replay .doppelganger/snapshots/my-capture \
  --project examples/vault-runtime-devnet --candidate /path/to/another-v2.so \
  --candidate-idl /path/to/v2.json --output report/another-candidate.json
```

Replay verifies integrity before building a VM, runs no fixture initialization
or setup transactions, and restores all configured accounts exactly. Local
V1/V2 binaries and IDLs remain required. The optional V2 IDL override requires
an explicit binary override and the same program ID. Both candidates use the
same stored input, with separately reported artifact hashes.

Imported addresses are observation-only unless an explicit deterministic local
test signer derives the exact public key. Missing authority gives a structured
`MISSING_LOCAL_SIGNER` error; mismatched imported signer bindings give
`SIGNER_ADDRESS_MISMATCH`. Existing R8 scenarios requiring unavailable keys
return structured scenario errors. Alternate initialization fixtures cannot
replace the pinned snapshot. No signing authority is inferred from account data.

R6 executes the migration; R7 evaluates the existing rules; `--lifecycle` runs
existing R8 recipes with fresh imported worlds. The only core extension is R8's
generic world-builder callback. The R9 baseline test removes that callback and
the additive config/CLI declarations before checking the historical hashes;
all recipe bodies and migration/preservation code still match the baseline.

Reports include the source snapshot, slot, hashes, candidate hashes, execution,
preservation, and lifecycle evidence. `remote_transactions_sent=false`,
`rpc_write_calls=0`, and `migration_executed_locally=true` are explicit. Mock
inputs also carry `acquisition=mock_rpc` and `remote_state_imported=false`.
Evaluation failures are report evidence, consistent with existing runtime CLI
commands; they do not introduce exit-code policy. Argument, integrity, and
execution errors return a nonzero exit status with structured diagnostics.

## Snapshot-derived R10 regressions

```bash
cargo run -p dg-cli -- regression create --from-report report/replay.json \
  --manifest examples/vault-runtime-devnet --finding PRESERVE-balance-field
cargo run -p dg-cli -- regression reproduce REG-<id>
```

`--scenario ID` works for a lifecycle failure in the combined replay report.
The bundle uses `snapshot_replay` and copies `snapshots/source.json`, including
snapshot ID, hash, slot, account hashes, and provenance. R10 bundle hashes pin
this file. V2 overrides from the source report are verified and pinned, so the
original project manifest can be supplied unchanged. Creation verifies the
failure locally; reproduction needs neither the report nor RPC access.

## Offline acceptance fixture

The original offline acceptance used local fixture program IDs. The later
`fixtures/devnet-snapshots/vault-devnet` fixture contains a genuine deployed
and initialized application capture with matching retained artifacts and public
provenance. The final acceptance report distinguishes it from the mock below.

`fixtures/devnet-snapshots/escrow-mock/snapshot.json` is a **mock RPC capture of
R5-created local state**, not a real Devnet capture. It deliberately uses a
mock origin, slot 42, capture time 0, and public test identities. Its V1 IDL is
pinned; its binary reference is unpinned to permit rebuilding the local SBF
fixture with another toolchain. It must not be relabeled as remote evidence.

```bash
bash scripts/build-escrow.sh
cargo run -p dg-cli -- snapshot inspect fixtures/devnet-snapshots/escrow-mock
cargo run -p dg-cli -- snapshot replay fixtures/devnet-snapshots/escrow-mock \
  --project examples/escrow-snapshot-offline --lifecycle --output report/r11-good.json
cargo run -p dg-cli -- snapshot replay fixtures/devnet-snapshots/escrow-mock \
  --project examples/escrow-snapshot-offline \
  --candidate target/deploy/escrow-broken/dg_escrow_v2.so --lifecycle \
  --output report/r11-broken.json
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo fmt --all -- --check
```

Expected offline results: good migration succeeds, preservation 11/11,
lifecycle 6/6. The same snapshot and broken candidate migrate successfully but
produce `PRESERVE-amount-retained` with preservation 10/11. R10 reproduces both
preservation and lifecycle failures without RPC. HTTP mock tests record exactly
the two allowed read methods and zero writes, then stop the server before replay.

The explicitly gated live test captures a built-in Devnet account without
deploying or funding anything. It does not establish application replay acceptance:

```bash
DG_RUN_DEVNET_TESTS=1 cargo test -p dg-snapshot live_devnet_capture_only_when_explicitly_enabled -- --nocapture
```

`DG_DEVNET_RPC_URL` optionally overrides that test's endpoint. Without the gate,
no live request runs and the test prints `LIVE DEVNET INTEGRATION: NOT RUN`.
Real-state application acceptance has since passed using the retained Vault
snapshot. For a new application, prepare a fixture or supply a saved capture;
see [the one-time preparation steps](devnet-fixture-setup.md).

After preparing that disposable application, the separate ignored integration
test requires its manifest explicitly, checks initialized application targets,
and verifies exact local restoration. It never substitutes a builtin account:

```bash
DG_RUN_DEVNET_TESTS=1 DG_DEVNET_PROJECT=/absolute/path/to/live/project \
  cargo test -p dg-snapshot --offline live_devnet_application_capture -- --ignored --nocapture
```

This test performs a new read-only capture; do not use it to recapture between
the GOOD and BROKEN canonical acceptance replays. Cargo's `--offline` prevents
dependency downloads, not this explicitly gated RPC call.
