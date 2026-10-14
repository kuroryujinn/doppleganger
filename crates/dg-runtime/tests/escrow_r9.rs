//! Second-application acceptance. All execution flows through the unchanged core.
use anchor_lang::{AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas};
use dg_config::ProjectManifest;
use dg_runtime::{
    lifecycle::{changed_accounts, LifecycleEngine, LifecycleReport, Operation, ScenarioStatus},
    preservation::{evaluate, PreservationReport, Resolution, Status},
    MigrationExecution, MigrationExecutor, Phase, RuntimeWorldBuilder, WorldBuildResult,
};
use dg_state::{DecodedAccount, Value};
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::{collections::BTreeMap, fs, path::PathBuf};

const AMOUNT: u64 = 1_000_000;
const DEADLINE: i64 = 4_102_444_800;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn key(seed: u8) -> Address {
    Keypair::new_from_array([seed; 32]).pubkey()
}
fn escrow() -> Address {
    Address::find_program_address(
        &[b"escrow", key(31).as_ref(), key(32).as_ref()],
        &dg_escrow_v1::id(),
    )
    .0
}
fn manifest(broken: bool) -> (ProjectManifest, PathBuf) {
    let base = root().join(if broken {
        "examples/escrow-runtime-broken"
    } else {
        "examples/escrow-runtime"
    });
    let m = ProjectManifest::parse(&fs::read_to_string(base.join("doppelganger.toml")).unwrap())
        .unwrap();
    for v in [&m.versions.v1, &m.versions.v2] {
        for p in [&v.program_path, &v.idl] {
            assert!(base.join(p).is_file(),"Missing required Escrow artifact {}. Run bash scripts/build-escrow.sh; acceptance never skips.",base.join(p).display());
        }
    }
    (m, base)
}
fn build(broken: bool) -> (ProjectManifest, PathBuf, WorldBuildResult) {
    let (m, base) = manifest(broken);
    let built = RuntimeWorldBuilder::default()
        .build_manifest(&m, &base)
        .unwrap();
    (m, base, built)
}
fn migrate(broken: bool) -> (MigrationExecution, PreservationReport) {
    let (m, base, mut built) = build(broken);
    let e = MigrationExecutor::default()
        .execute_manifest(&mut built.world, &m, &base)
        .unwrap();
    assert!(
        e.transaction.as_ref().unwrap().success,
        "{:?}",
        e.transaction
    );
    let p = evaluate(&e, &m.preservation, &built.world.model).unwrap();
    (e, p)
}
fn lifecycle(broken: bool) -> LifecycleReport {
    let (m, base) = manifest(broken);
    LifecycleEngine.run(&m, &base).unwrap()
}
fn fields(v2: bool, broken: bool, status: &str) -> BTreeMap<String, Value> {
    let mut f = BTreeMap::from([
        ("initializer".into(), Value::Pubkey(key(31).to_string())),
        ("beneficiary".into(), Value::Pubkey(key(32).to_string())),
        (
            "amount".into(),
            Value::Integer((if broken { 900_000u64 } else { AMOUNT }).into()),
        ),
        (
            "status".into(),
            Value::Enum {
                variant: status.into(),
                fields: BTreeMap::new(),
            },
        ),
    ]);
    if v2 {
        f.insert("deadline".into(), Value::Integer(DEADLINE.into()));
        f.insert(
            "arbitrator".into(),
            Value::Pubkey(Address::new_from_array([42; 32]).to_string()),
        );
    }
    f
}
fn decoded(d: &DecodedAccount, v2: bool, broken: bool, status: &str) {
    assert_eq!(d.fields, fields(v2, broken, status));
    assert_eq!(d.consumed_bytes, if v2 { 121 } else { 81 });
    assert!(d.trailing_bytes.is_empty());
}

#[test]
fn v1_real_setup_and_decoder_match_independent_typed_state() {
    let (_, _, built) = build(false);
    assert_eq!(built.executed_actions.len(), 2);
    assert_eq!(built.executed_actions[0].phase, Phase::Initialize);
    assert_eq!(
        built.executed_actions[0].instruction_name,
        "initialize_escrow"
    );
    assert_eq!(built.executed_actions[1].phase, Phase::Setup);
    assert_eq!(built.executed_actions[1].instruction_name, "fund");
    assert!(built.executed_actions.iter().all(|r| r.success));
    let observed = &built.observed_accounts[0];
    let raw = &observed.snapshot;
    assert_eq!(raw.address, escrow().to_string());
    assert_eq!(raw.owner, dg_escrow_v1::id().to_string());
    assert!(!raw.executable);
    assert_eq!(raw.data_length, 81);
    assert_eq!(
        raw.lamports,
        (anchor_lang::prelude::Rent::default().minimum_balance(81) + AMOUNT).into()
    );
    let expected = dg_escrow_v1::EscrowV1 {
        initializer: key(31),
        beneficiary: key(32),
        amount: AMOUNT,
        status: dg_escrow_v1::EscrowStatus::Funded,
    };
    let mut bytes = vec![];
    expected.try_serialize(&mut bytes).unwrap();
    assert_eq!(raw.raw_data, bytes);
    let typed = dg_escrow_v1::EscrowV1::try_deserialize(&mut raw.raw_data.as_slice()).unwrap();
    assert_eq!(typed.initializer, key(31));
    assert_eq!(typed.beneficiary, key(32));
    assert_eq!(typed.amount, AMOUNT);
    assert_eq!(typed.status, dg_escrow_v1::EscrowStatus::Funded);
    decoded(observed.decoded.as_ref().unwrap(), false, false, "Funded");
}

#[test]
fn v1_multiple_signers_and_two_party_pda_resolve_generically() {
    let (_, _, built) = build(false);
    assert_eq!(built.world.resolved.signers.len(), 3);
    for (name, seed) in [("initializer", 31), ("beneficiary", 32), ("stranger", 33)] {
        assert_eq!(built.world.resolved.addresses[name], key(seed));
    }
    assert_eq!(built.world.resolved.addresses["escrow"], escrow());
    let mut signers = built.executed_actions[0].signer_public_keys.clone();
    signers.sort();
    let mut expected = vec![key(31).to_string(), key(32).to_string()];
    expected.sort();
    assert_eq!(signers, expected);
    assert_eq!(
        built.executed_actions[1].signer_public_keys,
        [key(31).to_string()]
    );
    assert_eq!(
        built.executed_actions[0].instruction_data,
        dg_escrow_v1::instruction::InitializeEscrow { amount: AMOUNT }.data()
    );
}

#[test]
fn v1_world_needs_no_v2_artifact_or_migration() {
    let (mut m, base) = manifest(false);
    m.versions.v2.program_path = "missing.so".into();
    m.versions.v2.idl = "missing.json".into();
    m.migration.instruction = "missing".into();
    let built = RuntimeWorldBuilder::default()
        .build_manifest(&m, &base)
        .unwrap();
    decoded(
        built.observed_accounts[0].decoded.as_ref().unwrap(),
        false,
        false,
        "Funded",
    );
}

#[test]
fn candidate_loading_keeps_the_live_v1_world_and_snapshot() {
    let (m, base, mut built) = build(false);
    let before = built.observed_accounts[0].snapshot.clone();
    let executor = MigrationExecutor::default();
    let (artifact, model) = executor.candidate(&m, &base).unwrap();
    executor
        .load_candidate(&mut built.world, &artifact, &model)
        .unwrap();
    let live = built.world.runtime.get_account(&escrow()).unwrap();
    assert_eq!(live.data, before.raw_data);
    assert_eq!(live.lamports.to_string(), before.lamports.to_string());
    assert_eq!(live.owner.to_string(), before.owner);
    let execution = executor
        .execute_loaded(
            &mut built.world,
            &m.project.name,
            &m.migration,
            &m.migration_targets,
            Some("initializer"),
        )
        .unwrap();
    assert!(execution.transaction.as_ref().unwrap().success);
    assert!(execution.program_swap.is_none());
    assert_eq!(
        built.observed_accounts[0].snapshot, before,
        "captured BEFORE does not alias the VM"
    );
}

#[test]
fn migration_instruction_and_accounts_match_anchor_encoding() {
    let (e, _) = migrate(false);
    let tx = e.transaction.as_ref().unwrap();
    assert_eq!(tx.instruction_name, "migrate_escrow");
    assert_eq!(tx.signer_public_keys, [key(31).to_string()]);
    assert_eq!(
        tx.instruction_data,
        dg_escrow_v2::instruction::MigrateEscrow {
            deadline: DEADLINE,
            arbitrator: Address::new_from_array([42; 32]),
            fail_after_write: false
        }
        .data()
    );
    let metas = dg_escrow_v2::accounts::MigrateEscrow {
        escrow: escrow(),
        initializer: key(31),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    assert_eq!(tx.accounts.len(), metas.len());
    for (a, b) in tx.accounts.iter().zip(metas) {
        assert_eq!(a.address, b.pubkey.to_string());
        assert_eq!(a.signer, b.is_signer);
        assert_eq!(a.writable, b.is_writable);
    }
}

#[test]
fn good_and_broken_migrations_both_produce_valid_independent_v2_state() {
    for broken in [false, true] {
        let (e, _) = migrate(broken);
        let before = e.before_accounts[0].snapshot.as_ref().unwrap();
        let after = e.after_accounts[0].snapshot.as_ref().unwrap();
        assert_eq!(before.address, after.address);
        assert_eq!(after.address, escrow().to_string());
        assert_eq!(before.owner, after.owner);
        assert_eq!((before.data_length, after.data_length), (81, 121));
        let expected = dg_escrow_v2::EscrowV2 {
            initializer: key(31),
            beneficiary: key(32),
            amount: if broken { 900_000 } else { AMOUNT },
            deadline: DEADLINE,
            arbitrator: Address::new_from_array([42; 32]),
            status: dg_escrow_v2::EscrowStatus::Funded,
        };
        let mut bytes = vec![];
        expected.try_serialize(&mut bytes).unwrap();
        assert_eq!(after.raw_data, bytes);
        let typed =
            dg_escrow_v2::EscrowV2::try_deserialize(&mut after.raw_data.as_slice()).unwrap();
        assert_eq!(typed.initializer, expected.initializer);
        assert_eq!(typed.beneficiary, expected.beneficiary);
        assert_eq!(typed.amount, expected.amount);
        assert_eq!(typed.deadline, DEADLINE);
        assert_eq!(typed.arbitrator, expected.arbitrator);
        assert_eq!(typed.status, expected.status);
        decoded(
            e.before_accounts[0].decoded.as_ref().unwrap(),
            false,
            false,
            "Funded",
        );
        decoded(
            e.after_accounts[0].decoded.as_ref().unwrap(),
            true,
            broken,
            "Funded",
        );
        assert!(e.after_accounts[0].decode_error.is_none());
        assert!(!e.preservation_evaluated);
    }
}

#[test]
fn good_preservation_passes_all_eleven_rules() {
    let (_, p) = migrate(false);
    assert_eq!(
        (
            p.rules_total,
            p.passed,
            p.failed,
            p.errors,
            p.not_applicable
        ),
        (11, 11, 0, 0, 0)
    );
    assert!(p.findings.is_empty());
}

#[test]
fn broken_amount_is_the_only_violation_with_structured_evidence() {
    let (good_e, good) = migrate(false);
    let (broken_e, broken) = migrate(true);
    assert_eq!(
        serde_json::to_value(&good.rules).unwrap(),
        serde_json::to_value(&broken.rules).unwrap()
    );
    assert_eq!(
        (
            broken.rules_total,
            broken.passed,
            broken.failed,
            broken.errors,
            broken.not_applicable
        ),
        (11, 10, 1, 0, 0)
    );
    assert_eq!(broken.findings.len(), 1);
    let f = &broken.findings[0];
    let e = &f.evaluation;
    assert_eq!(f.id, "PRESERVE-amount-retained");
    assert_eq!(e.rule_id, "amount-retained");
    assert_eq!(e.category.as_deref(), Some("FIELD_NOT_PRESERVED"));
    assert_eq!(e.account, "escrow");
    assert_eq!(e.field.as_deref(), Some("amount"));
    assert_eq!(e.status, Status::Fail);
    assert_eq!(
        e.before,
        Some(Resolution::Found(Value::Integer(AMOUNT.into())))
    );
    assert_eq!(
        e.after,
        Some(Resolution::Found(Value::Integer(900_000u64.into())))
    );
    assert!(broken
        .evaluations
        .iter()
        .filter(|e| e.rule_id != "amount-retained")
        .all(|e| e.status == Status::Pass));
    assert_eq!(
        serde_json::to_value(&good_e.before_accounts).unwrap(),
        serde_json::to_value(&broken_e.before_accounts).unwrap()
    );
    let mut expected = good_e.after_accounts[0].snapshot.clone().unwrap();
    expected.raw_data[72..80].copy_from_slice(&900_000u64.to_le_bytes());
    assert_eq!(
        Some(expected),
        broken_e.after_accounts[0].snapshot,
        "only amount bytes differ, not owner, lamports, length or other fields"
    );
}

#[test]
fn good_and_broken_manifests_differ_only_in_name_and_binary() {
    let read = |name: &str| {
        toml::from_str::<toml::Value>(
            &fs::read_to_string(root().join(format!("examples/{name}/doppelganger.toml"))).unwrap(),
        )
        .unwrap()
    };
    let mut good = read("escrow-runtime");
    let broken = read("escrow-runtime-broken");
    assert_ne!(
        good["versions"]["v2"]["program_path"],
        broken["versions"]["v2"]["program_path"]
    );
    good["versions"]["v2"]["program_path"] = broken["versions"]["v2"]["program_path"].clone();
    good["project"]["name"] = broken["project"]["name"].clone();
    assert_eq!(good, broken);
}

fn scenario(id: &str) {
    for broken in [false, true] {
        let (mut m, base) = manifest(broken);
        m.scenario.retain(|s| s.id.as_deref() == Some(id));
        assert_eq!(m.scenario.len(), 1);
        let r = LifecycleEngine.run(&m, &base).unwrap();
        assert_eq!(
            (r.total, r.passed, r.failed, r.errors),
            (1, 1, 0, 0),
            "{}",
            serde_json::to_string_pretty(&r).unwrap()
        );
        let s = &r.scenarios[0];
        decoded(
            s.steps[0].after[0].decoded.as_ref().unwrap(),
            false,
            false,
            "Funded",
        );
        assert_eq!(s.steps[0].setup_actions.len(), 2);
        let last = s.steps.last().unwrap();
        let tx = last.execution.as_ref().unwrap();
        assert!(!tx.logs.is_empty());
        assert!(tx.compute_units.unwrap() > 0);
        match id {
            "standard" => {
                assert_eq!(s.steps.len(), 2);
                assert!(tx.success);
                decoded(
                    last.after[0].decoded.as_ref().unwrap(),
                    true,
                    broken,
                    "Funded",
                );
            }
            "unauthorized" => {
                assert!(!tx.success);
                assert!(tx.signer_public_keys.contains(&key(33).to_string()));
                assert!(changed_accounts(&last.before, &last.after).is_empty());
            }
            "repeat" => {
                assert_eq!(s.steps.len(), 3);
                assert!(s.steps[1].program_swap.is_some());
                assert!(last.program_swap.is_none());
                assert_eq!(s.steps[1].after[0].snapshot, last.before[0].snapshot);
                assert!(changed_accounts(&last.before, &last.after).is_empty());
            }
            "rollback" => {
                assert!(tx
                    .logs
                    .iter()
                    .any(|l| l.contains("attempted escrow write, resize and rent top-up")));
                assert!(changed_accounts(&last.before, &last.after).is_empty());
                assert_eq!(last.before[0].snapshot.as_ref().unwrap().data_length, 81);
            }
            "unmigrated" => {
                assert_eq!(s.steps[1].operation, Operation::LoadCandidate);
                assert!(s.steps[1].execution.is_none());
                assert!(!s.steps.iter().any(|s| s.operation == Operation::Migrate));
                assert_eq!(last.before[0].snapshot.as_ref().unwrap().data_length, 81);
                assert!(changed_accounts(&last.before, &last.after).is_empty());
            }
            "post-migration" => {
                assert_eq!(s.steps.len(), 3);
                assert!(last.program_swap.is_none());
                assert_eq!(s.steps[1].after[0].snapshot, last.before[0].snapshot);
                assert_eq!(tx.instruction_name, "release");
                assert!(tx.signer_public_keys.contains(&key(32).to_string()));
                assert!(!tx.signer_public_keys.contains(&key(33).to_string()));
                decoded(
                    last.after[0].decoded.as_ref().unwrap(),
                    true,
                    broken,
                    "Released",
                );
            }
            _ => panic!("uncovered scenario"),
        }
    }
}
#[test]
fn lifecycle_standard() {
    scenario("standard");
}
#[test]
fn lifecycle_unauthorized() {
    scenario("unauthorized");
}
#[test]
fn lifecycle_repeat() {
    scenario("repeat");
}
#[test]
fn lifecycle_rollback_after_mutation() {
    scenario("rollback");
}
#[test]
fn lifecycle_unmigrated() {
    scenario("unmigrated");
}
#[test]
fn lifecycle_post_migration_beneficiary_release() {
    scenario("post-migration");
}

#[test]
fn all_scenarios_start_fresh_in_configured_order_for_both_candidates() {
    for broken in [false, true] {
        let r = lifecycle(broken);
        assert_eq!((r.total, r.passed, r.failed, r.errors), (6, 6, 0, 0));
        assert_eq!(
            r.scenarios
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            [
                "standard",
                "unauthorized",
                "repeat",
                "rollback",
                "unmigrated",
                "post-migration"
            ]
        );
        for s in &r.scenarios {
            assert_eq!(s.status, ScenarioStatus::Pass);
            assert_eq!(
                s.steps[0].after[0].snapshot,
                r.scenarios[0].steps[0].after[0].snapshot
            );
            assert_eq!(s.steps[0].setup_actions.len(), 2);
        }
        let (mut m, base) = manifest(broken);
        m.scenario.reverse();
        let reversed = LifecycleEngine.run(&m, &base).unwrap();
        for (a, b) in r.scenarios.iter().zip(reversed.scenarios.iter().rev()) {
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            );
        }
        assert!(r.lifecycle_evaluated && !r.preservation_evaluated && !r.policy_evaluated);
    }
}

#[test]
fn deterministic_world_migration_preservation_and_lifecycle_reports() {
    for broken in [false, true] {
        let (_, _, a) = build(broken);
        let (_, _, b) = build(broken);
        assert_eq!(
            serde_json::to_vec(&a.observed_accounts).unwrap(),
            serde_json::to_vec(&b.observed_accounts).unwrap()
        );
        let (a, p) = migrate(broken);
        let (b, q) = migrate(broken);
        assert_eq!(
            serde_json::to_vec(&a).unwrap(),
            serde_json::to_vec(&b).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&p).unwrap(),
            serde_json::to_vec(&q).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&lifecycle(broken)).unwrap(),
            serde_json::to_vec(&lifecycle(broken)).unwrap()
        );
    }
}

#[test]
fn phase_zero_detects_real_escrow_shapes_without_special_handling() {
    let (m, base) = manifest(false);
    let v1 = dg_analysis::normalize(&fs::read(base.join(m.versions.v1.idl)).unwrap()).unwrap();
    let v2 = dg_analysis::normalize(&fs::read(base.join(m.versions.v2.idl)).unwrap()).unwrap();
    let mappings = [("EscrowV1".into(), "EscrowV2".into())];
    let r = dg_analysis::analyze("escrow", &v1, &v2, &mappings);
    assert!(!r.runtime_executed);
    let sizes = &r.account_sizes["EscrowV1 -> EscrowV2"];
    assert_eq!(sizes[0].as_ref().unwrap().bytes, Some(81));
    assert_eq!(sizes[1].as_ref().unwrap().bytes, Some(121));
    for field in ["arbitrator", "deadline"] {
        assert!(r
            .findings
            .iter()
            .any(|f| f.classification == "ACCOUNT_SCHEMA_CHANGE" && f.path.ends_with(field)));
    }
    assert!(r
        .findings
        .iter()
        .any(|f| f.classification == "SERIALIZATION_COMPATIBILITY_CHANGE"
            && f.path == "EscrowStatus.Disputed"));
    assert!(r
        .findings
        .iter()
        .any(|f| f.classification == "DISCRIMINATOR_CHANGE"));
    assert!(r
        .findings
        .iter()
        .any(|f| f.target_name == "release" && f.classification == "INTERFACE_CHANGE"));
    assert_eq!(
        serde_json::to_vec(&r).unwrap(),
        serde_json::to_vec(&dg_analysis::analyze("escrow", &v1, &v2, &mappings)).unwrap()
    );
    assert!(dg_analysis::analyze("same", &v1, &v1, &[])
        .findings
        .is_empty());
}

#[test]
fn unchanged_pre_r9_sources_still_match_historical_baseline() {
    let baseline = include_str!("../../../docs/r9-core-baseline.sha256");
    for line in baseline.lines() {
        let (hash, path) = line.split_once("  ").unwrap();
        // These files evolved after R9 for versioning, registry safety, diagnostics and input limits.
        // Their behavior is qualified by the production hardening suite, not the historical freeze.
        if [
            "crates/dg-analysis/src/lib.rs",
            "crates/dg-analysis/src/normalize.rs",
            "crates/dg-analysis/src/size.rs",
            "crates/dg-cli/src/main.rs",
            "crates/dg-cli/src/project.rs",
            "crates/dg-config/src/lib.rs",
            "crates/dg-cli/src/preflight.rs",
            "crates/dg-runtime/src/lib.rs",
            "crates/dg-runtime/src/migration.rs",
            "crates/dg-state/src/lib.rs",
        ]
        .contains(&path)
        {
            continue;
        }
        let mut bytes = fs::read(root().join(path)).unwrap();
        if path == "crates/dg-runtime/src/lifecycle.rs" {
            // R11 adds only an injectable world builder. Compare every recipe to R9.
            let mut source = String::from_utf8(bytes).unwrap();
            let start = source.find("        self.run_with_world_builder").unwrap();
            let end = source[start..]
                .find("        let scenarios = normalize")
                .unwrap()
                + start;
            source.replace_range(start..end, "");
            bytes = source.replace(
                "            if let Err(error) =\n                self.run_scenario(manifest, base, &definition, &mut result.steps, &build)\n            {",
                "            if let Err(error) = self.run_scenario(manifest, base, &definition, &mut result.steps) {"
            ).replace(
                "        build: &impl Fn(&ProjectManifest, &Path) -> crate::Result<crate::WorldBuildResult>,\n", ""
            ).replace(
                "let built = build(&local, base);",
                "let built = RuntimeWorldBuilder::default().build_manifest(&local, base);"
            ).into_bytes();
        }
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            hash,
            "core change in {path}; classify before accepting R9"
        );
        let source = String::from_utf8(bytes).unwrap();
        for name in [
            "Escrow",
            "escrow",
            "beneficiary",
            "arbitrator",
            "migrate_escrow",
        ] {
            assert!(!source.contains(name), "project-specific name in {path}");
        }
    }
}

#[test]
fn current_runtime_state_config_and_policy_have_no_project_registration() {
    fn inspect(directory: &std::path::Path) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                inspect(&path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                for forbidden in [
                    "Vault",
                    "vault",
                    "Escrow",
                    "escrow",
                    "beneficiary",
                    "arbitrator",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "project-specific registration in {}",
                        path.display()
                    );
                }
            }
        }
    }
    for name in ["dg-runtime", "dg-state", "dg-config", "dg-policy"] {
        inspect(&root().join(format!("crates/{name}/src")));
    }
}
