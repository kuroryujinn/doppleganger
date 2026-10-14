# Real Devnet Vault Capture

**REAL DEVNET CAPTURE**, not mock. Canonical capture slot: 507756125.
Program: `DTbPK89xmJH65ay3YyJTZnPnDWsyXU3KNLiEvXQ7bjoT`.
Vault: `9D4qd9qyKzi5rf3CWw7c5hfcLxh4jYofEsGZ6xtgFgns`.

`snapshot.json` is immutable. Both candidates and the R10 case use its exact
state; never refresh it to run offline tests. Snapshot content SHA-256:
`550545bdee55508afffdecbb9afc07b18cf9acd2b48d4ba739b5cf315013a47f`.

`artifacts/` retains exact V1, GOOD V2, and BROKEN V2 SBF ELF binaries as `.bin`
files plus the matching IDLs. V1 was independently dumped from Devnet and
hash-matched to `v1.bin`. Neither V2 was deployed remotely. BROKEN uses the
existing `r7-broken-balance` feature and the same V2 IDL.

`evidence/` retains setup signatures, finalized public wallet/owner signature
inventories, independent account inspection, GOOD/BROKEN reports, R10 results,
and live/offline verification logs. Wallet-named files are transaction histories,
not wallet secrets. `regression/` is the unmodified, hash-pinned R10 bundle.
Its absolute artifact paths refer to the acceptance machine; regenerate it
using the commands below on another checkout. No private wallet, program,
buffer keypair, seed phrase, or API token is included.

## Offline Replay

From the repository root, after building `dg-cli`:

```bash
export DG_DEVNET_RPC_URL=http://127.0.0.1:1
target/debug/doppelganger snapshot inspect fixtures/devnet-snapshots/vault-devnet
target/debug/doppelganger snapshot replay fixtures/devnet-snapshots/vault-devnet \
  --project examples/vault-runtime-devnet --lifecycle --output report/live-good.json
target/debug/doppelganger snapshot replay fixtures/devnet-snapshots/vault-devnet \
  --project examples/vault-runtime-devnet --lifecycle \
  --candidate fixtures/devnet-snapshots/vault-devnet/artifacts/v2-broken.bin \
  --output report/live-broken.json
target/debug/doppelganger regression create --from-report report/live-broken.json \
  --manifest examples/vault-runtime-devnet --finding PRESERVE-balance-field
```

Use the returned regression directory with `regression reproduce`. BROKEN must
return `REPRODUCED`; `--candidate fixtures/devnet-snapshots/vault-devnet/artifacts/v2-good.bin`
must return `NOT_REPRODUCED`. The retained bundle also reproduces directly on
the acceptance machine:

```bash
target/debug/doppelganger regression reproduce fixtures/devnet-snapshots/vault-devnet/regression
```

Normal `cargo test --workspace --offline` checks exact imported BEFORE state,
same-snapshot candidate comparison, R10 reproduction, and rejection of a
tampered snapshot copy. It does not contact Devnet. The separately gated live
test is documented in [R11 runtime](../../../docs/runtime-r11.md).

## Accounting

Setup sent 151 confirmed transactions: 148 loader/upload/deployment, one owner
funding transfer, initialize, and deposit. Counts exclude the two pre-existing
user funding signatures and de-duplicate the owner transfer appearing in both
inventories. R11 itself sent **zero** write RPC calls and zero remote migration
transactions. The authority's seed-73 identity is explicitly public, disposable,
and Devnet-only; never use it for assets with real value.

Full hashes and public setup identifiers:
final acceptance report.
