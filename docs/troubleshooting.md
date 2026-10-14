# Troubleshooting

Run `doppelganger doctor PROJECT --json` first. Doctor checks manifest/IDL/program identity, binary format, policy and deterministic local identities without running migration or RPC. Missing build tools produce warnings because prepared artifacts can execute without them.

| Symptom | Correction |
| --- | --- |
| Exit 1 with a static finding | Review the exact difference. Approve only intended classification/path/before/after values or explicitly choose warning policy. |
| Exit 1 with preservation/relationship/lifecycle failure | Inspect typed before/after state and transaction logs. Fix migration or the incorrect declared expectation; static approvals do not hide runtime violations. |
| Exit 2 unsupported version/unknown field | Use schema_version 1 and the documented field names; do not delete a hash from a frozen artifact. |
| Exit 2 missing policy/binary/IDL | Correct paths relative to the manifest and build both program versions/IDLs. |
| Exit 2 legacy discriminator/unsupported serializer | Rebuild/convert to an explicit-discriminator Borsh IDL; custom/zero-copy formats are unsupported. |
| Exit 3 runtime setup or missing state evidence | Inspect retained execution records; check signer bindings, funding, instruction-created observations and V1 fixture setup. |
| Exit 4 output exists | Pick a fresh output directory. Prior reports are intentionally never overwritten. |
| Exit 4 input changed | Freeze binaries/IDLs/config/policy/snapshot for the duration of the run and retry. |
| Registry corruption | Save the diagnostic, run `project recover`, then re-register any latest entry lost when restoring the backup. |
| Portable replay integrity error | Restore an intact full case directory. Every listed file and hash is required. |
| Anchor/Solana tool unavailable in doctor | Prepared tests still run; install the pinned build tools only when compiling artifacts or capturing Devnet state. |
| Build script offline cache miss | Provision Rust dependencies and Solana platform-tools before rerunning the offline script. |
| Snapshot authority mismatch | Use the explicit controlled test signer from the imported fixture; never forge or fetch a production private key. |

A captured regression that replays successfully still represents a failed migration. Read `matched`/`REPRODUCED` separately from the migration's own result/exit code. Use the native candidate override or a new normal production run to qualify a fix.
