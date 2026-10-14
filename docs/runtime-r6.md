# R6: Generic Migration Execution

R6 executes configured V1-to-V2 migrations and captures before/after evidence.
It does not yet evaluate preservation correctness.

## API and Boundaries

`dg_runtime::MigrationExecutor::execute` accepts an existing R5 `RuntimeWorld`
and a `MigrationRequest`. `execute_manifest` supplies the same request from the
manifest and candidate IDL. Neither rebuilds accounts, resolves new identities,
nor repeats V1 initialization/setup.

`load_candidate` validates the same program ID, reads the ELF, and stages its
installation in a clone of the existing LiteSVM. The clone contains the original
state and committed transaction history. Only a successful load replaces the
live VM. This method is separately tested before any migration transaction.
The world's normalized model is retained from construction, so BEFORE decoding
does not re-read a potentially edited V1 IDL.

Migration encoding uses `MigrationDefinition::resolve`, `InstructionBinding::build`,
and the existing recursive argument/address resolver. The transaction path is
shared with R5. It signs with the required identities and fee payer only.
Exactly one migration instruction is submitted in one transaction.

## Observations

Optional top-level targets provide explicit version-specific schema names:

```toml
[[migration_targets]]
name = "vault"
address = "$vault"
v1_type = "VaultV1"
v2_type = "VaultV2"
```

An explicit list replaces the default observation list. When omitted, fixture
accounts are observed using their existing type name on both sides. There is no
account-name inference or Vault-specific mapping. Omit either version's type
for raw-only observation. Addresses may be literals or existing `$bindings`.
Migration-created accounts should be listed here, not as required R5 observations.

Each `AccountEvidence` wraps the existing `RawAccountSnapshot`, with separately
optional `DecodedAccount` and `StateError`. A null snapshot means absent. A
present but undecodable account retains its raw bytes and metadata. BEFORE uses
the V1 model and AFTER the candidate model, including after program failure.
Snapshots own their bytes and do not alias VM memory.

## Results and Errors

`MigrationExecution` includes schema version, project name, both artifact paths
and IDs, `ProgramSwapRecord`, before/after observations, `ExecutionRecord`, and
`preservation_evaluated: false`. Account and signer order is deterministic for
the same input world. Explicit local `test_seed` identities make independent
runs reproducible; default generated identities intentionally differ.

`ExecutionRecord` includes ordered account metas, discriminator plus argument
bytes, public signer keys, signature, outcome, logs, fee, compute units, and the
SDK's structured `transaction_error`. No keypair or seed is serialized.

A program `InstructionError` is returned as successful evidence collection with
`transaction.success: false`, not as an executor error. `MigrationError` separates
artifact, IDL, binding, argument, signer, submission, internal runtime and snapshot failures. It
retains partial execution evidence once capture has begun. Snapshot limits can
fail after a transaction has committed; consumers must check the attached record
and must not assume executor failure implies transaction rollback.

Failed migration transactions are atomic for attempted account changes. Earlier
R5 setup is already committed and stays committed. Transaction fees can still
affect the payer. Candidate code stays installed after a failed transaction.

## CLI

```bash
cargo run -p dg-cli -- runtime migrate examples/vault-runtime \
  --output report/migration-execution.json
```

The CLI builds V1 through R5 and calls R6 once. Program errors are reported as
execution evidence with a zero command exit status; configuration/executor errors
return nonzero. This is not a safety gate. R6 errors write structured partial
evidence to the requested output. The final execution summary explicitly says
`Preservation evaluation: not executed.`

## Verification

Dedicated SBF fixtures `dg-r6-fixture-v1` and `dg-r6-fixture-v2` use a record with
authority and value; V2 adds a version byte. They include migration, creation,
closure, argument decoding, early rejection and deliberate mutation-then-failure.
The latter performs realloc, a raw byte write and a SOL transfer before error.

`runtime_r6` contains 36 isolated tests covering candidate loading and load-time
state continuity; instruction order/discriminator; mapping and signer errors;
aliases and separate fee payer; u64/u128/i128 boundaries and nested struct, enum,
Vec, Option, array and tuple arguments against independent Anchor encoding;
success/realloc; transaction rollback; creation/closure; decoding and immutable
snapshots; deterministic full reports; structured runtime errors and limits.

`vault_r6_acceptance` contains three separate acceptance tests against an independent
typed version of the original demo recipe: good Vault, faulty Vault, and rejected
authority. They compare raw state, decoded fields and program error behavior.
Faulty Vault evidence is collected without preservation findings or a safety verdict.

Build the original Vault artifacts/IDLs with the existing demo build instructions
first. Tests fail rather than skip when required artifacts are missing. Then:

```bash
bash scripts/check-r6.sh
```

For an installed offline SBF toolchain, append `--offline --skip-tools-install`
to the build commands in that script. Host Cargo commands accept `--offline`.

## Limitations

- Local LiteSVM only: replacing executable code does not test loader authority,
  deployment transactions, cluster feature parity, or Devnet.
- One instruction and one migration transaction; no lifecycle/scenario framework.
- Artifacts and IDLs are supplied locally; R6 does not attest that an IDL matches
  the binary. Programs can run but decode incorrectly; raw evidence remains available.
- Standard explicit Anchor/Borsh schema support comes from the existing decoder.
  Unknown/unsupported layouts produce decode errors, not invented fields.
- No R7 preservation rules, economic judgment, safety verdict, relationship graph,
  second-project proof, regression generation, CI, or final integrated runtime suite.
