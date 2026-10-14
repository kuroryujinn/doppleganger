# Supported formats and limits — 0.1.0

The release is qualified for prepared Anchor/Borsh projects on Linux x86_64 (including WSL2), with Rust 1.98.0 for source builds, LiteSVM 0.16.0, Anchor CLI/lang 1.2.0 and Solana platform-tools v1.57. Prepared binaries run without those build tools. Solana CLI 3.1.10 was used locally. Other host platforms/toolchain versions require their own qualification.

| Input | Static normalization | Account decoding | Full SBF execution evidence |
| --- | --- | --- | --- |
| Anchor 1.2.0 explicit-discriminator Borsh IDLs | Qualified | Qualified | Vault and Escrow fixed/broken upgrades; Record fixture |
| Anchor 0.30.1 explicit-discriminator IDL | Qualified using pinned upstream generated fixture | Qualified against known u32 wire bytes | No separately compiled 0.30.1 program execution claimed |
| Legacy inline account IDL from upstream compatibility fixture | Qualified for supported static shapes | Explicit UNSUPPORTED when discriminator is absent | Unsupported until converted/rebuilt with explicit discriminators |
| Zero-copy/bytemuck/custom repr | Rejected | Unsupported | Unsupported |
| Generic defined types/custom serializers | Rejected | Unsupported | Unsupported |

Supported Borsh primitives are bool, u8–u128, i8–i128, public keys, String and bytes. Structs, enum unit/tuple/named payloads, nested defined types, Vec, Option, arrays and tuples are supported within budgets. Floats, 256-bit integers and other custom encodings are not supported by runtime encoding/decoding; static analysis may describe those primitives. JSON metadata not affecting supported wire interpretation may be retained/ignored, but dangerous serialization/repr metadata is rejected. Raw account bytes are always retained; trailing bytes are rejected or explicitly preserved according to the call's mode.

## Fixture provenance

`tests/compatibility/anchor-0.30.1/external.json` and `external-legacy.json` are copied byte-for-byte from the [upstream Anchor v0.30.1 declare-program IDLs](https://github.com/coral-xyz/anchor/tree/e6d7dafe12da661a36ad1b4f3b5970e8986e5321/tests/declare-program/idls). They are pinned to commit e6d7dafe12da661a36ad1b4f3b5970e8986e5321; the upstream project license applies to these fixtures. `tests/compatibility/anchor-1.2.0/escrow-v1.json` and `escrow-v2.json` were generated with Anchor CLI 1.2.0 from this repository's Escrow programs. `tests/compatibility/SHA256SUMS` seals the fixture bytes. A release-family fixture qualifies the stated format/subset, not every possible IDL emitted by that compiler.

## Resource budgets

| Resource | Budget |
| --- | --- |
| Manifest / policy text | 4 MiB |
| IDL file | 8 MiB |
| IDL JSON nesting / nodes | 64 levels / 100,000 nodes |
| Defined-type validation nesting / traversal work | 64 levels / 100,000 visits |
| Program artifact | 64 MiB |
| Declared bindings / migration targets / fixture accounts | 10,000 each |
| Decoded account | 10 MiB |
| Decode depth / sequence length / decoded values | 64 / 100,000 / 100,000 |
| Synthetic fixture raw data total | 64 MiB |
| Devnet acquisition | One finalized batch, at most 100 configured accounts |
| Devnet aggregate raw bytes | 16 MiB |
| Frozen snapshot file | 128 MiB |
| Registry | 4 MiB / 10,000 entries |

Oversized or recursive inputs return a structured error. A fixture may observe up to 10,000 accounts, but scale execution is measured through 1,000 representative accounts in this release. The 10,000-record test qualifies generic stored raw snapshot serialization/loading/validation, not a 10,000-account RPC import or automatically generated batch-migration transaction. The migration program must implement the configured behavior; the engine does not infer a migration instruction or deployment safety from source code.

## Deliberate limitations

No API service, uploaded-source compilation workflow, drag-and-drop frontend, automatic account discovery, custom invariant language, token accounting/conservation plugin, mainnet scanning, exploit generation or automatic upgrade deployment is included. Graph validation covers declared edges and observed targets. Policy success is bounded by the fixture/snapshot/rules/scenarios supplied; it is not a general security certification. Hosted CI execution and independent security review are separate operational checks, not claimed by the local acceptance.
