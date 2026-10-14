use anchor_lang::AccountDeserialize;
use dg_analysis::model::ProgramModel;
use dg_config::{preservation::PreservationRule, ProjectManifest};
use dg_runtime::{
    preservation::{evaluate, Status},
    MigrationExecution, MigrationExecutor, RuntimeWorldBuilder,
};
use std::path::PathBuf;

fn execute(faulty: bool) -> (MigrationExecution, ProjectManifest, ProgramModel) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/vault-runtime");
    let mut manifest =
        ProjectManifest::parse(&std::fs::read_to_string(base.join("doppelganger.toml")).unwrap())
            .unwrap();
    if faulty {
        manifest.versions.v2.program_path = "../../target/deploy/bad/dg_vault_v2.so".into();
    }
    let mut result = RuntimeWorldBuilder::default()
        .build_manifest(&manifest, &base)
        .unwrap();
    let execution = MigrationExecutor::default()
        .execute_manifest(&mut result.world, &manifest, &base)
        .unwrap();
    assert!(execution.transaction.as_ref().unwrap().success);
    (execution, manifest, result.world.model)
}

fn assert_overlapping_reference(faulty: bool) {
    let (execution, manifest, model) = execute(faulty);
    let before = execution.before_accounts[0].snapshot.as_ref().unwrap();
    let after = execution.after_accounts[0].snapshot.as_ref().unwrap();
    let old = dg_vault_v1::VaultV1::try_deserialize(&mut before.raw_data.as_slice()).unwrap();
    let new = dg_vault_v2::VaultV2::try_deserialize(&mut after.raw_data.as_slice()).unwrap();
    // These are the overlapping typed predicates in dg-cli's original demo.
    // No lifecycle operations are executed by this acceptance test.
    let reference = [
        ("owner-field", new.owner == old.owner),
        ("balance-field", new.balance == old.balance),
        ("limit-default", new.withdrawal_limit == 500_000),
        ("program-owner", before.owner == after.owner),
        ("account-size", after.data_length >= 56),
        ("address-continuity", before.address == after.address),
    ];
    let report = evaluate(&execution, &manifest.preservation, &model).unwrap();
    assert_eq!(
        (
            report.rules_total,
            report.passed,
            report.failed,
            report.errors
        ),
        (7, 7, 0, 0)
    );
    for (id, passed) in reference {
        assert_eq!(
            report
                .evaluations
                .iter()
                .find(|e| e.rule_id == id)
                .unwrap()
                .status
                == Status::Pass,
            passed
        );
    }
    assert!(!execution.preservation_evaluated);
    assert!(!report.lifecycle_evaluated && !report.policy_evaluated);
}
#[test]
fn good_vault_matches_overlapping_typed_demo_checks() {
    assert_overlapping_reference(false);
}
#[test]
fn faulty_backing_fixture_does_not_invent_an_economic_rule() {
    assert_overlapping_reference(true);
}

#[test]
fn exact_lamports_rule_is_not_a_backing_rule() {
    let rule: PreservationRule =
        toml::from_str("account='vault'\nrule='lamports_preserved'").unwrap();
    for (faulty, expected) in [(false, Status::Fail), (true, Status::Pass)] {
        let (execution, _, model) = execute(faulty);
        let report = evaluate(&execution, std::slice::from_ref(&rule), &model).unwrap();
        assert_eq!(report.evaluations[0].status, expected);
        if !faulty {
            assert_eq!(
                report.findings[0].evaluation.category.as_deref(),
                Some("LAMPORTS_CHANGED")
            );
        }
    }
}

#[test]
fn explicit_default_violation_produces_generic_finding_on_faulty_vault() {
    let (execution, _, model) = execute(true);
    // Negative control with a deliberately different developer expectation.
    // This detects a default mismatch, NOT the fixture's SOL-backing defect.
    let rule: PreservationRule=toml::from_str("id='explicit-default'\naccount='vault'\nfield='withdrawal_limit'\nrule='default'\nvalue='u64:1000'").unwrap();
    let report = evaluate(&execution, &[rule], &model).unwrap();
    assert_eq!((report.failed, report.errors), (1, 0));
    assert_eq!(
        report.findings[0].evaluation.category.as_deref(),
        Some("DEFAULT_VALUE_MISMATCH")
    );
    assert!(report.findings[0].evaluation.after.is_some());
}
