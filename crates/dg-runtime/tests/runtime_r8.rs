mod r6_support;
use anchor_lang::Discriminator;
use dg_config::{lifecycle::LifecycleScenario, ProjectManifest};
use dg_runtime::lifecycle::{
    changed_accounts, LifecycleEngine, LifecycleReport, Operation, ScenarioStatus,
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    dir: PathBuf,
    manifest: ProjectManifest,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
impl Fixture {
    fn new(scenarios: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "dg-r8-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        for v2 in [false, true] {
            let mut idl = r6_support::idl(v2);
            if v2 {
                let instructions = idl["instructions"].as_array_mut().unwrap();
                instructions.push(json!({"name":"operate","discriminator":dg_r6_fixture_v2::instruction::Operate::DISCRIMINATOR,"accounts":[{"name":"record","writable":true},{"name":"authority","signer":true}],"args":[{"name":"value","type":"u64"},{"name":"fail","type":"bool"}]}));
                instructions.push(json!({"name":"noop","discriminator":dg_r6_fixture_v2::instruction::Noop::DISCRIMINATOR,"accounts":[{"name":"record"},{"name":"authority","signer":true}],"args":[]}));
            }
            fs::write(
                dir.join(if v2 { "v2.json" } else { "v1.json" }),
                idl.to_string(),
            )
            .unwrap();
        }
        let source = format!(
            r#"
[project]
name = 'non-vault-lifecycle'
framework = 'anchor'
[versions.v1]
program_path = '{}'
idl = 'v1.json'
program_id = '{}'
[versions.v2]
program_path = '{}'
idl = 'v2.json'
program_id = '{}'
[addresses.owner]
type = 'signer'
test_seed = {:?}
lamports = 'u64:10000000000'
[addresses.unused]
type = 'signer'
test_seed = {:?}
lamports = 'u64:1000000000'
[addresses.alias]
type = 'reference'
account = '$owner'
[addresses.program]
type = 'program'
version = 'v1'
[addresses.system]
type = 'literal'
address = '11111111111111111111111111111111'
[addresses.record]
type = 'pda'
program = 'v1'
seeds = [{{ const = 'record' }}, {{ pubkey = '$owner' }}]
[fixture]
name = 'record'
payer = 'owner'
[[fixture.account]]
name = 'record'
type = 'RecordV1'
address = '$record'
owner = '$program'
lamports = 0
[migration]
instruction = 'migrate'
account_mapping = {{ record = 'record', authority = 'alias', system_program = 'system' }}
args = {{ mode = 0, version = 2, value = 42 }}
{scenarios}
"#,
            r6_support::artifact(false).program_path.display(),
            dg_r6_fixture_v1::id(),
            r6_support::artifact(true).program_path.display(),
            dg_r6_fixture_v2::id(),
            vec![7; 32],
            vec![8; 32]
        );
        let mut manifest = ProjectManifest::parse(&source).unwrap();
        manifest
            .fixture
            .as_mut()
            .unwrap()
            .initialize
            .push(r6_support::definition("initialize"));
        manifest
            .fixture
            .as_mut()
            .unwrap()
            .setup
            .push(r6_support::definition("advance"));
        manifest.migration_targets = r6_support::targets();
        Self { dir, manifest }
    }
    fn run(&self) -> LifecycleReport {
        LifecycleEngine.run(&self.manifest, &self.dir).unwrap()
    }
}
fn scenario(kind: &str, extra: &str) -> String {
    format!("[[scenario]]\ntype='{kind}'\n{extra}\n")
}
fn standard() -> Fixture {
    Fixture::new(&scenario("standard_migration", "expected='success'"))
}
fn status(f: &Fixture, expected: ScenarioStatus) -> LifecycleReport {
    let r = f.run();
    assert_eq!(
        r.scenarios[0].status,
        expected,
        "{}",
        serde_json::to_string_pretty(&r).unwrap()
    );
    r
}
fn operation(kind: &str, expected: &str, fail: bool) -> Fixture {
    Fixture::new(&scenario(kind,&format!("instruction='operate'\nexpected='{expected}'\naccount_mapping={{record='$record',authority='$alias'}}\nargs={{value=99,fail={fail}}}")))
}

#[test]
fn standard_success() {
    let r = status(&standard(), ScenarioStatus::Pass);
    assert_eq!(r.scenarios[0].steps[0].setup_actions.len(), 2);
    assert!(!r.preservation_evaluated && !r.policy_evaluated);
}
#[test]
fn standard_expected_error() {
    let mut f = standard();
    f.manifest.scenario[0].expected = Some(dg_config::lifecycle::Outcome::Error);
    f.manifest
        .migration
        .arguments
        .insert("mode".into(), 2.into());
    status(&f, ScenarioStatus::Pass);
}
#[test]
fn standard_mismatch() {
    let f = Fixture::new(&scenario("standard_migration", "expected='error'"));
    status(&f, ScenarioStatus::Fail);
}
#[test]
fn standard_unexpected_rejection() {
    let mut f = standard();
    f.manifest
        .migration
        .arguments
        .insert("mode".into(), 2.into());
    status(&f, ScenarioStatus::Fail);
}
#[test]
fn standard_engine_failure_is_not_rejection() {
    let mut f = Fixture::new(&scenario("standard_migration", "expected='error'"));
    f.manifest.migration.instruction = "absent".into();
    status(&f, ScenarioStatus::Error);
}
fn unauthorized(extra: &str) -> Fixture {
    Fixture::new(&scenario(
        "unauthorized_migration",
        &format!("expected='error'\nsigner_account='authority'\nsigner_override='unused'\n{extra}"),
    ))
}
#[test]
fn unauthorized_rejection_and_exact_code() {
    status(
        &unauthorized("expected_error=2003\nstate_unchanged=true"),
        ScenarioStatus::Pass,
    );
}
#[test]
fn unauthorized_wrong_error_identity_fails() {
    status(&unauthorized("expected_error=9999"), ScenarioStatus::Fail);
}
#[test]
fn unauthorized_unexpected_acceptance() {
    let mut f = unauthorized("");
    f.manifest.scenario[0].signer_override = Some("alias".into());
    status(&f, ScenarioStatus::Fail);
}
#[test]
fn unresolved_override_is_engine_error() {
    let mut f = unauthorized("");
    f.manifest.scenario[0].signer_override = Some("absent".into());
    status(&f, ScenarioStatus::Error);
}
#[test]
fn override_without_keypair_is_engine_error() {
    let mut f = unauthorized("");
    f.manifest.scenario[0].signer_override = Some("system".into());
    status(&f, ScenarioStatus::Error);
}
#[test]
fn override_must_select_idl_signer() {
    let mut f = unauthorized("");
    f.manifest.scenario[0].signer_account = Some("record".into());
    status(&f, ScenarioStatus::Error);
}
fn repeat(second: &str) -> Fixture {
    Fixture::new(&scenario(
        "repeat_migration",
        &format!("first='success'\nsecond='{second}'"),
    ))
}
#[test]
fn repeat_uses_same_world_without_reload() {
    let r = status(&repeat("error"), ScenarioStatus::Pass);
    let s = &r.scenarios[0].steps;
    assert_eq!(s.len(), 3);
    assert!(s[1].program_swap.is_some());
    assert!(s[2].program_swap.is_none());
    assert_eq!(s[1].after[0].snapshot, s[2].before[0].snapshot);
    assert!(changed_accounts(&s[2].before, &s[2].after).is_empty());
    assert_ne!(
        s[1].execution.as_ref().unwrap().signature,
        s[2].execution.as_ref().unwrap().signature
    );
}
#[test]
fn repeat_acceptance_can_be_expected() {
    let mut f = repeat("success");
    f.manifest.migration = r6_support::definition("noop");
    status(&f, ScenarioStatus::Pass);
}
#[test]
fn repeat_unexpected_acceptance_fails() {
    let mut f = repeat("error");
    f.manifest.migration = r6_support::definition("noop");
    status(&f, ScenarioStatus::Fail);
}
fn rollback() -> Fixture {
    let mut f = Fixture::new(&scenario(
        "failed_migration_rollback",
        "expected='error'\nexpected_error=6000",
    ));
    let mut d = f.manifest.migration.clone();
    d.arguments.insert("mode".into(), 2.into());
    f.manifest.scenario[0].migration = Some(d);
    f
}
#[test]
fn rollback_after_actual_write_resize_transfer() {
    let r = status(&rollback(), ScenarioStatus::Pass);
    let s = &r.scenarios[0].steps[1];
    assert!(s
        .execution
        .as_ref()
        .unwrap()
        .logs
        .iter()
        .any(|l| l.contains("attempted write, realloc and transfer")));
    assert!(changed_accounts(&s.before, &s.after).is_empty());
    assert_eq!(s.before[0].snapshot.as_ref().unwrap().data_length, 48);
}
#[test]
fn raw_comparison_detects_each_account_property() {
    let r = standard().run();
    let b = &r.scenarios[0].steps[1].before;
    for property in 0..6 {
        let mut a = b.clone();
        let raw = a[0].snapshot.as_mut().unwrap();
        match property {
            0 => a[0].address = "other".into(),
            1 => raw.owner = "other".into(),
            2 => raw.lamports = 0u64.into(),
            3 => raw.data_length += 1,
            4 => raw.raw_data[0] ^= 1,
            _ => raw.executable = !raw.executable,
        };
        assert_eq!(changed_accounts(b, &a), ["record"]);
    }
    assert_eq!(changed_accounts(b, &[]), ["record"]);
    let mut absent = b.clone();
    absent[0].snapshot = None;
    assert_eq!(changed_accounts(b, &absent), ["record"]);
}
#[test]
fn rollback_reports_selected_fee_payer_change() {
    let mut f = rollback();
    f.manifest
        .migration_targets
        .push(dg_config::MigrationTarget {
            name: "payer".into(),
            address: "$owner".into(),
            v1_type: None,
            v2_type: None,
        });
    let r = status(&f, ScenarioStatus::Fail);
    assert!(r.scenarios[0].steps[1]
        .diagnostics
        .iter()
        .any(|d| d.code == "STATE_CHANGED" && d.context == "payer"));
}
#[test]
fn unmigrated_rejects_old_layout_without_migration() {
    let r = status(
        &operation("unmigrated_state_against_v2", "error", false),
        ScenarioStatus::Pass,
    );
    assert_eq!(r.scenarios[0].steps[1].operation, Operation::LoadCandidate);
    assert!(r.scenarios[0].steps[1].execution.is_none());
    assert!(changed_accounts(
        &r.scenarios[0].steps[1].before,
        &r.scenarios[0].steps[1].after
    )
    .is_empty());
}
#[test]
fn unmigrated_acceptance_is_configurable() {
    let mut f = operation("unmigrated_state_against_v2", "success", false);
    f.manifest.scenario[0].instruction = Some("noop".into());
    f.manifest.scenario[0].arguments.clear();
    status(&f, ScenarioStatus::Pass);
}
#[test]
fn unmigrated_outcome_mismatch() {
    status(
        &operation("unmigrated_state_against_v2", "success", false),
        ScenarioStatus::Fail,
    );
}
#[test]
fn post_migration_operation_success() {
    let r = status(
        &operation("post_migration_instruction", "success", false),
        ScenarioStatus::Pass,
    );
    let s = &r.scenarios[0].steps;
    assert_eq!(s[1].after[0].snapshot, s[2].before[0].snapshot);
    assert_eq!(
        &s[2].after[0].snapshot.as_ref().unwrap().raw_data[40..48],
        &99u64.to_le_bytes()
    );
}
#[test]
fn post_migration_expected_program_rejection() {
    status(
        &operation("post_migration_instruction", "error", true),
        ScenarioStatus::Pass,
    );
}
#[test]
fn post_migration_stops_if_migration_fails() {
    let mut f = operation("post_migration_instruction", "success", false);
    f.manifest
        .migration
        .arguments
        .insert("mode".into(), 2.into());
    let r = status(&f, ScenarioStatus::Fail);
    assert_eq!(r.scenarios[0].steps.len(), 2);
}
fn boundary(kind: &str, value: u64) {
    let mut f = Fixture::new(&scenario(kind, "fixture='edge'\nexpected='success'"));
    let mut variant = f.manifest.fixture.clone().unwrap();
    variant.setup.clear();
    variant.initialize[0]
        .arguments
        .insert("value".into(), format!("u64:{value}").into());
    f.manifest.fixtures.insert("edge".into(), variant);
    let r = status(&f, ScenarioStatus::Pass);
    assert_eq!(
        &r.scenarios[0].steps[0].after[0]
            .snapshot
            .as_ref()
            .unwrap()
            .raw_data[40..48],
        &value.to_le_bytes()
    );
}
#[test]
fn zero_fixture() {
    boundary("zero_value_state", 0);
}
#[test]
fn maximum_u64_fixture_is_lossless() {
    boundary("boundary_value_state", u64::MAX);
}
#[test]
fn invalid_fixture_reference_is_error() {
    let f = Fixture::new(&scenario(
        "zero_value_state",
        "fixture='absent'\nexpected='success'",
    ));
    status(&f, ScenarioStatus::Error);
}
#[test]
fn independent_scenarios_start_fresh() {
    let mut f = operation("post_migration_instruction", "success", false);
    f.manifest
        .scenario
        .push(standard().manifest.scenario.remove(0));
    let r = f.run();
    assert_eq!(r.passed, 2);
    assert_eq!(
        r.scenarios[0].steps[0].after[0].snapshot,
        r.scenarios[1].steps[0].after[0].snapshot
    );
    assert_ne!(
        r.scenarios[0].steps[2].after[0].snapshot,
        r.scenarios[1].steps[0].after[0].snapshot
    );
}
#[test]
fn deterministic_reports_and_ids() {
    let f = repeat("error");
    let a = f.run();
    let b = f.run();
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert!(a.scenarios[0].id.starts_with("scenario-"));
}
#[test]
fn missing_candidate_never_satisfies_expected_error() {
    let mut f = Fixture::new(&scenario("standard_migration", "expected='error'"));
    f.manifest.versions.v2.program_path = "missing.so".into();
    status(&f, ScenarioStatus::Error);
}
#[test]
fn bad_setup_does_not_satisfy_rejection() {
    let mut f = Fixture::new(&scenario("standard_migration", "expected='error'"));
    f.manifest.fixture.as_mut().unwrap().initialize[0].instruction = "missing".into();
    let r = status(&f, ScenarioStatus::Error);
    assert_eq!(r.scenarios[0].steps.len(), 1);
}
#[test]
fn duplicate_generated_ids_rejected() {
    let mut f = standard();
    f.manifest.scenario.push(f.manifest.scenario[0].clone());
    assert!(LifecycleEngine.run(&f.manifest, &f.dir).is_err());
}
fn invalid(source: &str) {
    let parsed = toml::from_str::<LifecycleScenario>(source);
    assert!(
        parsed.is_err() || dg_config::lifecycle::validate_scenarios(&[parsed.unwrap()]).is_err()
    );
}
#[test]
fn config_unknown_type() {
    invalid("type='script'\nexpected='success'");
}
#[test]
fn config_missing_instruction() {
    invalid("type='post_migration_instruction'\nexpected='success'");
}
#[test]
fn config_missing_outcome() {
    invalid("type='standard_migration'");
}
#[test]
fn config_invalid_error_identity() {
    invalid("type='standard_migration'\nexpected='error'\nexpected_error='Unauthorized'");
}
#[test]
fn config_incompatible_fields() {
    invalid("type='standard_migration'\nexpected='success'\ninstruction='noop'");
}
#[test]
fn config_duplicate_explicit_ids() {
    let mut f = standard();
    f.manifest.scenario[0].id = Some("same".into());
    f.manifest.scenario.push(f.manifest.scenario[0].clone());
    assert!(LifecycleEngine.run(&f.manifest, &f.dir).is_err());
}
#[test]
fn config_missing_signer_override() {
    invalid("type='unauthorized_migration'\nexpected='error'\nsigner_account='authority'");
}
#[test]
fn config_boundary_requires_fixture() {
    invalid("type='boundary_value_state'\nexpected='success'");
}
#[test]
fn config_repeat_requires_both_outcomes() {
    invalid("type='repeat_migration'\nsecond='error'");
}
#[test]
fn config_error_code_requires_rejection() {
    invalid("type='standard_migration'\nexpected='success'\nexpected_error=6000");
}
#[test]
fn scenario_order_and_generated_ids_survive_reordering() {
    let mut f = operation("post_migration_instruction", "success", false);
    f.manifest
        .scenario
        .push(standard().manifest.scenario.remove(0));
    let first = f.run();
    f.manifest.scenario.reverse();
    let second = f.run();
    assert_eq!(first.scenarios[0].id, second.scenarios[1].id);
    assert_eq!(first.scenarios[1].id, second.scenarios[0].id);
    assert_eq!(
        serde_json::to_value(&first.scenarios[0]).unwrap(),
        serde_json::to_value(&second.scenarios[1]).unwrap()
    );
}
#[test]
fn engine_error_does_not_block_independent_scenarios() {
    let mut f = standard();
    let mut bad = f.manifest.scenario[0].clone();
    bad.fixture = Some("missing".into());
    f.manifest.scenario.insert(0, bad);
    let r = f.run();
    assert_eq!((r.total, r.errors, r.passed), (2, 1, 1));
    assert_eq!(r.scenarios[0].diagnostics[0].code, "FIXTURE_REFERENCE");
}
#[test]
fn lifecycle_module_only_orchestrates_existing_primitives() {
    let source = include_str!("../src/lifecycle.rs");
    for forbidden in [
        "Vault",
        "vault",
        "withdraw",
        "deposit",
        "LiteSVM::new",
        "add_program(",
        "send_transaction(",
        "preservation::evaluate",
    ] {
        assert!(
            !source.contains(forbidden),
            "unexpected lifecycle implementation: {forbidden}"
        );
    }
    for primitive in [
        "RuntimeWorldBuilder",
        "execute_manifest",
        "load_candidate",
        "execute_loaded",
    ] {
        assert!(source.contains(primitive));
    }
}
