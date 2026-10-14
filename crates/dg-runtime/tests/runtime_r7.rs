use dg_analysis::{
    model::{FieldType, ProgramModel},
    normalize,
};
use dg_config::preservation::PreservationRule;
use dg_runtime::{
    migration::{AccountEvidence, MigrationExecution},
    preservation::{evaluate, resolve_field, PreservationReport, Resolution, Status},
};
use dg_state::{
    raw::RawAccountSnapshot, DecodeLimits, DecodedAccount, ProgramArtifact, StateError, Value,
};
use serde_json::json;
use std::collections::BTreeMap;

fn integer(n: i128) -> Value {
    Value::Integer(n.into())
}
fn object(fields: &[(&str, Value)]) -> Value {
    Value::Struct(
        fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}
fn model() -> ProgramModel {
    normalize(json!({"instructions":[],"accounts":[{"name":"Record","discriminator":[1]}],"types":[
        {"name":"Record","type":{"kind":"struct","fields":[{"name":"value","type":"u128"},{"name":"new","type":"u64"},{"name":"config","type":{"defined":"Config"}}]}},
        {"name":"Config","type":{"kind":"struct","fields":[{"name":"flag","type":"bool"}]}},
        {"name":"Choice","type":{"kind":"enum","variants":[{"name":"Empty"},{"name":"Count","fields":["u64"]}]}}
    ]}).to_string().as_bytes()).unwrap()
}
fn account(value: Value) -> AccountEvidence {
    AccountEvidence {
        name: "record".into(),
        address: "11111111111111111111111111111111".into(),
        account_type: Some("Record".into()),
        snapshot: Some(
            RawAccountSnapshot::capture(
                "11111111111111111111111111111111".into(),
                "11111111111111111111111111111111".into(),
                100,
                false,
                vec![0; 48],
                DecodeLimits::default(),
            )
            .unwrap(),
        ),
        decoded: Some(DecodedAccount {
            account_type: "Record".into(),
            fields: BTreeMap::from([("value".into(), value)]),
            consumed_bytes: 48,
            trailing_bytes: vec![],
        }),
        decode_error: None,
    }
}
fn execution(before: Value, after: Value) -> MigrationExecution {
    let artifact = ProgramArtifact {
        program_id: "11111111111111111111111111111111".into(),
        program_path: "test.so".into(),
        idl_path: "test.json".into(),
    };
    MigrationExecution {
        schema_version: 1,
        project: "synthetic".into(),
        v1_program: artifact.clone(),
        v2_program: artifact,
        program_swap: None,
        before_accounts: vec![account(before)],
        after_accounts: vec![account(after)],
        transaction: None,
        preservation_evaluated: false,
    }
}
fn rule(text: &str) -> PreservationRule {
    toml::from_str(&format!("account='record'\n{text}")).unwrap()
}
fn report(e: &MigrationExecution, text: &str) -> PreservationReport {
    evaluate(e, &[rule(text)], &model()).unwrap()
}
fn status(e: &MigrationExecution, text: &str) -> Status {
    report(e, text).evaluations[0].status
}
fn fields_mut(a: &mut AccountEvidence) -> &mut BTreeMap<String, Value> {
    &mut a.decoded.as_mut().unwrap().fields
}

macro_rules! equality {
    ($name:ident,$before:expr,$after:expr,$status:ident) => {
        #[test]
        fn $name() {
            assert_eq!(
                status(&execution($before, $after), "field='value'\nrule='equal'"),
                Status::$status
            );
        }
    };
}
equality!(equal_integer, integer(100), integer(100), Pass);
equality!(different_integer, integer(100), integer(90), Fail);
equality!(
    equal_pubkey,
    Value::Pubkey("A".into()),
    Value::Pubkey("A".into()),
    Pass
);
equality!(
    different_pubkey,
    Value::Pubkey("A".into()),
    Value::Pubkey("B".into()),
    Fail
);
equality!(
    equal_enum,
    Value::Enum {
        variant: "Active".into(),
        fields: BTreeMap::new()
    },
    Value::Enum {
        variant: "Active".into(),
        fields: BTreeMap::new()
    },
    Pass
);
equality!(
    different_enum,
    Value::Enum {
        variant: "Active".into(),
        fields: BTreeMap::new()
    },
    Value::Enum {
        variant: "Closed".into(),
        fields: BTreeMap::new()
    },
    Fail
);
equality!(
    enum_payload_change,
    Value::Enum {
        variant: "Count".into(),
        fields: BTreeMap::from([("0".into(), integer(1))])
    },
    Value::Enum {
        variant: "Count".into(),
        fields: BTreeMap::from([("0".into(), integer(2))])
    },
    Fail
);
equality!(
    nested_struct_equal,
    object(&[("inner", object(&[("n", integer(2))]))]),
    object(&[("inner", object(&[("n", integer(2))]))]),
    Pass
);
equality!(
    vector_equal,
    Value::Sequence(vec![integer(0), integer(2)]),
    Value::Sequence(vec![integer(0), integer(2)]),
    Pass
);
equality!(
    sequence_order_change,
    Value::Sequence(vec![integer(0), integer(2)]),
    Value::Sequence(vec![integer(2), integer(0)]),
    Fail
);
equality!(
    option_equal,
    Value::Option(Some(Box::new(integer(2)))),
    Value::Option(Some(Box::new(integer(2)))),
    Pass
);
equality!(
    option_none_to_some,
    Value::Option(None),
    Value::Option(Some(Box::new(integer(0)))),
    Fail
);
equality!(bool_equal, Value::Bool(false), Value::Bool(false), Pass);
equality!(
    string_equal,
    Value::String("hello".into()),
    Value::String("hello".into()),
    Pass
);
equality!(
    no_integer_string_coercion,
    integer(1),
    Value::String("1".into()),
    Fail
);

#[test]
fn field_top_level() {
    assert_eq!(
        resolve_field(&account(integer(7)), "value"),
        Resolution::Found(integer(7))
    );
}
#[test]
fn field_nested() {
    assert_eq!(
        resolve_field(&account(object(&[("n", integer(3))])), "value.n"),
        Resolution::Found(integer(3))
    );
}
#[test]
fn field_deep_nested() {
    assert_eq!(
        resolve_field(
            &account(object(&[("inner", object(&[("n", integer(4))]))])),
            "value.inner.n"
        ),
        Resolution::Found(integer(4))
    );
}
#[test]
fn field_missing() {
    assert_eq!(
        resolve_field(&account(integer(7)), "missing"),
        Resolution::Missing
    );
}
#[test]
fn field_non_struct() {
    assert_eq!(
        resolve_field(&account(integer(7)), "value.n"),
        Resolution::TypeMismatch
    );
}
#[test]
fn field_invalid_path() {
    for p in [
        "",
        ".value",
        "value.",
        "value..n",
        "value[0]",
        "value + 1",
        " value",
    ] {
        assert_eq!(
            resolve_field(&account(integer(7)), p),
            Resolution::InvalidPath
        );
    }
}
#[test]
fn field_absent_account() {
    let mut a = account(integer(7));
    a.snapshot = None;
    assert_eq!(resolve_field(&a, "value"), Resolution::AccountAbsent);
}
#[test]
fn not_equal_pass() {
    assert_eq!(
        status(
            &execution(integer(1), integer(2)),
            "rule='not_equal'\nfield='value'"
        ),
        Status::Pass
    );
}
#[test]
fn not_equal_fail() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='not_equal'\nfield='value'"
        ),
        Status::Fail
    );
}

#[test]
fn initialized_zero_passes() {
    let mut e = execution(integer(1), integer(1));
    fields_mut(&mut e.after_accounts[0]).insert("new".into(), integer(0));
    assert_eq!(status(&e, "field='new'\nrule='initialized'"), Status::Pass);
}
#[test]
fn initialized_missing_fails() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "field='new'\nrule='initialized'"
        ),
        Status::Fail
    );
}
#[test]
fn initialized_existing_is_not_applicable() {
    assert_eq!(
        status(
            &execution(integer(1), integer(2)),
            "field='value'\nrule='initialized'"
        ),
        Status::NotApplicable
    );
}
#[test]
fn initialized_new_account_passes() {
    let mut e = execution(integer(1), integer(0));
    e.before_accounts[0].snapshot = None;
    assert_eq!(
        status(&e, "field='value'\nrule='initialized'"),
        Status::Pass
    );
}
#[test]
fn initialized_undecodable_after_is_error_even_if_before_exists() {
    let mut e = execution(integer(1), integer(2));
    e.after_accounts[0].decoded = None;
    assert_eq!(
        status(&e, "field='value'\nrule='initialized'"),
        Status::Error
    );
}

#[test]
fn default_matches() {
    assert_eq!(
        status(
            &execution(integer(1), integer(2)),
            "field='value'\nrule='default'\nvalue='u128:2'"
        ),
        Status::Pass
    );
}
#[test]
fn default_mismatch_has_expected_evidence() {
    let r = report(
        &execution(integer(1), integer(2)),
        "field='value'\nrule='default'\nvalue='u128:3'",
    );
    assert_eq!(r.failed, 1);
    assert_eq!(r.findings[0].evaluation.expected, Some(integer(3)));
}
#[test]
fn default_new_field_ignores_missing_before() {
    let mut e = execution(integer(1), integer(2));
    fields_mut(&mut e.before_accounts[0]).clear();
    assert_eq!(
        status(&e, "field='value'\nrule='default'\nvalue=2"),
        Status::Pass
    );
}
#[test]
fn default_enum_uses_existing_codec() {
    let value = Value::Enum {
        variant: "Count".into(),
        fields: BTreeMap::from([("0".into(), integer(9))]),
    };
    let e = execution(value.clone(), value);
    let mut m = model();
    m.accounts.get_mut("Record").unwrap().fields[0].ty = FieldType::Defined("Choice".into());
    let r = evaluate(
        &e,
        &[rule(
            "field='value'\nrule='default'\nvalue={variant='Count',fields=[9]}",
        )],
        &m,
    )
    .unwrap();
    assert_eq!(r.passed, 1);
}
#[test]
fn default_nested_field_uses_schema() {
    let mut e = execution(integer(1), integer(1));
    fields_mut(&mut e.after_accounts[0])
        .insert("config".into(), object(&[("flag", Value::Bool(false))]));
    assert_eq!(
        status(&e, "field='config.flag'\nrule='default'\nvalue=false"),
        Status::Pass
    );
}
#[test]
fn default_out_of_range_is_error() {
    let r = report(
        &execution(integer(1), integer(1)),
        "field='value'\nrule='default'\nvalue='u128:340282366920938463463374607431768211456'",
    );
    assert_eq!(r.errors, 1);
    assert_eq!(
        r.evaluations[0].error_code.as_deref(),
        Some("EXPECTED_VALUE")
    );
}

macro_rules! numeric {
    ($name:ident,$actual:expr,$kind:literal,$expected:literal,$status:ident) => {
        #[test]
        fn $name() {
            assert_eq!(
                status(
                    &execution(integer(0), $actual),
                    concat!("field='value'\nrule='", $kind, "'\nvalue=", $expected)
                ),
                Status::$status
            );
        }
    };
}
numeric!(minimum_equal, integer(10), "greater_or_equal", "10", Pass);
numeric!(minimum_above, integer(11), "greater_or_equal", "10", Pass);
numeric!(minimum_below, integer(9), "greater_or_equal", "10", Fail);
numeric!(maximum_equal, integer(10), "less_or_equal", "10", Pass);
numeric!(maximum_below, integer(9), "less_or_equal", "10", Pass);
numeric!(maximum_above, integer(11), "less_or_equal", "10", Fail);
numeric!(
    u128_max_boundary,
    Value::Integer(u128::MAX.into()),
    "greater_or_equal",
    "'u128:340282366920938463463374607431768211455'",
    Pass
);
numeric!(
    u128_max_default,
    Value::Integer(u128::MAX.into()),
    "default",
    "'u128:340282366920938463463374607431768211455'",
    Pass
);
numeric!(
    wrong_numeric_prefix,
    integer(1),
    "greater_or_equal",
    "'u64:1'",
    Error
);
numeric!(
    non_numeric_value,
    Value::String("10".into()),
    "greater_or_equal",
    "10",
    Error
);

#[test]
fn u64_max_boundary() {
    let mut m = model();
    m.accounts.get_mut("Record").unwrap().fields[0].ty = FieldType::Primitive("u64".into());
    let r = evaluate(
        &execution(integer(0), Value::Integer(u64::MAX.into())),
        &[rule(
            "field='value'\nrule='less_or_equal'\nvalue='u64:18446744073709551615'",
        )],
        &m,
    )
    .unwrap();
    assert_eq!(r.passed, 1);
}
#[test]
fn signed_negative_boundary() {
    let mut m = model();
    m.accounts.get_mut("Record").unwrap().fields[0].ty = FieldType::Primitive("i128".into());
    let r=evaluate(&execution(integer(0),integer(i128::MIN)),&[rule("field='value'\nrule='greater_or_equal'\nvalue='i128:-170141183460469231731687303715884105728'")],&m).unwrap();
    assert_eq!(r.passed, 1);
}

#[test]
fn exists_account_present() {
    assert_eq!(
        status(&execution(integer(1), integer(1)), "rule='exists'"),
        Status::Pass
    );
}
#[test]
fn exists_account_absent() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot = None;
    assert_eq!(status(&e, "rule='exists'"), Status::Fail);
}
#[test]
fn exists_field_present() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='exists'\nfield='value'"
        ),
        Status::Pass
    );
}
#[test]
fn exists_field_missing() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='exists'\nfield='missing'"
        ),
        Status::Fail
    );
}
#[test]
fn not_exists_account_removed() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot = None;
    assert_eq!(status(&e, "rule='not_exists'"), Status::Pass);
}
#[test]
fn not_exists_account_remains() {
    assert_eq!(
        status(&execution(integer(1), integer(1)), "rule='not_exists'"),
        Status::Fail
    );
}
#[test]
fn not_exists_field_removed() {
    let mut e = execution(integer(1), integer(1));
    fields_mut(&mut e.after_accounts[0]).clear();
    assert_eq!(status(&e, "rule='not_exists'\nfield='value'"), Status::Pass);
}
#[test]
fn not_exists_field_remains() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='not_exists'\nfield='value'"
        ),
        Status::Fail
    );
}
#[test]
fn field_absence_does_not_hide_decode_failure() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].decoded = None;
    assert_eq!(
        status(&e, "rule='not_exists'\nfield='missing'"),
        Status::Error
    );
}

#[test]
fn owner_same() {
    assert_eq!(
        status(&execution(integer(1), integer(1)), "rule='owner_preserved'"),
        Status::Pass
    );
}
#[test]
fn owner_changed() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot.as_mut().unwrap().owner = "changed".into();
    assert_eq!(status(&e, "rule='owner_preserved'"), Status::Fail);
}
#[test]
fn owner_before_missing() {
    let mut e = execution(integer(1), integer(1));
    e.before_accounts[0].snapshot = None;
    assert_eq!(status(&e, "rule='owner_preserved'"), Status::Error);
}
#[test]
fn owner_after_missing() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot = None;
    assert_eq!(status(&e, "rule='owner_preserved'"), Status::Error);
}
#[test]
fn owner_metadata_not_decoded_owner() {
    let mut e = execution(integer(1), integer(1));
    fields_mut(&mut e.before_accounts[0]).insert("owner".into(), Value::Pubkey("A".into()));
    fields_mut(&mut e.after_accounts[0]).insert("owner".into(), Value::Pubkey("B".into()));
    let r = evaluate(
        &e,
        &[
            rule("rule='owner_preserved'"),
            rule("field='owner'\nrule='equal'"),
        ],
        &model(),
    )
    .unwrap();
    assert_eq!((r.passed, r.failed), (1, 1));
}
#[test]
fn lamports_same() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='lamports_preserved'"
        ),
        Status::Pass
    );
}
#[test]
fn lamports_increase() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot.as_mut().unwrap().lamports = 101.into();
    assert_eq!(status(&e, "rule='lamports_preserved'"), Status::Fail);
}
#[test]
fn lamports_decrease() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot.as_mut().unwrap().lamports = 99.into();
    assert_eq!(status(&e, "rule='lamports_preserved'"), Status::Fail);
}
#[test]
fn size_equal() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='account_size_minimum'\nvalue=48"
        ),
        Status::Pass
    );
}
#[test]
fn size_above() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='account_size_minimum'\nvalue=47"
        ),
        Status::Pass
    );
}
#[test]
fn size_below() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='account_size_minimum'\nvalue=49"
        ),
        Status::Fail
    );
}
#[test]
fn size_negative_invalid() {
    assert_eq!(
        status(
            &execution(integer(1), integer(1)),
            "rule='account_size_minimum'\nvalue=-1"
        ),
        Status::Error
    );
}
#[test]
fn pda_same() {
    assert_eq!(
        status(&execution(integer(1), integer(1)), "rule='pda_preserved'"),
        Status::Pass
    );
}
#[test]
fn pda_changed() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].address = "different".into();
    assert_eq!(status(&e, "rule='pda_preserved'"), Status::Fail);
}
#[test]
fn pda_binding_continuity_before_absent() {
    let mut e = execution(integer(1), integer(1));
    e.before_accounts[0].snapshot = None;
    assert_eq!(status(&e, "rule='pda_preserved'"), Status::Pass);
}
#[test]
fn pda_binding_continuity_after_absent() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot = None;
    assert_eq!(status(&e, "rule='pda_preserved'"), Status::Pass);
}

#[test]
fn decode_error_keeps_raw_and_allows_metadata_rules() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].decoded = None;
    e.after_accounts[0].decode_error = Some(StateError {
        code: "TRUNCATED".into(),
        path: "record".into(),
        offset: 2,
        message: "short data".into(),
    });
    let original = serde_json::to_vec(&e).unwrap();
    let rules = [
        rule("rule='equal'\nfield='value'"),
        rule("rule='owner_preserved'"),
        rule("rule='lamports_preserved'"),
        rule("rule='account_size_minimum'\nvalue=48"),
        rule("rule='pda_preserved'"),
        rule("rule='exists'"),
    ];
    let r = evaluate(&e, &rules, &model()).unwrap();
    assert_eq!((r.passed, r.errors), (5, 1));
    assert_eq!(serde_json::to_vec(&e).unwrap(), original);
    assert!(matches!(
        &r.evaluations[0].after,
        Some(Resolution::Undecodable(Some(_)))
    ));
}
#[test]
fn multiple_rules_collect_all_counts_and_evidence() {
    let e = execution(integer(1), integer(2));
    let rules = [
        rule("rule='equal'\nfield='value'"),
        rule("rule='not_equal'\nfield='value'"),
        rule("rule='equal'\nfield='missing'"),
        rule("rule='initialized'\nfield='value'"),
    ];
    let r = evaluate(&e, &rules, &model()).unwrap();
    assert_eq!(
        (
            r.rules_total,
            r.passed,
            r.failed,
            r.errors,
            r.not_applicable
        ),
        (4, 1, 1, 1, 1)
    );
    assert_eq!(r.findings.len(), 2);
    assert_eq!(
        r.findings[0].evaluation.before,
        Some(Resolution::Found(integer(1)))
    );
    assert_eq!(
        r.findings[0].evaluation.after,
        Some(Resolution::Found(integer(2)))
    );
}
#[test]
fn deterministic_report_and_stable_generated_ids() {
    let e = execution(integer(1), integer(2));
    let rules = [rule("rule='equal'\nfield='value'"), rule("rule='exists'")];
    let a = evaluate(&e, &rules, &model()).unwrap();
    let b = evaluate(&e, &rules, &model()).unwrap();
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    let reversed = evaluate(&e, &[rules[1].clone(), rules[0].clone()], &model()).unwrap();
    assert_eq!(a.evaluations[0].rule_id, reversed.evaluations[1].rule_id);
    assert_eq!(a.findings[0].id, reversed.findings[0].id);
}
#[test]
fn explicit_id_preserved() {
    let r = report(
        &execution(integer(1), integer(2)),
        "id='custom'\nrule='equal'\nfield='value'",
    );
    assert_eq!(r.evaluations[0].rule_id, "custom");
    assert_eq!(r.findings[0].id, "PRESERVE-custom");
}
#[test]
fn unknown_logical_account_is_diagnostic() {
    let mut r = rule("rule='exists'");
    r.account = "unknown".into();
    let p = evaluate(&execution(integer(1), integer(1)), &[r], &model()).unwrap();
    assert_eq!(
        p.evaluations[0].error_code.as_deref(),
        Some("UNKNOWN_ACCOUNT")
    );
}
#[test]
fn missing_side_is_not_absent_account() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts.clear();
    let p = report(&e, "rule='not_exists'");
    assert_eq!(
        p.evaluations[0].error_code.as_deref(),
        Some("TARGET_MISSING")
    );
}
#[test]
fn ambiguous_binding_rejected() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts.push(e.after_accounts[0].clone());
    let p = report(&e, "rule='exists'");
    assert_eq!(
        p.evaluations[0].error_code.as_deref(),
        Some("AMBIGUOUS_ACCOUNT")
    );
}
#[test]
fn no_rules_does_not_invent_static_requirements() {
    let r = evaluate(&execution(integer(1), integer(2)), &[], &model()).unwrap();
    assert_eq!(r.rules_total, 0);
    assert!(r.findings.is_empty());
    assert!(!r.policy_evaluated && !r.lifecycle_evaluated);
}

macro_rules! bad_config {
    ($name:ident,$text:literal) => {
        #[test]
        fn $name() {
            let parsed = toml::from_str::<PreservationRule>($text);
            match parsed {
                Err(_) => {}
                Ok(r) => {
                    assert!(evaluate(&execution(integer(1), integer(1)), &[r], &model()).is_err())
                }
            }
        }
    };
}
bad_config!(unknown_rule_type, "account='record'\nrule='script'");
bad_config!(missing_account_config, "rule='exists'");
bad_config!(empty_account_config, "account=''\nrule='exists'");
bad_config!(
    invalid_account_reference,
    "account='$record'\nrule='exists'"
);
bad_config!(missing_required_field, "account='record'\nrule='equal'");
bad_config!(
    unexpected_field,
    "account='record'\nfield='value'\nrule='owner_preserved'"
);
bad_config!(
    missing_expected,
    "account='record'\nfield='value'\nrule='default'"
);
bad_config!(
    unexpected_expected,
    "account='record'\nfield='value'\nrule='equal'\nvalue=1"
);
bad_config!(
    invalid_field_path_config,
    "account='record'\nfield='value..inner'\nrule='equal'"
);
#[test]
fn invalid_integer_is_structured_error() {
    assert_eq!(
        report(
            &execution(integer(1), integer(1)),
            "field='value'\nrule='default'\nvalue='u128:garbage'"
        )
        .errors,
        1
    );
}
#[test]
fn duplicate_explicit_ids_rejected() {
    let r = rule("id='same'\nrule='exists'");
    assert!(evaluate(
        &execution(integer(1), integer(1)),
        &[r.clone(), r],
        &model()
    )
    .is_err());
}
#[test]
fn duplicate_generated_ids_rejected() {
    let r = rule("rule='exists'");
    assert!(evaluate(
        &execution(integer(1), integer(1)),
        &[r.clone(), r],
        &model()
    )
    .is_err());
}

#[test]
fn initialized_cannot_treat_undecodable_before_as_missing() {
    let mut e = execution(integer(1), integer(1));
    e.before_accounts[0].decoded = None;
    assert_eq!(
        status(&e, "field='value'\nrule='initialized'"),
        Status::Error
    );
}

#[test]
fn after_only_default_does_not_require_before_decoding() {
    let mut e = execution(integer(1), integer(2));
    e.before_accounts[0].decoded = None;
    assert_eq!(
        status(&e, "field='value'\nrule='default'\nvalue=2"),
        Status::Pass
    );
}

#[test]
fn field_absence_does_not_hide_missing_account() {
    let mut e = execution(integer(1), integer(1));
    e.after_accounts[0].snapshot = None;
    let result = report(&e, "field='value'\nrule='not_exists'");
    assert_eq!(result.errors, 1);
    assert_eq!(
        result.evaluations[0].error_code.as_deref(),
        Some("ACCOUNT_MISSING")
    );
}

#[test]
fn compound_defaults_reuse_lossless_encoder_and_decoder() {
    let cases = [
        (
            FieldType::Primitive("bytes".into()),
            Value::Sequence(vec![integer(0), integer(255)]),
            "[0,255]",
        ),
        (
            FieldType::Array(Box::new(FieldType::Primitive("u8".into())), 2),
            Value::Sequence(vec![integer(0), integer(255)]),
            "[0,255]",
        ),
        (
            FieldType::Tuple(vec![
                FieldType::Primitive("u8".into()),
                FieldType::Primitive("u16".into()),
            ]),
            Value::Sequence(vec![integer(1), integer(65535)]),
            "[1,65535]",
        ),
        (
            FieldType::Option(Box::new(FieldType::Primitive("u64".into()))),
            Value::Option(Some(Box::new(Value::Integer(u64::MAX.into())))),
            "{some='u64:18446744073709551615'}",
        ),
        (
            FieldType::Vec(Box::new(FieldType::Primitive("u64".into()))),
            Value::Sequence(vec![Value::Integer(u64::MAX.into())]),
            "['u64:18446744073709551615']",
        ),
        (
            FieldType::Defined("Config".into()),
            object(&[("flag", Value::Bool(true))]),
            "{flag=true}",
        ),
    ];
    for (ty, value, expected) in cases {
        let mut m = model();
        m.accounts.get_mut("Record").unwrap().fields[0].ty = ty;
        let e = execution(value.clone(), value);
        let r = evaluate(
            &e,
            &[rule(&format!(
                "rule='default'\nfield='value'\nvalue={expected}"
            ))],
            &m,
        )
        .unwrap();
        assert_eq!(r.passed, 1, "{:?}", r.evaluations);
    }
}
