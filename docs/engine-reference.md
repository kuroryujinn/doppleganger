# Engine reference

## Static analysis

`doppelganger analyze --from V1_IDL --to V2_IDL --account Old:New --format json` performs static comparison. It reports account fields/order/types/discriminators, enum variants, instruction arguments/account flags, PDA seeds, events and errors. This command is analysis evidence. The production `test` command applies policy to its findings and determines CI success.

## Fixtures

`[fixture]` names a local world and payer binding. `[[fixture.initialize]]` and `[[fixture.setup]]` execute V1 instructions in order. Each has an instruction, account_mapping and args. Instruction-created observations use `[[fixture.account]]` with name, type, address, owner and lamports; omit fields. Synthetic observations add `[fixture.account.fields]` containing every declared field. Program ownership/address checks, exact Borsh encoding, duplicates and unsupported types are validated. Raw bytes remain in evidence even when decoding fails. Synthetic state is deliberately labelled test construction and need not have been produced by an on-chain constructor.

## Migration configuration

`[migration]` selects one candidate instruction and explicit logical bindings. IDL account order, writable/signer flags, discriminator and argument encoding determine the actual transaction. Signatures use local seeded identities. `[[migration_targets]]` defines named observed accounts and V1/V2 account type pairings; the set may include accounts unchanged by the selected transaction. The engine swaps the same program ID locally, captures before/after data and execution logs/compute units, and preserves failed-transaction state. It does not automatically migrate every account in a directory/corpus: configure the instruction and targets for the behavior you intend to test.

Use decimal typed strings for unsigned/signed boundaries, e.g. `"u64:18446744073709551615"`. Prefixes must match the IDL type. Supported options, sequences, structs and enums are described in the example configurations and compatibility suite. Arbitrary Rust structs, generics and custom wire encodings are not inferred.

## Preservation rules

Each `[[preservation]]` contains an optional stable id, logical account observation name, rule, optional field path and optional value.

| Rule | Field | Value | Behavior |
| --- | --- | --- | --- |
| equal / not_equal | Required | Forbidden | Compare before and after typed field values |
| initialized | Required | Forbidden | Require field absent before and present after; use default for an exact value |
| default | Required | Required | After value equals the configured value |
| greater_or_equal / less_or_equal | Required | Required | Compare typed numeric after value with the configured bound |
| exists / not_exists | Optional | Forbidden | Require presence/absence of the account or field |
| owner_preserved | Forbidden | Forbidden | Keep program ownership |
| lamports_preserved | Forbidden | Forbidden | Keep account lamports |
| account_size_minimum | Forbidden | Required | Minimum after byte length |
| pda_preserved | Forbidden | Forbidden | Keep observed address continuity |

Nested paths use dot-separated struct fields. Missing/decode failures and unsupported comparisons produce errors; they do not become passes. Equal values use arbitrary-width integers internally, bounded to the declared on-wire primitive. Sum/conservation/custom expressions are later extensions.

## Lifecycle scenarios

`[[scenario]]` defines a stable id and type. Supported types are `standard_migration`, `unauthorized_migration`, `repeat_migration`, `failed_migration_rollback`, `post_migration_instruction`, `unmigrated_state_against_v2`, `zero_value_state` and `boundary_value_state`. A fresh fixture world is constructed per scenario. Outcomes are `success`/`error`; `expected_error` pins a program error number. `state_unchanged = true` verifies rollback. `signer_account` plus `signer_override` select the alternate identity for unauthorized tests. Instruction-based scenarios supply an instruction name plus account_mapping/args directly on the scenario; migration overrides use a nested migration definition. Alternate fixtures are named in the manifest's `fixtures` table. Vault and Escrow provide complete working recipes; neither is selected by project name inside the runtime.

## Relationship rules

Declared graph edges live in `doppelganger-policy.toml`: authority, owner, reference, parent, mint, program_owner and pda. The [production workflow](production-workflow.md#account-relationships) documents exact fields and examples. Field relationships require Pubkey values; reference/parent/mint edges require observed target existence. PDA edges require a configured PDA binding. Missing, ambiguous or undecodable required state produces error evidence. Graph coverage is configured, not discovered automatically.

## Snapshots and Devnet import

`doppelganger snapshot --help` documents import flags. Configure developer-controlled Devnet accounts and program identities under `[snapshot]`. Acquisition records raw bytes, owner, lamports, executable flag, finalized slot, genesis identity and hashes. It uses read-only genesis/account RPC calls. The importer accepts configured accounts only; discovery and mainnet scanning are out of scope.

`doppelganger test PROJECT --snapshot SNAPSHOT_DIRECTORY --ci` restores frozen state and executes locally without RPC. Imported state cannot be silently replaced by fixture setup. Snapshot versions, content hashes, source program/IDL identities and local signer consistency are checked. The live acceptance documents the captured real Devnet case; routine tests replay its frozen bytes offline. Never edit hashed snapshots to make them pass.

## Regression cases

`regression create --from-report FILE --manifest PROJECT --finding PRESERVE-ID --store DIR` captures one native preservation finding. Use `--scenario ID` for lifecycle findings. `regression reproduce CASE --candidate BINARY --candidate-idl IDL` can verify candidate fixes while preserving the recorded base state. Native cases pin referenced source artifacts and require those paths; snapshot-backed native cases retain imported state.

`regression capture --from-report FILE --store NEW_DIRECTORY` creates a portable complete-workflow case, including static/policy/relationship failures. `regression replay DIRECTORY --output NEW_DIRECTORY` validates case identity, exact file inventory, every input hash and normalized state/findings. Copies of binaries/IDLs/manifests/policies and optional snapshots make this case independent of source directories. Replay success means the recorded failure was reproduced, not that its migration is safe. Full-workflow capture requires a complete failed production report; argument/configuration failures without tested artifact identities cannot be captured.

## CI integration and policy

`test --ci` always uses exits 0–4 and saves JSON, HTML and Markdown. Exact expected static changes, warnings, stale approvals and required coverage are described in [production workflow](production-workflow.md#expected-changes-and-required-coverage). Runtime violations cannot be hidden by approving a static change. CI on a provisioned Linux runner builds both versions, performs local qualification, audits dependencies and retains reports. The reusable GitHub workflow is prepared with trusted same-repository PR, main/master push and manual triggers; forks are excluded from the self-hosted runner. local execution is verified, while no hosted GitHub run is claimed.

## Report schema

`report.json` has schema_version 1, engine_version, workflow_version, run_id, project, result, exit_code, analysis, execution, preservation, relationships, lifecycle, policy, artifacts and source. Manifest version/legacy status is explicit. `artifacts` contains canonical paths and SHA256 for the manifest, policy, binaries, IDLs and optional snapshot. The run ID derives from content hashes and workflow version. Before/after accounts include raw snapshots, optional decoded state and decode errors. Execution retains transaction outcome, logs and compute units. Each check has PASS, FAIL, ERROR or NOT_APPLICABLE status; required unavailable evidence blocks. Policy reports expected, warning, ignored, blocking and evaluation-error counts without a global safety score.

Snapshot runs also expose a top-level snapshot envelope for regression compatibility. Reports state that migration/preservation/lifecycle/policy were evaluated locally and no remote transactions or RPC writes occurred. Error reports use structured category/code/context/message/exit_code/evidence and suggested_correction. Error reports need not contain execution artifacts if input validation failed. HTML escapes user-supplied evidence. `--json` prints full evidence, `--quiet` suppresses successful stdout while retaining files and error location, and `--verbose` writes workflow context to stderr. These modes describe production `test`; legacy evidence commands retain their original interfaces.
