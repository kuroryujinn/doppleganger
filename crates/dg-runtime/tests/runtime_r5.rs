mod support;
use dg_config::{fixture::FixtureInstructions, MigrationDefinition};
use dg_runtime::{Phase, RuntimeWorldBuilder};
use dg_state::{DecodeLimits, Value};
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_signer::Signer;
use std::collections::BTreeMap;
use support::*;

#[test]
fn fresh_world_loads_valid_configured_program() {
    let result = build(&fixture());
    assert!(
        result
            .world
            .runtime
            .get_account(&dg_r5_fixture::id())
            .unwrap()
            .executable
    );
    assert!(result.executed_actions.is_empty());
    assert!(result.observed_accounts.is_empty());
}
#[test]
fn missing_program_is_structured() {
    let mut a = artifact();
    a.program_path = a.program_path.with_file_name("missing-r5.so");
    let error = RuntimeWorldBuilder::default()
        .build(
            a,
            &model(),
            &fixture(),
            resolved(),
            FixtureInstructions {
                initialize: vec![],
                setup: vec![],
            },
        )
        .err()
        .unwrap();
    assert_eq!(error.code, "MISSING_PROGRAM");
}
#[test]
fn invalid_program_artifacts_fail_cleanly() {
    let path = std::env::temp_dir().join(format!("dg-r5-invalid-{}.so", std::process::id()));
    for data in [&b"not-elf"[..], &b"\x7fELFbroken"[..]] {
        std::fs::write(&path, data).unwrap();
        let mut a = artifact();
        a.program_path = path.clone();
        let e = RuntimeWorldBuilder::default()
            .build(
                a,
                &model(),
                &fixture(),
                resolved(),
                FixtureInstructions {
                    initialize: vec![],
                    setup: vec![],
                },
            )
            .err()
            .unwrap();
        assert!(matches!(
            e.code.as_str(),
            "INVALID_PROGRAM" | "PROGRAM_LOAD"
        ));
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn signer_funding_is_exact_and_alias_not_funded_twice() {
    let r = build(&fixture());
    let owner = r.world.resolved.addresses["owner"];
    assert_eq!(owner, r.world.resolved.addresses["alias"]);
    assert_eq!(r.world.resolved.signers.len(), 1);
    assert_eq!(r.world.funded_signers, 1);
    assert_eq!(
        r.world.runtime.get_account(&owner).unwrap().lamports,
        10_000_000_000
    );
}
#[test]
fn zero_funding_is_supported() {
    let r = build_with(&fixture(), resolve(definitions(Some([7; 32]), 0))).unwrap();
    assert_eq!(
        r.world
            .runtime
            .get_account(&r.world.resolved.addresses["owner"])
            .unwrap()
            .lamports,
        0
    );
}
#[test]
fn test_keys_are_reproducible_and_different_seeds_differ() {
    let a = build(&fixture());
    let b = build(&fixture());
    let c = build_with(&fixture(), resolve(definitions(Some([8; 32]), 1))).unwrap();
    assert_eq!(
        a.world.resolved.addresses["owner"],
        b.world.resolved.addresses["owner"]
    );
    assert_ne!(
        a.world.resolved.addresses["owner"],
        c.world.resolved.addresses["owner"]
    );
}
#[test]
fn random_signer_default_still_generates_distinct_keys() {
    let a = build_with(&fixture(), resolve(definitions(None, 1))).unwrap();
    let b = build_with(&fixture(), resolve(definitions(None, 1))).unwrap();
    assert_ne!(
        a.world.resolved.addresses["owner"],
        b.world.resolved.addresses["owner"]
    );
}
fn synthetic(lamports: i64, body: &str) -> dg_config::FixtureDefinition {
    let mut f = fixture();
    let mut account = observe(0);
    account.account_type = "Blob".into();
    account.lamports = toml::Value::Integer(lamports);
    account.fields = Some(toml::Table::from_iter([(
        "body".into(),
        toml::Value::String(body.into()),
    )]));
    f.account.push(account);
    f
}
#[test]
fn synthetic_accounts_use_configured_owner_lamports_and_exact_bytes() {
    let r = build(&synthetic(123, "\0hello"));
    let s = &r.observed_accounts[0].snapshot;
    assert_eq!(s.owner, dg_r5_fixture::id().to_string());
    assert_eq!(s.lamports.to_string(), "123");
    assert_eq!(
        s.raw_data,
        [vec![9, 6, 0, 0, 0], b"\0hello".to_vec()].concat()
    );
    assert!(r.observed_accounts[0].decoded.is_some());
}
#[test]
fn synthetic_zero_lamport_account_is_retained() {
    assert_eq!(
        build(&synthetic(0, "x")).observed_accounts[0]
            .snapshot
            .lamports
            .to_string(),
        "0"
    );
}
#[test]
fn large_synthetic_account_respects_limits() {
    let f = synthetic(1, &"x".repeat(1024 * 1024));
    assert_eq!(
        build(&f).observed_accounts[0].snapshot.data_length,
        1024 * 1024 + 5
    );
    let r = resolved();
    let m = model();
    let actions = f.instructions(&m, &artifact().program_id, &r).unwrap();
    let e = RuntimeWorldBuilder {
        limits: DecodeLimits {
            max_bytes: 100,
            ..DecodeLimits::default()
        },
    }
    .build(artifact(), &m, &f, r, actions)
    .err()
    .unwrap();
    assert_eq!(e.code, "SYNTHETIC_ACCOUNT");
}
#[test]
fn invalid_synthetic_fields_return_error() {
    let mut f = synthetic(1, "x");
    f.account[0]
        .fields
        .as_mut()
        .unwrap()
        .insert("body".into(), toml::Value::Boolean(true));
    let e = RuntimeWorldBuilder::default()
        .build(
            artifact(),
            &model(),
            &f,
            resolved(),
            FixtureInstructions {
                initialize: vec![],
                setup: vec![],
            },
        )
        .err()
        .unwrap();
    assert_eq!(e.code, "SYNTHETIC_ACCOUNT");
}
#[test]
fn synthetic_state_cannot_overwrite_signer() {
    let mut f = synthetic(1, "x");
    f.account[0].address = "$owner".into();
    assert_eq!(
        build_with(&f, resolved()).err().unwrap().code,
        "SYNTHETIC_ACCOUNT"
    );
}
#[test]
fn initialization_creates_real_account_and_attaches_required_signer() {
    let r = build(&initialized());
    assert_eq!(r.executed_actions.len(), 1);
    let action = &r.executed_actions[0];
    assert!(action.success);
    assert_eq!(action.phase, Phase::Initialize);
    assert_eq!(
        action.signer_public_keys,
        [r.world.resolved.signers["owner"].pubkey().to_string()]
    );
    assert!(action.compute_units.unwrap() > 0);
    assert!(!action.logs.is_empty());
    assert_eq!(
        r.observed_accounts[0].decoded.as_ref().unwrap().fields["value"],
        Value::Integer(1.into())
    );
}
#[test]
fn multiple_initialization_actions_preserve_declaration_order() {
    let mut f = initialized();
    f.initialize.push(init(1, 10));
    f.account.push(observe(1));
    let r = build(&f);
    assert_eq!(r.executed_actions.len(), 2);
    assert_eq!(r.executed_actions[0].action_index, 0);
    assert_eq!(r.executed_actions[1].action_index, 1);
    assert_eq!(
        r.observed_accounts[1].decoded.as_ref().unwrap().fields["value"],
        Value::Integer(10.into())
    );
}
#[test]
fn setup_actions_execute_after_initialization_in_order() {
    let mut f = initialized();
    f.setup = vec![advance(0, 1, 2), advance(0, 2, 3)];
    let r = build(&f);
    assert_eq!(
        r.executed_actions
            .iter()
            .map(|r| r.phase)
            .collect::<Vec<_>>(),
        [Phase::Initialize, Phase::Setup, Phase::Setup]
    );
    assert_eq!(
        r.observed_accounts[0].decoded.as_ref().unwrap().fields["value"],
        Value::Integer(3.into())
    );
}
#[test]
fn missing_required_keypair_is_an_attempted_action_failure() {
    let f = initialized();
    let m = model();
    let mut r = resolved();
    let plan = f.instructions(&m, &artifact().program_id, &r).unwrap();
    r.signers.clear();
    let e = RuntimeWorldBuilder::default()
        .build(artifact(), &m, &f, r, plan)
        .err()
        .unwrap();
    assert_eq!(e.code, "ACTION_FAILED");
    assert_eq!(e.executed_actions.len(), 1);
    assert!(e.executed_actions[0]
        .error
        .as_ref()
        .unwrap()
        .contains("missing required signer"));
}
#[test]
fn failure_in_initialization_prevents_setup() {
    let mut f = initialized();
    f.initialize.push(init(0, 2));
    f.setup.push(advance(0, 1, 3));
    let e = build_with(&f, resolved()).err().unwrap();
    assert_eq!(e.executed_actions.len(), 2);
    assert!(e.executed_actions[0].success);
    assert!(!e.executed_actions[1].success);
    assert!(e
        .executed_actions
        .iter()
        .all(|r| r.phase == Phase::Initialize));
    assert!(!e.executed_actions[1].logs.is_empty());
}
#[test]
fn setup_failure_is_fail_fast_with_context() {
    let mut f = initialized();
    f.setup = vec![advance(0, 99, 2), advance(0, 1, 3)];
    let e = build_with(&f, resolved()).err().unwrap();
    assert_eq!(e.executed_actions.len(), 2);
    assert!(e.context.contains("Setup[0] advance"));
    assert!(e.executed_actions[1]
        .error
        .as_ref()
        .unwrap()
        .contains("Custom"));
}
#[test]
fn observation_only_accounts_are_not_synthesized() {
    let mut f = initialized();
    f.account.push(observe(1));
    f.account[1].lamports = toml::Value::Integer(900);
    let e = build_with(&f, resolved()).err().unwrap();
    assert_eq!(e.code, "MISSING_ACCOUNT");
    assert_eq!(e.observed_accounts.len(), 1);
    assert_eq!(e.executed_actions.len(), 1);
}
#[test]
fn observations_use_actual_owner_lamports_and_bytes() {
    let mut f = initialized();
    f.account[0].owner = "11111111111111111111111111111111".into();
    f.account[0].lamports = toml::Value::Integer(999);
    let r = build(&f);
    let s = &r.observed_accounts[0].snapshot;
    let actual = r
        .world
        .runtime
        .get_account(&r.world.resolved.addresses["record0"])
        .unwrap();
    assert_eq!(s.owner, actual.owner.to_string());
    assert_eq!(s.lamports.to_string(), actual.lamports.to_string());
    assert_ne!(s.lamports.to_string(), "999");
    assert_eq!(s.raw_data, actual.data);
}
#[test]
fn multiple_observations_keep_configured_order() {
    let mut f = initialized();
    f.initialize.push(init(1, 2));
    f.account = vec![observe(1), observe(0)];
    let r = build(&f);
    assert_eq!(
        r.observed_accounts
            .iter()
            .map(|o| o.binding.name.as_str())
            .collect::<Vec<_>>(),
        ["record1", "record0"]
    );
}
#[test]
fn decode_failure_retains_raw_state() {
    let mut f = initialized();
    f.setup.push(MigrationDefinition {
        instruction: "corrupt".into(),
        account_mapping: BTreeMap::from([("record".into(), "record0".into())]),
        arguments: BTreeMap::new(),
    });
    let r = build(&f);
    let o = &r.observed_accounts[0];
    assert!(o.decoded.is_none());
    assert_eq!(o.decode_error.as_ref().unwrap().code, "DISCRIMINATOR");
    assert_eq!(o.snapshot.raw_data[0], 255);
}
#[test]
fn unresolved_observation_binding_fails_cleanly() {
    let mut f = fixture();
    let mut account = observe(0);
    account.address = "$missing".into();
    f.account.push(account);
    let e = RuntimeWorldBuilder::default()
        .build(
            artifact(),
            &model(),
            &f,
            resolved(),
            FixtureInstructions {
                initialize: vec![],
                setup: vec![],
            },
        )
        .err()
        .unwrap();
    assert_eq!(e.code, "ACCOUNT_BINDING");
}
#[test]
fn world_accounts_balances_clock_and_history_are_isolated() {
    let mut a = build(&initialized());
    let owner = a.world.resolved.addresses["owner"];
    let record = a.world.resolved.addresses["record0"];
    a.world
        .runtime
        .set_account(
            owner,
            Account {
                lamports: 77,
                ..Account::default()
            },
        )
        .unwrap();
    a.world.runtime.warp_to_slot(500);
    let b = build(&fixture());
    assert!(b.world.runtime.get_account(&record).is_none());
    assert_eq!(
        b.world.runtime.get_account(&owner).unwrap().lamports,
        10_000_000_000
    );
    assert_ne!(
        a.world.runtime.get_sysvar::<Clock>().slot,
        b.world.runtime.get_sysvar::<Clock>().slot
    );
    let signature = a.executed_actions[0]
        .signature
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    assert!(a.world.runtime.get_transaction(&signature).is_some());
    assert!(b.world.runtime.get_transaction(&signature).is_none());
}
#[test]
fn snapshot_limits_fail_with_execution_history() {
    let f = initialized();
    let r = resolved();
    let m = model();
    let actions = f.instructions(&m, &artifact().program_id, &r).unwrap();
    let e = RuntimeWorldBuilder {
        limits: DecodeLimits {
            max_bytes: 1,
            ..DecodeLimits::default()
        },
    }
    .build(artifact(), &m, &f, r, actions)
    .err()
    .unwrap();
    assert_eq!(e.code, "SNAPSHOT");
    assert_eq!(e.executed_actions.len(), 1);
}
#[test]
fn reports_contain_public_keys_not_private_key_fields() {
    let r = build(&initialized());
    let json = serde_json::to_string(&r.executed_actions).unwrap();
    assert!(json.contains("signer_public_keys"));
    assert!(!json.contains("test_seed"));
    assert!(!json.contains("secret"));
}
#[test]
fn compiled_action_count_mismatch_is_rejected() {
    let e = RuntimeWorldBuilder::default()
        .build(
            artifact(),
            &model(),
            &initialized(),
            resolved(),
            FixtureInstructions {
                initialize: vec![],
                setup: vec![],
            },
        )
        .err()
        .unwrap();
    assert_eq!(e.code, "CONFIG");
    let f = initialized();
    let m = model();
    let r = resolved();
    let mut actions = f.instructions(&m, &artifact().program_id, &r).unwrap();
    for i in 0u16..257 {
        let mut bytes = [0; 32];
        bytes[..2].copy_from_slice(&i.to_le_bytes());
        actions.initialize[0]
            .accounts
            .push(solana_instruction::AccountMeta::new_readonly(
                Address::new_from_array(bytes),
                false,
            ));
    }
    let error = RuntimeWorldBuilder::default()
        .build(artifact(), &m, &f, r, actions)
        .err()
        .unwrap();
    assert!(error.message.contains("256 unique accounts"));
}
#[test]
fn malformed_resolved_signer_identity_is_rejected() {
    let mut r = resolved();
    r.addresses.insert("owner".into(), Address::default());
    let e = RuntimeWorldBuilder::default()
        .build(
            artifact(),
            &model(),
            &fixture(),
            r,
            FixtureInstructions {
                initialize: vec![],
                setup: vec![],
            },
        )
        .err()
        .unwrap();
    assert_eq!(e.code, "SIGNER");
}

#[test]
fn synthetic_binary_payload_is_preserved_exactly() {
    let mut f = synthetic(1, "");
    f.account[0].account_type = "Raw".into();
    f.account[0].fields.as_mut().unwrap().insert(
        "body".into(),
        toml::Value::Array(
            [0, 255, 128, 0]
                .into_iter()
                .map(toml::Value::Integer)
                .collect(),
        ),
    );
    assert_eq!(
        build(&f).observed_accounts[0].snapshot.raw_data,
        [10, 4, 0, 0, 0, 0, 255, 128, 0]
    );
}

#[test]
fn initialization_order_is_enforced_by_program_state() {
    let mut f = initialized();
    f.initialize.push(advance(0, 1, 2));
    assert_eq!(
        build(&f).observed_accounts[0]
            .decoded
            .as_ref()
            .unwrap()
            .fields["value"],
        Value::Integer(2.into())
    );
    f.initialize.swap(0, 1);
    let error = build_with(&f, resolved()).err().unwrap();
    assert_eq!(error.executed_actions.len(), 1);
    assert!(!error.executed_actions[0].success);
}

#[test]
fn fee_payer_alias_reuses_identity() {
    let mut f = initialized();
    f.payer = Some("alias".into());
    let r = build(&f);
    assert_eq!(r.world.funded_signers, 1);
    assert_eq!(r.executed_actions[0].signer_public_keys.len(), 1);
}

#[test]
fn insufficient_funding_returns_action_error_without_panicking() {
    let error = build_with(&initialized(), resolve(definitions(Some([7; 32]), 0)))
        .err()
        .unwrap();
    assert_eq!(error.code, "ACTION_FAILED");
    assert_eq!(error.executed_actions.len(), 1);
    assert!(!error.executed_actions[0].success);
}
