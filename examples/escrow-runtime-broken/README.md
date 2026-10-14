# DG Escrow Broken-Amount Candidate

Acceptance-only build of V2 with `r9-broken-amount`. Migration succeeds and produces
valid V2 state, but reduces the recorded amount by 100000 after the normal rent
top-up. Only those amount bytes differ from good V2; all other declared preservation
rules remain passing. The same six lifecycle expectations still pass, demonstrating
that successful operations do not imply preserved values or financial correctness.

This manifest differs from `../escrow-runtime/doppelganger.toml` only in project
name and V2 binary path. Do not change rules or expectations for this candidate.

Build both candidates with `bash scripts/build-escrow.sh` (see the good fixture
README for the local Anchor override), then run:

```bash
cargo run -p dg-cli --offline -- runtime verify examples/escrow-runtime-broken --output report/escrow-broken-preservation.json
cargo run -p dg-cli --offline -- runtime lifecycle examples/escrow-runtime-broken --output report/escrow-broken-lifecycle.json
```

Expected: migration success, preservation 10/11 with the single amount finding,
lifecycle 6/6. Reporting commands exit 0. Never deploy this fixture.
