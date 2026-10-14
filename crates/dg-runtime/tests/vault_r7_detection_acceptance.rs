//! Candidate discrimination using the same V1 world recipe and seven real rules.
use anchor_lang::AccountDeserialize;
use dg_config::{addresses::AddressDefinition, preservation::RuleKind, ProjectManifest};
use dg_runtime::{
    preservation::{evaluate, PreservationReport, Resolution, Status},
    MigrationExecution, MigrationExecutor, RuntimeWorldBuilder,
};
use dg_state::Value;
use std::{fs, path::PathBuf};

fn base(broken: bool) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if broken {
        "../../examples/vault-runtime-r7-broken"
    } else {
        "../../examples/vault-runtime"
    })
}

fn execute(broken: bool) -> (MigrationExecution, PreservationReport) {
    let base = base(broken);
    let mut manifest =
        ProjectManifest::parse(&fs::read_to_string(base.join("doppelganger.toml")).unwrap())
            .unwrap();
    for (name, seed) in [("owner", [11; 32]), ("stranger", [12; 32])] {
        match manifest.addresses.get_mut(name).unwrap() {
            AddressDefinition::Signer { test_seed, .. } => *test_seed = Some(seed),
            _ => panic!("expected configured signer"),
        }
    }
    let candidate = base.join(&manifest.versions.v2.program_path);
    assert!(candidate.is_file(), "Missing candidate {}. Build the R7 acceptance fixture as documented; tests do not skip artifacts.", candidate.display());
    let mut built = RuntimeWorldBuilder::default()
        .build_manifest(&manifest, &base)
        .unwrap();
    let execution = MigrationExecutor::default()
        .execute_manifest(&mut built.world, &manifest, &base)
        .unwrap();
    let transaction = execution.transaction.as_ref().unwrap();
    assert!(
        transaction.success,
        "R6 must succeed: {:?}",
        transaction.error
    );
    assert!(transaction.error.is_none() && transaction.transaction_error.is_none());
    assert!(execution.after_accounts[0].snapshot.is_some());
    assert!(execution.after_accounts[0].decoded.is_some());
    assert!(execution.after_accounts[0].decode_error.is_none());
    let report = evaluate(&execution, &manifest.preservation, &built.world.model).unwrap();
    (execution, report)
}

#[test]
fn manifests_differ_only_in_project_name_and_candidate_binary() {
    let mut good: toml::Value =
        toml::from_str(&fs::read_to_string(base(false).join("doppelganger.toml")).unwrap())
            .unwrap();
    let broken: toml::Value =
        toml::from_str(&fs::read_to_string(base(true).join("doppelganger.toml")).unwrap()).unwrap();
    assert_ne!(
        good["versions"]["v2"]["program_path"],
        broken["versions"]["v2"]["program_path"]
    );
    good["project"]["name"] = broken["project"]["name"].clone();
    good["versions"]["v2"]["program_path"] = broken["versions"]["v2"]["program_path"].clone();
    assert_eq!(
        good, broken,
        "V1 setup, addresses, instructions, IDLs and preservation rules must match"
    );
}

#[test]
fn same_rules_pass_good_candidate_and_detect_one_real_balance_violation() {
    let (good_execution, good) = execute(false);
    let (broken_execution, broken) = execute(true);
    assert_eq!(
        serde_json::to_value(&good.rules).unwrap(),
        serde_json::to_value(&broken.rules).unwrap(),
        "normalized rule IDs, order and contents must match"
    );
    assert_eq!(
        serde_json::to_value(&good_execution.before_accounts).unwrap(),
        serde_json::to_value(&broken_execution.before_accounts).unwrap(),
        "both candidates start with identical V1 account evidence"
    );
    assert_eq!(
        (
            good.rules_total,
            good.passed,
            good.failed,
            good.errors,
            good.not_applicable
        ),
        (7, 7, 0, 0, 0)
    );
    assert!(good.findings.is_empty());
    assert_eq!(
        (
            broken.rules_total,
            broken.passed,
            broken.failed,
            broken.errors,
            broken.not_applicable
        ),
        (7, 6, 1, 0, 0)
    );
    assert_eq!(broken.findings.len(), 1);
    let finding = &broken.findings[0];
    let e = &finding.evaluation;
    assert_eq!(finding.id, "PRESERVE-balance-field");
    assert_eq!(e.rule_id, "balance-field");
    assert_eq!(e.account, "vault");
    assert_eq!(e.field.as_deref(), Some("balance"));
    assert_eq!(e.rule, RuleKind::Equal);
    assert_eq!(e.status, Status::Fail);
    assert_eq!(e.category.as_deref(), Some("FIELD_NOT_PRESERVED"));
    assert_eq!(
        e.before,
        Some(Resolution::Found(Value::Integer(1_000_000u64.into())))
    );
    assert_eq!(
        e.after,
        Some(Resolution::Found(Value::Integer(950_000u64.into())))
    );
    assert!(e.expected.is_none() && e.error_code.is_none());
    for other in &broken.evaluations {
        if other.rule_id != "balance-field" {
            assert_eq!(other.status, Status::Pass, "{}", other.rule_id);
        }
    }

    let before = broken_execution.before_accounts[0]
        .snapshot
        .as_ref()
        .unwrap();
    let after = broken_execution.after_accounts[0]
        .snapshot
        .as_ref()
        .unwrap();
    let good_after = good_execution.after_accounts[0].snapshot.as_ref().unwrap();
    let old = dg_vault_v1::VaultV1::try_deserialize(&mut before.raw_data.as_slice()).unwrap();
    let new = dg_vault_v2::VaultV2::try_deserialize(&mut after.raw_data.as_slice()).unwrap();
    let good_new =
        dg_vault_v2::VaultV2::try_deserialize(&mut good_after.raw_data.as_slice()).unwrap();
    assert_eq!(old.balance, 1_000_000);
    assert_eq!(good_new.balance, old.balance);
    assert_eq!(new.balance, 950_000);
    assert_eq!(new.owner, old.owner);
    assert_eq!(new.withdrawal_limit, 500_000);
    assert_eq!(after.address, before.address);
    assert_eq!(after.owner, before.owner);
    assert_eq!(after.data_length, 56);
    assert_eq!(after.lamports, good_after.lamports);
    // The serialized balance is the only difference between the two V2 accounts.
    let mut only_balance_changed = good_after.clone();
    only_balance_changed.raw_data[40..48].copy_from_slice(&950_000u64.to_le_bytes());
    assert_eq!(&only_balance_changed, after);
    assert_eq!(broken.migration_success, Some(true));
    assert!(!broken.lifecycle_evaluated && !broken.policy_evaluated);
}

#[test]
fn broken_candidate_reports_are_identical_across_deterministic_runs() {
    let (first_execution, first) = execute(true);
    let (second_execution, second) = execute(true);
    assert_eq!(
        serde_json::to_vec(&first_execution).unwrap(),
        serde_json::to_vec(&second_execution).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(first.failed, 1);
    assert_eq!(first.findings[0].id, second.findings[0].id);
}
