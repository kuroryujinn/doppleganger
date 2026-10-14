# Supported Anchor and Borsh scope

CLI 0.1.0 is qualified for the documented prepared Anchor/Borsh subset on Linux x86_64/WSL2. A compiler family fixture does not establish every format that compiler can emit.

## Compatibility

| Input | Static normalization | Account decoding | Executed SBF evidence |
| --- | --- | --- | --- |
| Anchor 1.2.0 explicit-discriminator Borsh IDLs | Qualified | Qualified | Good/broken Vault and Escrow; Record fixture |
| Anchor 0.30.1 explicit-discriminator IDL | Pinned upstream fixture | Known u32 wire bytes | No separately compiled 0.30.1 execution claimed |
| Legacy inline account IDL | Supported static shapes | Unsupported without explicit discriminator | Convert/rebuild before runtime use |
| Zero-copy, bytemuck, custom representation | Rejected | Unsupported | Unsupported |
| Generic defined types/custom serializers | Rejected | Unsupported | Unsupported |

Rust/Cargo 1.98.0, LiteSVM 0.16.0, Anchor CLI/lang 1.2.0, Solana CLI 3.1.10 and platform-tools v1.57 were used for release qualification. Prepared execution does not require the program-building tools.

## Supported wire shapes

Runtime encoding/decoding supports bool, u8–u128, i8–i128, public keys, strings, bytes, structs, enum unit/tuple/named payloads, nested defined types, vectors, options, arrays and tuples within budgets. Floats, 256-bit integers, generics and custom encodings are not supported by runtime encoding/decoding, even where static analysis can describe a primitive.

Integers use arbitrary precision internally and decimal-string JSON. Raw account bytes are retained when decoding fails. Trailing bytes are rejected or explicitly preserved according to the call's mode.

## Resource budgets

| Input/resource | Limit |
| --- | --- |
| Manifest / policy | 4 MiB each |
| IDL | 8 MiB |
| IDL nesting / nodes | 64 / 100,000 |
| Defined-type validation depth / work | 64 / 100,000 visits |
| Program binary | 64 MiB |
| Bindings, targets, fixture accounts | 10,000 each |
| Decoded account | 10 MiB |
| Decode depth / sequence / values | 64 / 100,000 / 100,000 |
| Synthetic raw fixture total | 64 MiB |
| Devnet account batch | 100 configured accounts |
| Aggregate Devnet raw bytes | 16 MiB |
| Frozen snapshot file | 128 MiB |
| Registry text / entries | 4 MiB / 10,000 |

The release measured execution through 1,000 representative observed accounts. A 10,000-record serialization test is not a claim of automatic batch migration or 10,000-account RPC capture.

## Scope boundary

No universal safety certification, automatic migration inference, mainnet discovery, token-conservation pack or custom invariant language is implemented. The prepared-project API exists separately from local execution; uploaded-source builds and the migration-job frontend are unfinished. [Known limitations](/docs/limitations) records those distinctions.
