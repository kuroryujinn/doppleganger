mod r6_support;
use anchor_lang::InstructionData;
use dg_config::MigrationTarget;
use dg_runtime::{migration::MigrationRequest, ExecutionErrorKind, MigrationExecutor};
use r6_support::*;
use solana_signer::Signer;

#[test]
fn candidate_load_preserves_committed_v1_state_before_transaction() {
    let mut w = world();
    let address = w.resolved.addresses["record"];
    let before = w.runtime.get_account(&address).unwrap();
    assert_eq!(&before.data[40..48], &42u64.to_le_bytes());
    let owner = w
        .runtime
        .get_account(&w.resolved.addresses["owner"])
        .unwrap();
    let swap = MigrationExecutor::default()
        .load_candidate(&mut w, &artifact(true), &model(true))
        .unwrap();
    assert!(swap.loaded);
    assert_eq!(swap.previous.program_id, swap.candidate.program_id);
    assert_eq!(w.runtime.get_account(&address).unwrap(), before);
    assert_eq!(
        w.runtime
            .get_account(&w.resolved.addresses["owner"])
            .unwrap(),
        owner
    );
}

#[test]
fn missing_binary_is_structured() {
    let mut w = world();
    let mut candidate = artifact(true);
    candidate.program_path = root().join("target/does-not-exist-r6.so");
    let e = MigrationExecutor::default()
        .load_candidate(&mut w, &candidate, &model(true))
        .unwrap_err();
    assert_eq!(e.code, "MISSING_PROGRAM");
    assert_eq!(w.artifact, artifact(false));
}

fn invalid_binary(bytes: &[u8], code: &str) {
    let mut w = world();
    let address = w.resolved.addresses["record"];
    let before = w.runtime.get_account(&address).unwrap();
    let path = std::env::temp_dir().join(format!("dg-r6-{}-{code}.so", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let mut candidate = artifact(true);
    candidate.program_path = path.clone();
    let e = MigrationExecutor::default()
        .load_candidate(&mut w, &candidate, &model(true))
        .unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert_eq!(e.code, code);
    assert_eq!(w.artifact, artifact(false));
    assert_eq!(w.runtime.get_account(&address).unwrap(), before);
    assert!(migrate(&mut w).transaction.unwrap().success);
}
#[test]
fn non_elf_rejected_without_altering_world() {
    invalid_binary(b"not an ELF", "INVALID_PROGRAM");
}
#[test]
fn invalid_elf_load_rejected_without_altering_world() {
    invalid_binary(b"\x7fELFbroken", "PROGRAM_LOAD");
}

#[test]
fn different_program_id_rejected() {
    let mut w = world();
    let mut candidate = artifact(true);
    candidate.program_id = "11111111111111111111111111111111".into();
    assert_eq!(
        MigrationExecutor::default()
            .load_candidate(&mut w, &candidate, &model(true))
            .unwrap_err()
            .code,
        "PROGRAM_ID"
    );
}
#[test]
fn candidate_idl_id_mismatch_rejected() {
    let mut w = world();
    let mut m = model(true);
    m.address = Some("11111111111111111111111111111111".into());
    assert_eq!(
        run(&mut w, &definition("migrate"), &m, &targets())
            .unwrap_err()
            .code,
        "PROGRAM_ID"
    );
}

#[test]
fn instruction_order_flags_and_data_match_independent_anchor_encoding() {
    let mut w = world();
    let expected = ["record", "owner", "system"].map(|name| w.resolved.addresses[name].to_string());
    let r = migrate(&mut w).transaction.unwrap();
    assert_eq!(
        r.accounts
            .iter()
            .map(|a| a.address.as_str())
            .collect::<Vec<_>>(),
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(r.accounts[0].writable && !r.accounts[0].signer);
    assert!(r.accounts[1].writable && r.accounts[1].signer);
    assert!(!r.accounts[2].writable && !r.accounts[2].signer);
    assert_eq!(
        r.instruction_data,
        dg_r6_fixture_v2::instruction::Migrate {
            mode: 0,
            version: 2,
            value: 42
        }
        .data()
    );
}
#[test]
fn configured_discriminator_is_used_even_when_program_rejects_it() {
    let mut w = world();
    let mut m = model(true);
    m.instructions.get_mut("migrate").unwrap().discriminator = Some(vec![17; 8]);
    let e = run(&mut w, &definition("migrate"), &m, &targets()).unwrap();
    let r = e.transaction.unwrap();
    assert_eq!(&r.instruction_data[..8], &[17; 8]);
    assert_eq!(r.error_kind, Some(ExecutionErrorKind::Program));
    assert!(e.after_accounts[0].snapshot.is_some());
}
#[test]
fn missing_instruction_rejected_with_before_evidence() {
    let e = run(
        &mut world(),
        &definition("missing"),
        &model(true),
        &targets(),
    )
    .unwrap_err();
    assert_eq!(e.code, "INSTRUCTION");
    let evidence = e.evidence.unwrap();
    assert!(evidence.before_accounts[0].snapshot.is_some());
    assert!(evidence.transaction.is_none());
    assert!(evidence.program_swap.is_none());
}
#[test]
fn unresolved_logical_account_rejected() {
    let mut d = definition("migrate");
    d.account_mapping.insert("record".into(), "missing".into());
    assert_eq!(
        run(&mut world(), &d, &model(true), &targets())
            .unwrap_err()
            .code,
        "ACCOUNT_BINDING"
    );
}
#[test]
fn missing_account_mapping_rejected() {
    let mut d = definition("migrate");
    d.account_mapping.remove("record");
    let e = run(&mut world(), &d, &model(true), &targets()).unwrap_err();
    assert!(e.context.contains("account_mapping.record"));
}
#[test]
fn unknown_account_mapping_rejected() {
    let mut d = definition("migrate");
    d.account_mapping.insert("extra".into(), "record".into());
    let e = run(&mut world(), &d, &model(true), &targets()).unwrap_err();
    assert!(e.context.contains("account_mapping.extra"));
}
#[test]
fn duplicate_incompatible_privileges_rejected() {
    let mut d = definition("migrate");
    d.account_mapping.insert("record".into(), "alias".into());
    assert_eq!(
        run(&mut world(), &d, &model(true), &targets())
            .unwrap_err()
            .code,
        "ACCOUNT_BINDING"
    );
}
#[test]
fn missing_required_signer_keypair_rejected_before_swap() {
    let mut w = world();
    w.resolved.signers.remove("owner");
    let e = run(&mut w, &definition("migrate"), &model(true), &targets()).unwrap_err();
    assert_eq!(e.code, "MISSING_SIGNER");
    assert!(e.evidence.unwrap().program_swap.is_none());
}
#[test]
fn alias_signer_reuses_identity_and_does_not_sign_with_unused_key() {
    let mut w = world();
    let public = w.resolved.signers["owner"].pubkey().to_string();
    let e = migrate(&mut w);
    let r = e.transaction.as_ref().unwrap();
    assert!(r.success);
    assert_eq!(r.signer_public_keys, vec![public]);
    let json = serde_json::to_string(&e).unwrap();
    for secret_label in ["test_seed", "secret_key", "keypair", "private_key"] {
        assert!(!json.contains(secret_label));
    }
}
#[test]
fn separate_fee_payer_is_signed_alongside_required_authority() {
    let mut w = world();
    let e = MigrationExecutor::default()
        .execute(
            &mut w,
            MigrationRequest {
                project: "fixture",
                candidate: artifact(true),
                model: &model(true),
                definition: &definition("migrate"),
                targets: &targets(),
                payer: Some("unused"),
            },
        )
        .unwrap();
    let r = e.transaction.unwrap();
    assert!(r.success);
    assert_eq!(r.signer_public_keys.len(), 2);
    assert_eq!(
        r.signer_public_keys[0],
        w.resolved.addresses["unused"].to_string()
    );
}
#[test]
fn success_reallocates_and_decodes_version_specific_models() {
    let e = migrate(&mut world());
    assert_eq!(
        e.before_accounts[0].snapshot.as_ref().unwrap().data_length,
        48
    );
    assert_eq!(
        e.after_accounts[0].snapshot.as_ref().unwrap().data_length,
        49
    );
    assert_eq!(
        e.before_accounts[0].decoded.as_ref().unwrap().account_type,
        "RecordV1"
    );
    let after = e.after_accounts[0].decoded.as_ref().unwrap();
    assert_eq!(after.account_type, "RecordV2");
    assert_eq!(
        serde_json::to_value(&after.fields["version"]).unwrap(),
        serde_json::json!({"kind":"integer","value":"2"})
    );
    let r = e.transaction.unwrap();
    assert!(r.success && r.error.is_none());
    assert!(r.fee.unwrap() > 0 && r.compute_units.unwrap() > 0);
    assert!(r.signature.is_some());
    assert!(!e.preservation_evaluated);
}
#[test]
fn program_error_before_mutation_is_execution_evidence() {
    let e = run(
        &mut world(),
        &definition("reject"),
        &model(true),
        &targets(),
    )
    .unwrap();
    let r = e.transaction.unwrap();
    assert!(!r.success);
    assert_eq!(r.error_kind, Some(ExecutionErrorKind::Program));
    assert!(r.error.unwrap().contains("Custom(6000)"));
    assert!(r.logs.iter().any(|line| line.contains("before mutation")));
    assert_eq!(e.before_accounts[0].snapshot, e.after_accounts[0].snapshot);
    assert!(e.after_accounts[0].decode_error.is_some());
}
#[test]
fn attempted_write_realloc_and_transfer_roll_back_but_setup_remains_committed() {
    let mut w = world();
    let payer = w.resolved.addresses["owner"];
    let payer_before = w.runtime.get_account(&payer).unwrap().lamports;
    let mut d = definition("migrate");
    d.arguments.insert("mode".into(), toml::Value::Integer(2));
    d.arguments
        .insert("value".into(), toml::Value::Integer(999));
    let e = run(&mut w, &d, &model(true), &targets()).unwrap();
    let r = e.transaction.unwrap();
    assert_eq!(r.error_kind, Some(ExecutionErrorKind::Program));
    assert!(r
        .logs
        .iter()
        .any(|line| line.contains("attempted write, realloc and transfer")));
    assert_eq!(
        e.before_accounts[0].snapshot, e.after_accounts[0].snapshot,
        "raw bytes, owner, lamports and size must be unchanged"
    );
    let raw = e.after_accounts[0].snapshot.as_ref().unwrap();
    assert_eq!(
        &raw.raw_data[40..48],
        &42u64.to_le_bytes(),
        "committed R5 setup is retained"
    );
    assert_eq!(
        payer_before - w.runtime.get_account(&payer).unwrap().lamports,
        r.fee.unwrap(),
        "only migration transaction fees survive the failure"
    );
}
#[test]
fn decode_failure_retains_complete_raw_evidence() {
    let mut m = model(true);
    m.accounts.get_mut("RecordV2").unwrap().discriminator = Some(vec![99; 8]);
    let e = run(&mut world(), &definition("migrate"), &m, &targets()).unwrap();
    assert!(e.transaction.unwrap().success);
    let after = &e.after_accounts[0];
    assert!(after.decoded.is_none() && after.decode_error.is_some());
    let raw = after.snapshot.as_ref().unwrap();
    assert_eq!(raw.raw_data.len(), 49);
    assert!(!raw.owner.is_empty());
}
#[test]
fn before_snapshot_does_not_alias_live_state() {
    let mut w = world();
    let e = migrate(&mut w);
    let before = e.before_accounts[0].snapshot.clone();
    let address = w.resolved.addresses["record"];
    let mut account = w.runtime.get_account(&address).unwrap();
    account.data.fill(0);
    w.runtime.set_account(address, account).unwrap();
    assert_eq!(e.before_accounts[0].snapshot, before);
    assert_eq!(&before.unwrap().raw_data[40..48], &42u64.to_le_bytes());
    assert_eq!(
        e.after_accounts[0].snapshot.as_ref().unwrap().raw_data[48],
        2
    );
}
#[test]
fn created_account_is_absent_before_and_present_after() {
    let targets = vec![MigrationTarget {
        name: "receipt".into(),
        address: "$receipt".into(),
        v1_type: None,
        v2_type: Some("RecordV2".into()),
    }];
    let e = run(&mut world(), &definition("create"), &model(true), &targets).unwrap();
    assert!(e.transaction.unwrap().success);
    assert!(e.before_accounts[0].snapshot.is_none());
    assert!(e.after_accounts[0].snapshot.is_some() && e.after_accounts[0].decoded.is_some());
}
#[test]
fn closed_account_is_present_before_and_absent_after() {
    let e = run(&mut world(), &definition("close"), &model(true), &targets()).unwrap();
    assert!(e.transaction.unwrap().success);
    assert!(e.before_accounts[0].snapshot.is_some());
    assert!(e.after_accounts[0].snapshot.is_none());
    assert!(e.after_accounts[0].decode_error.is_none());
}
#[test]
fn raw_only_target_is_supported() {
    let targets = vec![MigrationTarget {
        name: "payer".into(),
        address: "$owner".into(),
        v1_type: None,
        v2_type: None,
    }];
    let e = run(&mut world(), &definition("migrate"), &model(true), &targets).unwrap();
    assert!(e.before_accounts[0].snapshot.is_some());
    assert!(e.after_accounts[0].decoded.is_none() && e.after_accounts[0].decode_error.is_none());
}
#[test]
fn duplicate_observation_targets_rejected() {
    let t = [targets()[0].clone(), targets()[0].clone()];
    assert_eq!(
        run(&mut world(), &definition("migrate"), &model(true), &t)
            .unwrap_err()
            .code,
        "ACCOUNT_BINDING"
    );
}
#[test]
fn deterministic_worlds_produce_identical_execution_reports() {
    assert_eq!(
        serde_json::to_value(migrate(&mut world())).unwrap(),
        serde_json::to_value(migrate(&mut world())).unwrap()
    );
}
#[test]
fn invalid_argument_rejected_before_candidate_swap() {
    let mut d = definition("migrate");
    d.arguments.insert(
        "value".into(),
        toml::Value::String("u64:18446744073709551616".into()),
    );
    let e = run(&mut world(), &d, &model(true), &targets()).unwrap_err();
    assert!(e.context.contains("arguments.value"));
    assert!(e.evidence.unwrap().program_swap.is_none());
}
#[test]
fn lossless_u64_max_is_encoded_and_executed() {
    let mut d = definition("migrate");
    d.arguments.insert(
        "value".into(),
        toml::Value::String(format!("u64:{}", u64::MAX)),
    );
    let e = run(&mut world(), &d, &model(true), &targets()).unwrap();
    assert!(e.transaction.unwrap().success);
    assert_eq!(
        &e.after_accounts[0].snapshot.as_ref().unwrap().raw_data[40..48],
        &u64::MAX.to_le_bytes()
    );
}
#[test]
fn complex_arguments_match_anchor_encoding_and_execute_losslessly() {
    let mut d = definition("arguments");
    let table: toml::Table = toml::from_str(
        r#"
        [payload]
        wide = "u128:340282366920938463463374607431768211455"
        signed = "i128:-170141183460469231731687303715884105728"
        nested = {flag=true}
        choice = {variant="Number",fields=["u64:18446744073709551615"]}
        values = ["u64:0","u64:18446744073709551615"]
        maybe = {some="u64:18446744073709551615"}
        array = [0,255]
        pair = [255,65535]
    "#,
    )
    .unwrap();
    d.arguments = table.into_iter().collect();
    let e = run(&mut world(), &d, &model(true), &targets()).unwrap();
    let r = e.transaction.unwrap();
    assert!(r.success, "{:?}", r.error);
    let expected = dg_r6_fixture_v2::instruction::Arguments {
        payload: dg_r6_fixture_v2::Payload {
            wide: u128::MAX,
            signed: i128::MIN,
            nested: dg_r6_fixture_v2::Nested { flag: true },
            choice: dg_r6_fixture_v2::Choice::Number(u64::MAX),
            values: vec![0, u64::MAX],
            maybe: Some(u64::MAX),
            array: [0, 255],
            pair: (255, 65535),
        },
    }
    .data();
    assert_eq!(r.instruction_data, expected);
}
#[test]
fn submission_failure_is_not_reported_as_program_error() {
    let mut w = world();
    let payer = w.resolved.addresses["unused"];
    let mut account = w.runtime.get_account(&payer).unwrap();
    account.lamports = 1;
    w.runtime.set_account(payer, account).unwrap();
    let e = MigrationExecutor::default()
        .execute(
            &mut w,
            MigrationRequest {
                project: "fixture",
                candidate: artifact(true),
                model: &model(true),
                definition: &definition("migrate"),
                targets: &targets(),
                payer: Some("unused"),
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "TRANSACTION_SUBMISSION");
    let evidence = e.evidence.unwrap();
    assert_eq!(
        evidence.transaction.unwrap().error_kind,
        Some(ExecutionErrorKind::Submission)
    );
    assert_eq!(
        evidence.before_accounts[0].snapshot,
        evidence.after_accounts[0].snapshot
    );
}
#[test]
fn snapshot_capture_failure_retains_transaction_and_before() {
    let mut w = world();
    let mut executor = MigrationExecutor::default();
    executor.limits.max_bytes = 48;
    let e = executor
        .execute(
            &mut w,
            MigrationRequest {
                project: "fixture",
                candidate: artifact(true),
                model: &model(true),
                definition: &definition("migrate"),
                targets: &targets(),
                payer: Some("owner"),
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "SNAPSHOT");
    let evidence = e.evidence.unwrap();
    assert!(evidence.transaction.unwrap().success);
    assert!(evidence.before_accounts[0].snapshot.is_some());
}

#[test]
fn missing_fee_payer_keypair_rejected_before_load() {
    let mut w = world();
    w.resolved.signers.remove("unused");
    let e = MigrationExecutor::default()
        .execute(
            &mut w,
            MigrationRequest {
                project: "fixture",
                candidate: artifact(true),
                model: &model(true),
                definition: &definition("migrate"),
                targets: &targets(),
                payer: Some("unused"),
            },
        )
        .unwrap_err();
    assert_eq!(e.code, "MISSING_SIGNER");
    assert!(e.evidence.unwrap().program_swap.is_none());
}
#[test]
fn structured_program_error_can_be_consumed_without_parsing_messages() {
    let e = run(
        &mut world(),
        &definition("reject"),
        &model(true),
        &targets(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(e.transaction.unwrap().transaction_error.unwrap()).unwrap(),
        serde_json::json!({"InstructionError":[0,{"Custom":6000}]})
    );
}
#[test]
fn unresolved_observation_address_returns_binding_error() {
    let mut t = targets();
    t[0].address = "$missing".into();
    assert_eq!(
        run(&mut world(), &definition("migrate"), &model(true), &t)
            .unwrap_err()
            .code,
        "ACCOUNT_BINDING"
    );
}
#[test]
fn omitted_discriminator_returns_instruction_error() {
    let mut m = model(true);
    m.instructions.get_mut("migrate").unwrap().discriminator = None;
    assert_eq!(
        run(&mut world(), &definition("migrate"), &m, &targets())
            .unwrap_err()
            .code,
        "INSTRUCTION"
    );
}

#[test]
fn internal_runtime_errors_are_distinct_from_submission_and_program_errors() {
    use solana_transaction_error::TransactionError;
    for error in [
        TransactionError::CommitCancelled,
        TransactionError::ProgramCacheHitMaxLimit,
        TransactionError::ResanitizationNeeded,
        TransactionError::AccountBorrowOutstanding,
    ] {
        assert_eq!(
            ExecutionErrorKind::from(&error),
            ExecutionErrorKind::Runtime
        );
    }
    assert_eq!(
        ExecutionErrorKind::from(&TransactionError::InsufficientFundsForFee),
        ExecutionErrorKind::Submission
    );
}
