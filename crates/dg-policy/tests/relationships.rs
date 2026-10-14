use dg_policy::{
    relationships::{self, Account, Kind, Relationship},
    CheckStatus,
};
use dg_state::Value;
use std::collections::{BTreeMap, BTreeSet};

const A: &str = "11111111111111111111111111111112";
const B: &str = "11111111111111111111111111111113";
const PROGRAM: &str = "11111111111111111111111111111111";
fn source() -> Account {
    Account {
        name: "position".into(),
        address: A.into(),
        owner: Some(PROGRAM.into()),
        exists: true,
        fields: Some(BTreeMap::from([(
            "authority".into(),
            Value::Pubkey(B.into()),
        )])),
        decode_error: None,
    }
}
fn rule(kind: Kind) -> Relationship {
    Relationship {
        id: "link".into(),
        kind,
        account: "position".into(),
        field: (!matches!(kind, Kind::Pda | Kind::ProgramOwner)).then(|| "authority".into()),
        before_field: None,
        target: "$target".into(),
        check_before: true,
        preserve: true,
    }
}
fn report(
    rule: Relationship,
    before: Vec<Account>,
    after: Vec<Account>,
    target: &str,
    pda: bool,
) -> relationships::GraphReport {
    relationships::evaluate(
        &[rule],
        &before,
        &after,
        &BTreeMap::from([("target".into(), target.into())]),
        &if pda {
            BTreeSet::from(["target".into()])
        } else {
            BTreeSet::new()
        },
    )
    .unwrap()
}
#[test]
fn all_field_edge_kinds_validate_expected_targets() {
    let target = Account {
        name: "parent".into(),
        address: B.into(),
        ..source()
    };
    for kind in [
        Kind::Authority,
        Kind::Owner,
        Kind::Reference,
        Kind::Parent,
        Kind::Mint,
    ] {
        let accounts = vec![source(), target.clone()];
        let r = report(rule(kind), accounts.clone(), accounts, B, false);
        assert_eq!((r.passed, r.failed, r.errors), (1, 0, 0));
    }
}
#[test]
fn authority_change_is_a_violation_with_values() {
    let mut after = source();
    after
        .fields
        .as_mut()
        .unwrap()
        .insert("authority".into(), Value::Pubkey(A.into()));
    let r = report(rule(Kind::Authority), vec![source()], vec![after], B, false);
    assert_eq!(r.failed, 1);
    assert_eq!(r.edges[0].before.as_deref(), Some(B));
    assert_eq!(r.edges[0].after.as_deref(), Some(A));
}
#[test]
fn preexisting_wrong_authority_is_not_silently_accepted() {
    let r = report(
        rule(Kind::Authority),
        vec![source()],
        vec![source()],
        A,
        false,
    );
    assert_eq!(r.failed, 1);
}
#[test]
fn dangling_reference_fails() {
    let r = report(
        rule(Kind::Reference),
        vec![source()],
        vec![source()],
        B,
        false,
    );
    assert_eq!(r.failed, 1);
    assert_eq!(
        r.edges[0].code.as_deref(),
        Some("REFERENCED_ACCOUNT_MISSING")
    );
}
#[test]
fn removed_parent_fails() {
    let parent = Account {
        name: "parent".into(),
        address: B.into(),
        ..source()
    };
    let r = report(
        rule(Kind::Parent),
        vec![source(), parent],
        vec![source()],
        B,
        false,
    );
    assert_eq!(r.failed, 1);
}
#[test]
fn metadata_owner_change_is_detected() {
    let mut after = source();
    after.owner = Some(B.into());
    let r = report(
        rule(Kind::ProgramOwner),
        vec![source()],
        vec![after],
        PROGRAM,
        false,
    );
    assert_eq!(r.failed, 1);
}
#[test]
fn pda_continuity_is_checked_against_configured_derivation() {
    let r = report(rule(Kind::Pda), vec![source()], vec![source()], A, true);
    assert_eq!(r.passed, 1);
    let mut after = source();
    after.address = B.into();
    let r = report(rule(Kind::Pda), vec![source()], vec![after], A, true);
    assert_eq!(r.failed, 1);
}
#[test]
fn a_literal_binding_cannot_impersonate_pda_validation() {
    let r = report(rule(Kind::Pda), vec![source()], vec![source()], A, false);
    assert_eq!(r.errors, 1);
    assert_eq!(r.edges[0].code.as_deref(), Some("TARGET_IS_NOT_PDA"));
}
#[test]
fn missing_decode_is_an_error_instead_of_a_pass() {
    let mut after = source();
    after.decode_error = Some("TRUNCATED".into());
    let r = report(rule(Kind::Authority), vec![source()], vec![after], B, false);
    assert_eq!(r.errors, 1);
}
#[test]
fn missing_or_wrongly_typed_field_is_an_error() {
    for value in [None, Some(Value::String(B.into()))] {
        let mut after = source();
        after.fields = Some(
            value
                .map(|v| BTreeMap::from([("authority".into(), v)]))
                .unwrap_or_default(),
        );
        let r = report(rule(Kind::Authority), vec![source()], vec![after], B, false);
        assert_eq!(r.errors, 1);
    }
}
#[test]
fn nested_and_renamed_paths_work() {
    let mut r = rule(Kind::Authority);
    r.before_field = Some("authority".into());
    r.field = Some("config.owner".into());
    let mut after = source();
    after.fields = Some(BTreeMap::from([(
        "config".into(),
        Value::Struct(BTreeMap::from([("owner".into(), Value::Pubkey(B.into()))])),
    )]));
    assert_eq!(report(r, vec![source()], vec![after], B, false).passed, 1);
}
#[test]
fn newly_added_relationship_requires_explicit_after_only_semantics() {
    let mut r = rule(Kind::Authority);
    r.check_before = false;
    assert!(relationships::validate(&[r.clone()]).is_err());
    r.preserve = false;
    assert_eq!(report(r, vec![], vec![source()], B, false).passed, 1);
}
#[test]
fn absent_or_unobserved_source_never_passes() {
    let mut absent = source();
    absent.exists = false;
    let r = report(
        rule(Kind::Authority),
        vec![source()],
        vec![absent],
        B,
        false,
    );
    assert_eq!(r.failed, 1);
    let r = report(rule(Kind::Authority), vec![source()], vec![], B, false);
    assert_eq!(r.errors, 1);
}
#[test]
fn duplicate_sources_and_targets_are_errors() {
    let r = report(
        rule(Kind::Authority),
        vec![source()],
        vec![source(), source()],
        B,
        false,
    );
    assert_eq!(r.errors, 1);
    let target = Account {
        name: "target".into(),
        address: B.into(),
        ..source()
    };
    let r = report(
        rule(Kind::Reference),
        vec![source(), target.clone(), target.clone()],
        vec![source(), target],
        B,
        false,
    );
    assert_eq!(r.errors, 1);
}
#[test]
fn graph_serialization_is_deterministic_and_inputs_are_immutable() {
    let before = vec![source()];
    let bytes = serde_json::to_vec(&before).unwrap();
    let a = report(
        rule(Kind::Authority),
        before.clone(),
        before.clone(),
        B,
        false,
    );
    let b = report(
        rule(Kind::Authority),
        before.clone(),
        before.clone(),
        B,
        false,
    );
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert_eq!(serde_json::to_vec(&before).unwrap(), bytes);
    assert_eq!(a.edges[0].status, CheckStatus::Pass);
}
#[test]
fn bad_configuration_is_rejected() {
    let r = rule(Kind::Authority);
    assert!(relationships::validate(&[r.clone(), r]).is_err());
    let mut r = rule(Kind::Authority);
    r.field = Some("bad[0]".into());
    assert!(relationships::validate(&[r]).is_err());
    let mut r = rule(Kind::ProgramOwner);
    r.field = Some("owner".into());
    assert!(relationships::validate(&[r]).is_err());
}
