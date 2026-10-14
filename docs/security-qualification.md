# Security qualification — 2026-10-08

The local release gate includes formatting, Clippy with warnings denied, all workspace tests, seeded parser mutation suites and RustSec dependency audit. Exact logs and audit JSON live under `report/mvp-qualification/` and are retained in the local release qualification bundle. No external penetration test or security audit is claimed.

## Input and integrity review

- IDL input size, JSON depth/node count and defined-type traversal work are bounded before recursive schema interpretation. Cyclic/exponential custom schemas fail explicitly. Zero-copy/serialization/repr flags cannot silently select a Borsh interpretation.
- Manifest/profile versions and dangerous unknown fields fail. Pre-versioned manifests remain distinguishable. Duplicate bindings/targets/account observations, unknown/circular address references and unsupported comparisons fail.
- Regular-file reads are bounded; production artifact reads, runtime V1/V2 reads and native regression reads have budgets. ELF validation is delegated to the VM after a format check.
- Borsh byte reads use checked offsets; booleans/options/enums, lengths, truncations, invalid UTF-8, integer boundaries, trailing bytes and decoding depth/value limits have independent tests. Raw/decoded snapshot consistency and version/metadata validation are preserved.
- Acquisition validates genesis/slot/account sizes and frozen snapshot hashes. Replay verifies program and IDL identity and makes no RPC call. Reports hash inputs before and after execution; input mutation invalidates the result.
- Registry updates hold an OS lock through backup and rename, sync files/directories, reject symlink registry paths and refuse corrupt state. Twenty-four simultaneous CLI writers retain every entry. Recovery is explicit and validated.
- Fresh report/case output directories cannot overwrite input files or previous results. Portable case inventories disallow arbitrary relative paths and verify canonical containment plus every content hash before replay. Case capture verifies that copied inputs reproduce the source result. HTML escapes evidence strings.
- Local manifest artifact paths may intentionally leave the project directory. This is developer-controlled local execution; an upload/API service requires a separate filesystem/process isolation boundary before use with hostile projects.

The original R9 hash inventory remains historical evidence for configuration-only Escrow integration. Later CLI/parser/file-read changes are explicitly classified in its test. Unchanged analysis/preservation/address/scenario sources still satisfy their historical hashes, and current generic behavior is qualified by the new tests. The historical test does not pretend later hardening left every source byte unchanged.

## Seeded mutation coverage

2,000 IDL JSON mutations/truncations, 1,000 TOML mutations/truncations and 5,000 malformed Borsh byte sequences run in ordinary tests. Additional targeted fixtures cover size/depth/node/type-work limits, recursive types, bytemuck/repr rejection, unknown schema versions, snapshot tampering and malformed ELF. These are reproducible bounded parser stress tests, not an exhaustive libFuzzer campaign or Solana vulnerability fuzzer.

## Dependency review

`cargo-audit 0.22.2` found rustls 0.23.43 affected by [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html). Cargo.lock now pins patched rustls 0.23.45. A fresh audit reports **zero known vulnerabilities** and the following upstream maintenance notices. They are not suppressed by an ignore list.

| Package | Notice | Dependency route / disposition |
| --- | --- | --- |
| ansi_term 0.12.1 | RUSTSEC-2021-0139 | LiteSVM diagnostic formatting; retain with pinned VM, replace when upstream migrates |
| bincode 1.3.3 | RUSTSEC-2025-0141 | Anchor/Solana and VM serialization; direct replacement would change upstream wire contracts |
| derivative 2.2.0 | RUSTSEC-2024-0388 | arkworks proc macro through Poseidon syscall dependencies; compile-time use |
| libsecp256k1 0.7.2 | RUSTSEC-2025-0161 | Solana secp256k1 syscall implementation; upstream VM compatibility required |
| paste 1.0.15 | RUSTSEC-2024-0436 | arkworks proc macro through Poseidon syscall dependencies; compile-time use |

Run the current audit before distribution and on dependency updates; this dated result does not promise future advisory status. Avoid replacing VM serialization/cryptographic dependencies solely to hide maintenance warnings. The release archive records Cargo.lock, dependency license metadata and available upstream license/notice files.
