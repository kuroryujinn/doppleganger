# Regression reproduction and candidate comparison

Preserve a real failure before changing its inputs. A regression case answers whether an observed defect can be reproduced, not whether a program is universally safe.

## Portable whole-workflow cases

After [your first migration test](/docs/first-test):

```bash
doppelganger regression capture --from-report runs/vault-fail/report.json --store runs/vault-case
doppelganger regression replay runs/vault-case --output runs/vault-replay
```

The portable directory contains `manifest.toml`, `policy.toml`, `v1.bin`, `v2.bin`, two IDLs, `expected.json`, `case.json` and an optional frozen snapshot. It records exact inventory and input hashes. Capture reruns the normalized failure before sealing it; replay verifies those identities and writes new reports.

Keep the entire directory together. Replay can run without the original project. A matched failure returns process exit 0 with migration exit 1; mismatch exits 1. Tampering or incomplete inventory exits 2.

## Native finding or scenario cases

```bash
doppelganger regression create --from-report runs/vault-fail/report.json \
  --manifest runs/vault-broken-project --finding PRESERVE-balance-field --store runs/native-cases
```

Use `--scenario ID` instead of `--finding ID` for a lifecycle case. These cases retain original artifact references and recorded V1 state; they do not embed the same portable input inventory. Keep original paths valid.

## Compare a corrected V2

Using the directory returned by native `create`:

```bash
doppelganger regression reproduce CASE_DIRECTORY
doppelganger regression reproduce CASE_DIRECTORY \
  --candidate runs/vault-project/v2.bin --candidate-idl runs/vault-project/v2.json
```

The broken candidate returns `REPRODUCED`; the known-good candidate returns `NOT_REPRODUCED`. Both were verified against the same recorded base state. Normal native evaluation can exit 0 for either result.

Portable replay deliberately has no candidate override. Do not change its sealed manifest or binaries. Capture a native case for individual candidate comparison, or create a new normal prepared-project test for the complete corrected policy evaluation.

## Scope and invalid cases

Native cases cover preservation/lifecycle findings. Portable cases cover complete production failures, including static and relationship decisions. Missing-argument/artifact failures without evaluated input identities cannot be captured as full workflow regressions. A changed source artifact invalidates capture: recalculate evidence with a new test rather than rewriting a stored hash.
