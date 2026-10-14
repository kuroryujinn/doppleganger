# DG Escrow Runtime Acceptance

A local-only two-party escrow fixture, not a deployment-ready escrow protocol.
V1 requires both parties to sign initialization of a fixed positive amount. The
initializer funds it exactly once, moving `Created` to `Funded`. The PDA includes
both parties. V2 migrates that existing account and permits beneficiary-signed
release from `Funded` to `Released` before the configured deadline.

V2 adds a signed `i64` deadline and arbitrator public key, appends `Disputed` to the
status enum, and moves the status field from byte 80 to byte 120. Account storage
grows from 81 to 121 bytes, including the discriminator. The first four enum tags
remain compatible. The migration explicitly retains the old values, initializes
new fields from typed arguments, and funds any rent shortfall. This exercises two
actors, enum state, a different signer for the post-migration operation, and a
different instruction/account mapping than Vault.

`Refunded` and `Disputed` are reserved states in this small fixture; refund,
arbitration, new V2 escrow creation, and expiry recovery are not implemented.
The arbitrary local program ID has no deployment key. Signer seeds in the manifest
are public deterministic test identities. Never fund them on a cluster.

## Build

Use installed platform-tools v1.57, Anchor 1.2.0, and cached locked dependencies:

```bash
bash scripts/build-escrow.sh
```

On the current machine the Anchor wrapper selects an unavailable Solana version;
use the installed binary directly:

```bash
ANCHOR_BIN=/home/kuro/.avm/bin/anchor-1.2.0 bash scripts/build-escrow.sh
```

The script creates V1, good V2, and a separate acceptance-only broken V2, then
generates both IDLs from Anchor. Good and broken V2 have the same interface.

## Demo Sequence

```bash
cargo run -p dg-cli --offline -- analyze --from fixtures/escrow/idl/v1.json --to fixtures/escrow/idl/v2.json --account EscrowV1:EscrowV2
cargo run -p dg-cli --offline -- runtime build examples/escrow-runtime --output report/escrow-v1-world.json
cargo run -p dg-cli --offline -- runtime migrate examples/escrow-runtime --output report/escrow-migration.json
cargo run -p dg-cli --offline -- runtime verify examples/escrow-runtime --output report/escrow-preservation.json
cargo run -p dg-cli --offline -- runtime lifecycle examples/escrow-runtime --output report/escrow-lifecycle.json
cargo run -p dg-cli --offline -- runtime verify examples/escrow-runtime-broken --output report/escrow-broken-preservation.json
cargo run -p dg-cli --offline -- runtime lifecycle examples/escrow-runtime-broken --output report/escrow-broken-lifecycle.json
```

Expected matrix: good migration succeeds, preservation 11/11, lifecycle 6/6.
Broken migration succeeds, preservation 10/11, lifecycle 6/6. Its sole finding is
`PRESERVE-amount-retained`, `FIELD_NOT_PRESERVED`, amount 1000000 to 900000.
All runtime commands report evidence without enforcing CI or deployment policy.

The 11 preservation rules cover both parties, amount, enum status, deadline
initialization/default, arbitrator initialization/default, owner, address and size.
The six lifecycle scenarios cover standard migration, wrong signer, repeat,
rollback after attempted writes, unmigrated release, and post-migration release.
There is no inferred SOL-backing rule, relationship graph, or economic assertion.

The `fail_after_write` migration argument is an intentional test-only fault path:
the program resizes, tops up rent, writes account data, then rejects with code 6007.
The R8 rollback check compares actual account snapshots. Do not deploy this fixture
or confuse its controlled failure path with production protocol functionality.

See [the R9 audit](../../docs/runtime-r9.md) for the core baseline and evidence.
