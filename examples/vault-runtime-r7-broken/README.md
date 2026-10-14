# R7 Acceptance-Only Broken Candidate

This candidate deliberately changes recorded balance from 1000000 to 950000
after the ordinary rent/backing top-up. Migration succeeds and produces a valid
V2 account. The existing `balance-field` equality rule fails; the other six
configured preservation rules pass.

The fixture, addresses, IDLs, migration instruction and rules match
`examples/vault-runtime`. Only project name and V2 binary path differ. The
acceptance test checks both manifest content and normalized rule equality.

Build into a separate directory, without `skip-backing-top-up`:

```bash
NO_DNA=1 cargo build-sbf --tools-version v1.57 --offline --skip-tools-install \
  --manifest-path programs/dg-vault-v2/Cargo.toml \
  --features r7-broken-balance --sbf-out-dir target/deploy/r7-broken
```

The original good and SOL-backing-defect artifacts and shared IDLs must already
exist; see the root README for their build instructions. The new feature changes
only the handler's value, so it uses the same V2 IDL and account discriminator.
Do not enable this test-only feature for normal program builds.

```bash
cargo test -p dg-runtime --test vault_r7_detection_acceptance --offline
cargo run -p dg-cli --offline -- runtime verify examples/vault-runtime-r7-broken \
  --output report/r7-broken.json
```

Expected: R6 migration success; R7 six passes, one failure, no errors. The CLI
still exits successfully as a reporting command. This is not the original
SOL-backing faulty candidate, whose documented R7 limitation remains unchanged.
