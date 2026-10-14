# Generic State Interface, Version 1

This component materializes and decodes state without LiteSVM, transactions, or
project-specific account types. It does not determine whether an upgrade is safe.

The `raw` module additionally exposes `RawAccountSnapshot` and `SnapshotSet` for
arbitrary bytes, including accounts that fail decoding. Capture does not require
an IDL. Decode only after checking the observed address and owner against the
binding. Snapshot sets detect missing accounts and wrong owners; `restore_into`
validates all records before atomically replacing an in-memory set. This is not
VM restoration: programs, sysvars, and transaction history are not restored.

## Public Boundary

- `decode_account`: normalized `ProgramModel`, account name, raw bytes, limits,
  and explicit trailing-byte policy -> `DecodedAccount` or `StateError`.
- `AccountSnapshot::capture`: account binding, lamports, executable flag, and
  raw bytes -> versioned snapshot containing both raw and decoded state.
- `AccountSnapshot::validate`: recheck a deserialized snapshot against its IDL,
  byte length, addresses, lamport range, and schema version. Deserialization alone
  is not validation; consumers must call this before trusting imported state.
- `load_fixture`: declarative fixture, normalized IDL, resolved address bindings,
  and limits -> `RuntimeWorld`. This writes no files and executes no instructions.
- `RuntimeWorld`: ordered account snapshots and program artifact references.
  `ProgramArtifact` records paths and program ID, not artifact integrity hashes.

`Value` distinguishes booleans, integers, public keys, strings, sequences,
options, structs, and enums. Integers are `BigInt`; JSON stores decimal strings,
never floating-point values. Struct fields use ordered maps. Tuple enum fields
use positional keys (`"0"`, `"1"`). Arrays, vectors, and tuples are sequences;
their schema distinctions remain in the IDL. Missing option and missing field
are different states. Errors contain `code`, `path`, byte `offset`, and `message`.

## Supported Layouts

Borsh bool, u8/u16/u32/u64/u128, i8/i16/i32/i64/i128, pubkey/publicKey,
string, bytes, Vec, Option, fixed arrays, tuples, structs, aliases, and unit,
tuple-payload, or named-payload enums. Nested custom types are supported.
The decoder requires explicit account discriminators from the normalized model.
It does not derive legacy discriminators or support zero-copy/bytemuck layouts,
floats, 256-bit wire integers, generic type parameters, or custom serialization.
Callers must supply Borsh schemas, not zero-copy layouts normalized as structs.
Real generated-IDL compatibility qualification remains a separate roadmap item.

`TrailingBytes::Reject` fails on any unconsumed bytes. `Preserve` exposes all
remaining bytes, including nonzero bytes, for overallocated accounts; this does
not certify them as harmless padding. Raw bytes are always retained in snapshots.

Default decoder limits: 10 MiB per account, nesting depth 64, 100,000 sequence
elements, and 100,000 decoded values. Callers may set smaller limits. These are
decoder limits, not a bounded JSON/TOML import service or whole-world memory cap.
Untrusted file-size limits and multi-account scale qualification are still pending.

## Declarative Fixtures

In a project manifest:

```toml
[fixture]
name = "positions"

[[fixture.account]]
name = "alice_position"
type = "Position"
address = "$position"
owner = "$program"
lamports = "u64:1000000"

[fixture.account.fields]
owner = "$alice"
balance = "u128:340282366920938463463374607431768211455"
```

Supply `position`, `program`, and `alice` in the binding map. Addresses may also
be literal base58 keys. `$name` strings inside fields recursively resolve to a
binding (that prefix is reserved). `dg-config::ResolvedAddresses` now resolves
signers, aliases, program IDs and PDAs before fixture loading; there is no
`derive:...` expression parser. See runtime foundations.
Executable defaults to false; owner and lamports are explicit. All schema fields
are required when `fields` is supplied; unknown fields and duplicate names/addresses
are rejected. Omit `fields` for an instruction-created account. These accounts
and `initialize`/`setup` actions require `dg-runtime`'s VM world builder; `load_fixture`
rejects them explicitly rather than silently skipping execution.

Options use `{ none = true }` or `{ some = value }`. Structs use tables; arrays,
vectors, and tuples use arrays. Enums use a unit variant string or
`{ variant = "Filled", fields = { amount = "u64:42" } }`; tuple payloads use arrays.
Integer strings may be bare decimal or prefixed with the exact IDL integer type.
Encoding enforces the wire type's range before producing bytes.

Fixture construction shares the existing config encoder; it does not load raw
snapshot files or run program initialization. Synthetic fixture validity beyond
serialization belongs to later runtime and relationship validation phases.
The migration CLI deliberately rejects these account lists until its generic
migration executor is implemented. `migration inspect` can validate the project
binding. `runtime build` now executes V1 setup via `dg-runtime`, without migration.

## Acceptance Gate

Run `cargo test -p dg-state` for isolated decoder, fixture, and raw snapshot tests:

- Every integer width, signed boundaries, full u64/u128 values, typed config overflow.
- Bool/public keys, UTF-8 strings, vectors/bytes, options, arrays, tuples.
- Nested structs/aliases and all three enum payload forms.
- Discriminator mismatch, every truncation of a composite value, trailing bytes.
- Invalid tags, UTF-8, unknown types, malformed enums, recursive/oversized input.
- Fixture binding/duplicate validation and independent Borsh serialization parity.
- Raw-plus-decoded snapshot JSON round-trip, deterministic serialization, tamper checks.

No preservation engine, generic VM executor, devnet replay, or production-readiness
claim is part of this gate. Preserve this data-only boundary for the next phase.
