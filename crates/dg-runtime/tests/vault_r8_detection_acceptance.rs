mod r8_vault_support;
use dg_config::lifecycle::Outcome;
use dg_runtime::lifecycle::{changed_accounts, ScenarioStatus};

#[test]
fn manifests_differ_only_in_name_and_candidate() {
    let mut good: toml::Value = toml::from_str(
        &std::fs::read_to_string(
            r8_vault_support::root().join("examples/vault-runtime/doppelganger.toml"),
        )
        .unwrap(),
    )
    .unwrap();
    let broken: toml::Value = toml::from_str(
        &std::fs::read_to_string(
            r8_vault_support::root().join("examples/vault-runtime-r8-broken/doppelganger.toml"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_ne!(
        good["versions"]["v2"]["program_path"],
        broken["versions"]["v2"]["program_path"]
    );
    good["versions"]["v2"]["program_path"] = broken["versions"]["v2"]["program_path"].clone();
    good["project"]["name"] = broken["project"]["name"].clone();
    assert_eq!(good, broken);
}
#[test]
fn unchanged_suite_detects_only_repeat_acceptance() {
    let good = r8_vault_support::run(false);
    let broken = r8_vault_support::run(true);
    assert_eq!(
        (good.total, good.passed, good.failed, good.errors),
        (7, 7, 0, 0)
    );
    assert_eq!(
        (broken.total, broken.passed, broken.failed, broken.errors),
        (7, 6, 1, 0),
        "{}",
        serde_json::to_string_pretty(&broken).unwrap()
    );
    for (a, b) in good.scenarios.iter().zip(&broken.scenarios) {
        assert_eq!(a.id, b.id);
        assert_eq!(
            serde_json::to_value(&a.definition).unwrap(),
            serde_json::to_value(&b.definition).unwrap()
        );
        assert_eq!(a.steps[0].after[0].snapshot, b.steps[0].after[0].snapshot);
        if b.id == "repeat-migration" {
            assert_eq!(b.status, ScenarioStatus::Fail);
            let second = &b.steps[2];
            assert_eq!(second.expected, Some(Outcome::Error));
            assert_eq!(second.observed, Some(Outcome::Success));
            assert!(second
                .diagnostics
                .iter()
                .any(|d| d.code == "OUTCOME_MISMATCH"));
            assert!(changed_accounts(&second.before, &second.after).is_empty());
            assert!(second.after[0].decoded.is_some());
        } else {
            assert_eq!(b.status, ScenarioStatus::Pass);
        }
    }
    // Normal migration produces identical raw state; only repeat behavior differs.
    assert_eq!(
        good.scenarios[0].steps[1].after[0].snapshot,
        broken.scenarios[0].steps[1].after[0].snapshot
    );
}
#[test]
fn complete_reports_repeat_deterministically_for_both_candidates() {
    for broken in [false, true] {
        assert_eq!(
            serde_json::to_vec(&r8_vault_support::run(broken)).unwrap(),
            serde_json::to_vec(&r8_vault_support::run(broken)).unwrap()
        );
    }
}
