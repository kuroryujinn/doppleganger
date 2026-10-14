use dg_analysis::size::{estimate_account, SizeKind};
use dg_analysis::{analyze, normalize};
use serde_json::{json, Value};

fn fixture() -> Value {
    json!({
        "address":"Program111", "metadata":{"name":"fixture","version":"1.0"},
        "instructions":[{"name":"process","discriminator":[1],
            "accounts":[{"name":"state","writable":true,"pda":{"seeds":[{"kind":"const","value":[1]},{"kind":"account","path":"authority"}]}},
                        {"name":"authority","signer":true}],
            "args":[{"name":"amount","type":"u64"},{"name":"flag","type":"bool"}]}],
        "accounts":[{"name":"State","discriminator":[2]}],
        "types":[{"name":"State","type":{"kind":"struct","fields":[
            {"name":"owner","type":"pubkey"},{"name":"amount","type":"u64"},
            {"name":"status","type":{"defined":{"name":"Status"}}}]}},
            {"name":"Status","type":{"kind":"enum","variants":[{"name":"Open"},{"name":"Closed"}]}},
            {"name":"Log","type":{"kind":"struct","fields":[{"name":"owner","type":"pubkey"},{"name":"amount","type":"u64"}]}}],
        "events":[{"name":"Log","discriminator":[3]}],
        "errors":[{"name":"Failure","code":6000,"msg":"failed"}]
    })
}

fn model(v: &Value) -> dg_analysis::model::ProgramModel {
    normalize(v.to_string().as_bytes()).unwrap()
}
fn compare(a: &Value, b: &Value) -> dg_analysis::AnalysisReport {
    analyze("fixture", &model(a), &model(b), &[])
}
fn has(report: &dg_analysis::AnalysisReport, class: &str, path: &str) {
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.classification == class && f.path.contains(path)),
        "missing {class} {path}: {:#?}",
        report.findings
    );
}

#[test]
fn normalization_is_deterministic_and_rejects_bad_input() {
    let a = fixture();
    assert_eq!(model(&a), model(&a));
    assert!(normalize(b"not json").is_err());
    assert!(compare(&a, &a).findings.is_empty());
    let mut bad = a.clone();
    bad["instructions"][0]["accounts"][0]["pda"]["seeds"][0]["kind"] = json!("random");
    assert!(normalize(bad.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("unsupported PDA"));
    let mut bad = a.clone();
    let duplicate = bad["types"][0].clone();
    bad["types"].as_array_mut().unwrap().push(duplicate);
    assert!(normalize(bad.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    let mut bad = a.clone();
    bad["types"][0]["type"]["fields"][0]["type"] = json!({"defined":{"name":"Missing"}});
    assert!(normalize(bad.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("unresolved"));
    let mut bad = a.clone();
    bad["types"][0]["type"]["fields"][0]["type"] = json!({"defined":{"name":"State"}});
    assert!(normalize(bad.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("cyclic"));
    let mut bad = a.clone();
    bad["types"].as_array_mut().unwrap().remove(0);
    assert!(normalize(bad.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("missing account type"));
    let mut bad = a.clone();
    bad["types"][1]["type"]["variants"] = json!([{"name":"Open"},{"name":"Open"}]);
    assert!(normalize(bad.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("duplicate enum"));
}

#[test]
fn instruction_changes_are_independent() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["args"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    b["instructions"][0]["args"][0]["type"] = json!("u32");
    b["instructions"][0]["accounts"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    b["instructions"][0]["accounts"][0]["signer"] = json!(false);
    b["instructions"][0]["accounts"][0]["writable"] = json!(true);
    b["instructions"][0]["accounts"][0]["optional"] = json!(true);
    let report = compare(&a, &b);
    has(&report, "INTERFACE_CHANGE", "args.order");
    has(&report, "TYPE_COMPATIBILITY_CHANGE", "args.flag");
    has(&report, "INTERFACE_CHANGE", "accounts.order");
    has(&report, "INTERFACE_CHANGE", "authority.signer");
    has(&report, "INTERFACE_CHANGE", "authority.writable");
    has(&report, "INTERFACE_CHANGE", "authority.optional");
    let mut b = a.clone();
    b["instructions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"extra","accounts":[],"args":[],"discriminator":[9]}));
    has(&compare(&a, &b), "INTERFACE_CHANGE", "extra");
    has(&compare(&b, &a), "INTERFACE_CHANGE", "extra");
    let mut b = a.clone();
    b["instructions"][0]["args"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"new","type":"u8"}));
    b["instructions"][0]["accounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"extra"}));
    has(&compare(&a, &b), "INTERFACE_CHANGE", "args.new");
    has(&compare(&b, &a), "INTERFACE_CHANGE", "args.new");
    has(&compare(&a, &b), "INTERFACE_CHANGE", "accounts.extra");
    has(&compare(&b, &a), "INTERFACE_CHANGE", "accounts.extra");
}

#[test]
fn account_fields_size_and_dynamic_layout() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"limit","type":"u64"}));
    let report = compare(&a, &b);
    has(&report, "ACCOUNT_SCHEMA_CHANGE", "limit");
    has(&report, "ACCOUNT_SIZE_CHANGE", "size");
    assert_eq!(
        report.account_sizes["State"][0].as_ref().unwrap().bytes,
        Some(42)
    );
    assert_eq!(
        report.account_sizes["State"][1].as_ref().unwrap().bytes,
        Some(50)
    );
    let reduced = compare(&b, &a);
    has(&reduced, "ACCOUNT_SIZE_CHANGE", "size");
    let mut changed = a.clone();
    changed["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    changed["types"][0]["type"]["fields"][0]["type"] = json!("u32");
    has(
        &compare(&a, &changed),
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "order",
    );
    has(
        &compare(&a, &changed),
        "TYPE_COMPATIBILITY_CHANGE",
        "amount",
    );
    let mut dynamic = a.clone();
    dynamic["types"][0]["type"]["fields"][1]["type"] = json!({"vec":"u8"});
    let state = model(&dynamic);
    let size = estimate_account(&state.accounts["State"], &state.types);
    assert_eq!(size.kind, SizeKind::Dynamic);
    assert_eq!(size.bytes, None);
}

#[test]
fn enum_indexes_payloads_and_nested_types() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][1]["type"]["variants"]
        .as_array_mut()
        .unwrap()
        .insert(1, json!({"name":"Paused","fields":["u8"]}));
    let report = compare(&a, &b);
    has(
        &report,
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "Status.Closed.index",
    );
    has(
        &report,
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "Status.Paused",
    );
    let mut changed = a.clone();
    changed["types"][1]["type"]["variants"][0]["fields"] = json!(["u64"]);
    has(
        &compare(&a, &changed),
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "Status.Open.0",
    );
}

#[test]
fn pda_and_discriminator_changes() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]["pda"]["seeds"][0]["value"] = json!([2]);
    b["instructions"][0]["accounts"][0]["pda"]["seeds"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"arg","path":"amount"}));
    b["accounts"][0]["discriminator"] = json!([4]);
    b["instructions"][0]["discriminator"] = json!([5]);
    let report = compare(&a, &b);
    has(&report, "PDA_DERIVATION_CHANGE", "seed_count");
    has(&report, "PDA_DERIVATION_CHANGE", "seeds[0]");
    has(&report, "DISCRIMINATOR_CHANGE", "State.discriminator");
    has(&report, "DISCRIMINATOR_CHANGE", "process.discriminator");
    let mut b = a.clone();
    b["accounts"][0]
        .as_object_mut()
        .unwrap()
        .remove("discriminator");
    has(
        &compare(&a, &b),
        "DISCRIMINATOR_CHANGE",
        "State.discriminator",
    );
}

#[test]
fn account_add_remove_nested_type_and_field_removal() {
    let a = fixture();
    let mut b = a.clone();
    b["accounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"Config","discriminator":[4]}));
    b["types"].as_array_mut().unwrap().push(
        json!({"name":"Config","type":{"kind":"struct","fields":[{"name":"bump","type":"u8"}]}}),
    );
    has(&compare(&a, &b), "ACCOUNT_SCHEMA_CHANGE", "Config");
    has(&compare(&b, &a), "ACCOUNT_SCHEMA_CHANGE", "Config");
    let mut b = a.clone();
    b["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    has(&compare(&a, &b), "ACCOUNT_SCHEMA_CHANGE", "State.amount");
    let mut b = a.clone();
    b["types"][1]["type"]["variants"][0]["fields"] = json!(["u64"]);
    has(&compare(&a, &b), "ACCOUNT_SIZE_CHANGE", "State.size");
}

#[test]
fn pda_seed_sources_order_removal_and_program_override() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]["pda"]["seeds"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    has(&compare(&a, &b), "PDA_DERIVATION_CHANGE", "seeds[0]");
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]["pda"]["seeds"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    has(&compare(&a, &b), "PDA_DERIVATION_CHANGE", "seed_count");
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]["pda"]["seeds"][1] = json!({"kind":"arg","path":"amount"});
    b["instructions"][0]["accounts"][0]["pda"]["program"] = json!({"kind":"const","value":[9]});
    has(&compare(&a, &b), "PDA_DERIVATION_CHANGE", "seeds[1]");
    has(&compare(&a, &b), "PDA_DERIVATION_CHANGE", "program");
}

#[test]
fn event_field_and_discriminator_variants() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][2]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    b["types"][2]["type"]["fields"][0]["type"] = json!("u32");
    b["types"][2]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"slot","type":"u64"}));
    b["events"][0]["discriminator"] = json!([9]);
    has(&compare(&a, &b), "EVENT_SCHEMA_CHANGE", "order");
    has(&compare(&a, &b), "EVENT_SCHEMA_CHANGE", "amount");
    has(&compare(&a, &b), "EVENT_SCHEMA_CHANGE", "slot");
    has(
        &compare(&a, &b),
        "DISCRIMINATOR_CHANGE",
        "Log.discriminator",
    );
    let mut b = a.clone();
    b["events"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"Other","fields":[{"name":"value","type":"u8"}]}));
    has(&compare(&a, &b), "EVENT_SCHEMA_CHANGE", "Other");
}

#[test]
fn malformed_and_duplicate_idl_definitions_fail_cleanly() {
    let a = fixture();
    assert!(normalize(b"{}")
        .unwrap_err()
        .to_string()
        .contains("instructions"));
    let mut b = a.clone();
    let instruction = b["instructions"][0].clone();
    b["instructions"].as_array_mut().unwrap().push(instruction);
    assert!(normalize(b.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    let mut b = a.clone();
    let account = b["accounts"][0].clone();
    b["accounts"].as_array_mut().unwrap().push(account);
    assert!(normalize(b.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    let mut b = a.clone();
    b["types"][1]["type"]["variants"] = json!([]);
    assert!(normalize(b.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("enum needs"));
    let mut b = a.clone();
    b["types"][0]["type"]["fields"][0]["type"] = json!("nonsense");
    assert!(normalize(b.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("unsupported primitive"));
    let mut b = a.clone();
    b["types"][0]["type"]["fields"][0]["type"] =
        json!({"defined":{"name":"Status","generics":["u8"]}});
    assert!(normalize(b.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("generic defined"));
}

#[test]
fn events_errors_and_deterministic_test_matrix() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][2]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    b["errors"][0]["code"] = json!(6001);
    b["errors"][0]["msg"] = json!("changed");
    let report = compare(&a, &b);
    has(&report, "EVENT_SCHEMA_CHANGE", "Log.amount");
    has(&report, "ERROR_CODE_CHANGE", "Failure.code");
    has(&report, "ERROR_MESSAGE_CHANGE", "Failure.message");
    assert!(!report.suggested_runtime_tests.is_empty());
    assert_eq!(
        serde_json::to_vec(&report).unwrap(),
        serde_json::to_vec(&compare(&a, &b)).unwrap()
    );
    assert_eq!(report.findings[0].id, "STATIC-0001");
    let mut b = a.clone();
    b["events"].as_array_mut().unwrap().clear();
    b["errors"].as_array_mut().unwrap().clear();
    has(&compare(&a, &b), "EVENT_SCHEMA_CHANGE", "Log");
    has(&compare(&a, &b), "ERROR_DEFINITION_CHANGE", "Failure");
}

#[test]
fn legacy_account_flags_normalize_and_malformed_flags_fail() {
    let mut idl = fixture();
    let account = idl["instructions"][0]["accounts"][1]
        .as_object_mut()
        .unwrap();
    account.remove("signer");
    account.insert("isSigner".into(), json!(true));
    account.insert("isMut".into(), json!(true));
    account.insert("isOptional".into(), json!(true));
    let normalized = model(&idl);
    let authority = &normalized.instructions["process"].accounts[1];
    assert!(authority.signer && authority.writable && authority.optional);
    idl["instructions"][0]["accounts"][1]["isMut"] = json!("yes");
    assert!(normalize(idl.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("writable must be boolean"));
}

#[test]
fn narrowing_hint_and_test_matrix_are_specific_and_deduplicated() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][0]["type"]["fields"][1]["type"] = json!("u32");
    let report = compare(&a, &b);
    let narrowing = report
        .findings
        .iter()
        .find(|f| {
            f.path.ends_with("State.amount") && f.classification == "TYPE_COMPATIBILITY_CHANGE"
        })
        .unwrap();
    assert!(narrowing.migration_hints[0]
        .suggested_test
        .contains("u32::MAX"));
    let mut b = a.clone();
    b["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .extend([
            json!({"name":"limit","type":"u64"}),
            json!({"name":"nonce","type":"u64"}),
        ]);
    let report = compare(&a, &b);
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| f.classification == "ACCOUNT_SCHEMA_CHANGE")
            .count(),
        2
    );
    assert_eq!(
        report
            .suggested_runtime_tests
            .iter()
            .filter(|t| t.area == "account" && t.target == "State")
            .count(),
        1
    );
    assert_eq!(report.normalized_changes.len(), report.findings.len());
}

#[test]
fn existing_enum_variants_reordered_change_serialized_indexes() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][1]["type"]["variants"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let report = compare(&a, &b);
    has(
        &report,
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "Status.Open.index",
    );
    has(
        &report,
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "Status.Closed.index",
    );
    assert_eq!(
        report.account_sizes["State"][0].as_ref().unwrap().bytes,
        Some(42)
    );
    assert_eq!(
        report.account_sizes["State"][1].as_ref().unwrap().bytes,
        Some(42)
    );
}

#[test]
fn strings_and_vectors_never_claim_exact_account_size() {
    for dynamic_type in [json!("string"), json!({"vec":"u8"})] {
        let a = fixture();
        let mut b = a.clone();
        b["types"][0]["type"]["fields"][1]["type"] = dynamic_type;
        let report = compare(&a, &b);
        let estimate = report.account_sizes["State"][1].as_ref().unwrap();
        assert_eq!(estimate.kind, SizeKind::Dynamic);
        assert_eq!(estimate.bytes, None);
        assert!(estimate.minimum_bytes.is_some());
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["account_sizes"]["State"][1]["kind"], "dynamic");
        assert!(json["account_sizes"]["State"][1]["bytes"].is_null());
    }
}

#[test]
fn missing_pda_metadata_is_distinct_from_malformed_pda_metadata() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]
        .as_object_mut()
        .unwrap()
        .remove("pda");
    has(&compare(&a, &b), "PDA_DERIVATION_CHANGE", "state.pda");
    let mut without_pda = a.clone();
    without_pda["instructions"][0]["accounts"][0]
        .as_object_mut()
        .unwrap()
        .remove("pda");
    assert!(compare(&without_pda, &without_pda).findings.is_empty());
    let mut malformed = a.clone();
    malformed["instructions"][0]["accounts"][0]["pda"] = json!({});
    assert!(normalize(malformed.to_string().as_bytes())
        .unwrap_err()
        .to_string()
        .contains("missing seeds"));
}

#[test]
fn explicit_account_mapping_works_for_unrelated_names() {
    let mut a = fixture();
    let mut b = fixture();
    a["accounts"][0]["name"] = json!("LedgerV1");
    a["types"][0]["name"] = json!("LedgerV1");
    b["accounts"][0]["name"] = json!("LedgerV2");
    b["types"][0]["name"] = json!("LedgerV2");
    b["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"sequence","type":"u64"}));
    let report = analyze(
        "ledger",
        &model(&a),
        &model(&b),
        &[("LedgerV1".into(), "LedgerV2".into())],
    );
    has(&report, "ACCOUNT_SCHEMA_CHANGE", "sequence");
    has(&report, "ACCOUNT_SIZE_CHANGE", "size");
    assert_eq!(
        report.account_sizes["LedgerV1 -> LedgerV2"][0]
            .as_ref()
            .unwrap()
            .bytes,
        Some(42)
    );
    assert_eq!(
        report.account_sizes["LedgerV1 -> LedgerV2"][1]
            .as_ref()
            .unwrap()
            .bytes,
        Some(50)
    );
    assert!(!report
        .findings
        .iter()
        .any(|f| f.explanation == "Account type removed" || f.explanation == "Account type added"));
}

#[test]
fn report_exposes_typed_changes_without_parsing_explanations() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .extend((0..5).map(|n| json!({"name":format!("extra_{n}"),"type":"u64"})));
    let report = compare(&a, &b);
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| f.classification == "ACCOUNT_SCHEMA_CHANGE")
            .count(),
        5
    );
    assert_eq!(
        report
            .suggested_runtime_tests
            .iter()
            .filter(|t| t.area == "account" && t.target == "State")
            .count(),
        1
    );
    let json = serde_json::to_value(&report).unwrap();
    let decoded: dg_analysis::AnalysisReport = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(decoded, report);
    assert_eq!(json["format_version"], 1);
    assert_eq!(json["runtime_executed"], false);
    assert_eq!(json["account_sizes"]["State"][0]["bytes"], 42);
    assert_eq!(json["account_sizes"]["State"][1]["bytes"], 82);
    let change = json["normalized_changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == "account.State.extra_0")
        .unwrap();
    assert_eq!(change["classification"], "ACCOUNT_SCHEMA_CHANGE");
    assert!(change["before"].is_null());
    assert_eq!(change["after"]["kind"], "primitive");
    assert_eq!(change["after"]["value"], "u64");
    assert!(json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["id"].as_str().unwrap().starts_with("STATIC-")));
}

#[test]
fn signer_requirement_change_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][1]["signer"] = json!(false);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "INTERFACE_CHANGE", "authority.signer");
}

#[test]
fn writable_requirement_change_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][1]["writable"] = json!(true);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "INTERFACE_CHANGE", "authority.writable");
}

#[test]
fn argument_reorder_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["args"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "INTERFACE_CHANGE", "args.order");
}

#[test]
fn argument_type_change_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["args"][0]["type"] = json!("u32");
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "TYPE_COMPATIBILITY_CHANGE", "args.amount");
}

#[test]
fn account_field_reorder_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(
        &report,
        "SERIALIZATION_COMPATIBILITY_CHANGE",
        "account.State.order",
    );
}

#[test]
fn account_field_type_change_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][0]["type"]["fields"][1]["type"] = json!("u32");
    let report = compare(&a, &b);
    has(&report, "TYPE_COMPATIBILITY_CHANGE", "account.State.amount");
    has(&report, "ACCOUNT_SIZE_CHANGE", "account.State.size");
}

#[test]
fn pda_seed_order_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]["pda"]["seeds"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 2);
    has(&report, "PDA_DERIVATION_CHANGE", "seeds[0]");
    has(&report, "PDA_DERIVATION_CHANGE", "seeds[1]");
}

#[test]
fn pda_seed_source_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["instructions"][0]["accounts"][0]["pda"]["seeds"][1] = json!({"kind":"arg","path":"amount"});
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "PDA_DERIVATION_CHANGE", "seeds[1]");
}

#[test]
fn account_discriminator_change_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["accounts"][0]["discriminator"] = json!([9]);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "DISCRIMINATOR_CHANGE", "State.discriminator");
}

#[test]
fn event_field_removal_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["types"][2]["type"]["fields"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    let report = compare(&a, &b);
    has(&report, "EVENT_SCHEMA_CHANGE", "event.Log.amount");
}

#[test]
fn error_code_change_is_detected_alone() {
    let a = fixture();
    let mut b = a.clone();
    b["errors"][0]["code"] = json!(6001);
    let report = compare(&a, &b);
    assert_eq!(report.findings.len(), 1);
    has(&report, "ERROR_CODE_CHANGE", "Failure.code");
}
