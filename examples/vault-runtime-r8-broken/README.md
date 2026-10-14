# R8 Acceptance-Only Candidate

This deliberately flawed build accepts a repeated migration as a no-op. Initial
migration retains the good candidate's owner, balance, limit, lamports, address,
and raw serialized state. Never deploy this test-only candidate.

```bash
NO_DNA=1 cargo build-sbf --tools-version v1.57 --offline --skip-tools-install \
  --manifest-path programs/dg-vault-v2/Cargo.toml \
  --features r8-repeat-accepted --sbf-out-dir target/deploy/r8-broken
cargo run -p dg-cli --offline -- runtime lifecycle examples/vault-runtime-r8-broken \
  --output report/lifecycle-broken.json
```

Use the existing good V1 binary and V1/V2 IDLs. Do not combine defect features.
The manifest differs from `vault-runtime` only in project name and candidate
binary. Both suites have seven identical scenarios: good passes all seven; this
candidate fails only `repeat-migration` (expected error, observed success).
Both commands exit successfully as reporting commands, with no deployment policy.
The original backing-defect and R7-broken artifacts remain separate and unchanged.
