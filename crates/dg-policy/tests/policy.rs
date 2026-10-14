use dg_analysis::diff::Finding;
use dg_policy::{evaluate, Check, CheckStatus, Coverage, Profile};
use serde_json::json;

fn profile(extra: &str) -> Profile {
    Profile::parse(&format!("schema_version=1\n{extra}")).unwrap()
}
fn coverage() -> Coverage {
    Coverage {
        preservation: 1,
        relationships: 1,
        lifecycle: 1,
    }
}
fn finding() -> Finding {
    Finding {
        id: "STATIC-0001".into(),
        category: "account".into(),
        target_type: "account".into(),
        target_name: "Record".into(),
        path: "account.Record.limit".into(),
        before: json!(null),
        after: json!({"kind":"primitive","value":"u64"}),
        classification: "ACCOUNT_SCHEMA_CHANGE".into(),
        explanation: "Field added".into(),
        migration_hints: vec![],
        suggested_tests: vec![],
    }
}
fn expected() -> &'static str {
    "[[expected_change]]\nclassification='ACCOUNT_SCHEMA_CHANGE'\npath='account.Record.limit'\nbefore_json='null'\nafter_json='{\"kind\":\"primitive\",\"value\":\"u64\"}'\nreason='Approved new withdrawal limit'\n"
}
#[test]
fn unexpected_static_change_blocks_by_default() {
    let r = evaluate(&profile(""), &[finding()], &[], coverage());
    assert!(!r.passed);
    assert_eq!(r.blocking, 1);
}
#[test]
fn exact_expected_change_is_allowed() {
    let r = evaluate(&profile(expected()), &[finding()], &[], coverage());
    assert!(r.passed);
    assert_eq!(r.expected, 1);
}
#[test]
fn changed_values_cannot_match_expected_change() {
    let mut f = finding();
    f.after = json!("other");
    let r = evaluate(&profile(expected()), &[f], &[], coverage());
    assert!(!r.passed);
    assert_eq!(r.expected, 0);
    assert_eq!(r.blocking, 2);
}
#[test]
fn stale_expectation_blocks() {
    assert!(!evaluate(&profile(expected()), &[], &[], coverage()).passed);
}
#[test]
fn warnings_and_ignores_are_explicit_and_counted() {
    for (action, warnings, ignored) in [("warn", 1, 0), ("ignore", 0, 1)] {
        let p = profile(&format!("[policy]\nunexpected_static='{action}'"));
        let r = evaluate(&p, &[finding()], &[], coverage());
        assert!(r.passed);
        assert_eq!((r.warnings, r.ignored), (warnings, ignored));
    }
}
#[test]
fn every_runtime_violation_blocks_even_when_static_changes_are_ignored() {
    for category in ["PRESERVATION", "RELATIONSHIP", "LIFECYCLE", "MIGRATION"] {
        let p = profile("[policy]\nunexpected_static='ignore'");
        let c = Check {
            id: "bad".into(),
            category: category.into(),
            status: CheckStatus::Fail,
            message: "bad".into(),
        };
        assert!(!evaluate(&p, &[], &[c], coverage()).passed);
    }
}
#[test]
fn evaluation_errors_remain_distinct() {
    let c = Check {
        id: "decode".into(),
        category: "RELATIONSHIP".into(),
        status: CheckStatus::Error,
        message: "bad bytes".into(),
    };
    let r = evaluate(&profile(""), &[], &[c], coverage());
    assert_eq!(r.evaluation_errors, 1);
    assert!(!r.passed);
}
#[test]
fn not_applicable_checks_do_not_imply_success() {
    let c = Check {
        id: "missing".into(),
        category: "PRESERVATION".into(),
        status: CheckStatus::NotApplicable,
        message: "not run".into(),
    };
    assert!(!evaluate(&profile(""), &[], &[c], coverage()).passed);
}
#[test]
fn empty_required_coverage_blocks() {
    let r = evaluate(
        &profile(""),
        &[],
        &[],
        Coverage {
            preservation: 0,
            relationships: 0,
            lifecycle: 0,
        },
    );
    assert_eq!(r.blocking, 3);
}
#[test]
fn optional_coverage_is_an_explicit_choice() {
    let p=profile("[policy]\nrequire_preservation=false\nrequire_relationships=false\nrequire_lifecycle=false");
    assert!(
        evaluate(
            &p,
            &[],
            &[],
            Coverage {
                preservation: 0,
                relationships: 0,
                lifecycle: 0
            }
        )
        .passed
    );
}
#[test]
fn unknown_versions_fields_and_invalid_expected_json_are_rejected() {
    for text in ["schema_version=2","schema_version=1\nunknown=true","schema_version=1\n[policy]\nrequire_lifecyle=false", "schema_version=1\n[[expected_change]]\nclassification='x'\npath='x'\nbefore_json='broken'\nafter_json='null'\nreason='x'"]{assert!(Profile::parse(text).is_err());}
}
#[test]
fn duplicate_or_unexplained_expectations_are_rejected() {
    assert!(Profile::parse(&format!("schema_version=1\n{}{}", expected(), expected())).is_err());
    assert!(Profile::parse(&format!(
        "schema_version=1\n{}",
        expected().replace("Approved new withdrawal limit", "")
    ))
    .is_err());
}
