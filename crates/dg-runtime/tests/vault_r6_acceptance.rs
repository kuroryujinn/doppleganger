//! Independent typed recipe from the old demo versus configured R5 + R6 evidence.
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use dg_config::{addresses::AddressDefinition, ProjectManifest};
use dg_runtime::{MigrationExecutor, RuntimeWorldBuilder};
use dg_state::{raw::RawAccountSnapshot, DecodeLimits, TrailingBytes};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;

#[allow(clippy::result_large_err)]
fn send(vm: &mut LiteSVM, owner: &Keypair, instruction: Instruction) -> TransactionResult {
    vm.expire_blockhash();
    let tx = Transaction::new(
        &[owner],
        Message::new(&[instruction], Some(&owner.pubkey())),
        vm.latest_blockhash(),
    );
    vm.send_transaction(tx)
}

fn compare(faulty: bool, wrong_authority: bool) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let base = root.join("examples/vault-runtime");
    let mut manifest =
        ProjectManifest::parse(&std::fs::read_to_string(base.join("doppelganger.toml")).unwrap())
            .unwrap();
    for (name, seed) in [("owner", [11; 32]), ("stranger", [12; 32])] {
        match manifest.addresses.get_mut(name).unwrap() {
            AddressDefinition::Signer { test_seed, .. } => *test_seed = Some(seed),
            _ => panic!("expected signer"),
        }
    }
    if faulty {
        manifest.versions.v2.program_path = "../../target/deploy/bad/dg_vault_v2.so".into();
    }
    let mut generic = RuntimeWorldBuilder::default()
        .build_manifest(&manifest, &base)
        .unwrap();
    if wrong_authority {
        manifest
            .migration
            .account_mapping
            .insert("owner".into(), "stranger".into());
        manifest.fixture.as_mut().unwrap().payer = Some("stranger".into());
    }
    let execution = MigrationExecutor::default()
        .execute_manifest(&mut generic.world, &manifest, &base)
        .unwrap();

    // Keep this typed reference independent of the generic instruction encoder and builder.
    let owner = Keypair::new_from_array([11; 32]);
    let stranger = Keypair::new_from_array([12; 32]);
    let program_id = dg_vault_v1::id();
    let (vault, _) =
        Address::find_program_address(&[b"vault", owner.pubkey().as_ref()], &program_id);
    let mut reference = LiteSVM::new();
    reference.airdrop(&owner.pubkey(), 10_000_000_000).unwrap();
    reference
        .airdrop(&stranger.pubkey(), 1_000_000_000)
        .unwrap();
    reference
        .add_program_from_file(program_id, root.join("target/deploy/v1/dg_vault_v1.so"))
        .unwrap();
    send(
        &mut reference,
        &owner,
        Instruction {
            program_id,
            accounts: dg_vault_v1::accounts::Initialize {
                vault,
                owner: owner.pubkey(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: dg_vault_v1::instruction::Initialize {}.data(),
        },
    )
    .unwrap();
    send(
        &mut reference,
        &owner,
        Instruction {
            program_id,
            accounts: dg_vault_v1::accounts::Deposit {
                vault,
                owner: owner.pubkey(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: dg_vault_v1::instruction::Deposit { amount: 1_000_000 }.data(),
        },
    )
    .unwrap();
    let before = reference.get_account(&vault).unwrap();
    reference
        .add_program_from_file(program_id, base.join(&manifest.versions.v2.program_path))
        .unwrap();
    let signer = if wrong_authority { &stranger } else { &owner };
    let result = send(
        &mut reference,
        signer,
        Instruction {
            program_id,
            accounts: dg_vault_v2::accounts::Migrate {
                vault,
                owner: signer.pubkey(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: dg_vault_v2::instruction::Migrate {}.data(),
        },
    );
    let actual = execution.transaction.as_ref().unwrap();
    assert_eq!(actual.success, result.is_ok());
    assert_eq!(
        actual.error,
        result.as_ref().err().map(|e| format!("{:?}", e.err))
    );
    assert!(!actual.logs.is_empty());
    assert_eq!(actual.success, !wrong_authority);
    let after = reference.get_account(&vault).unwrap();
    for (account, observed) in [
        (&before, &execution.before_accounts[0]),
        (&after, &execution.after_accounts[0]),
    ] {
        let raw = RawAccountSnapshot::capture(
            vault.to_string(),
            account.owner.to_string(),
            account.lamports,
            account.executable,
            account.data.clone(),
            DecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(&raw, observed.snapshot.as_ref().unwrap());
    }
    if !wrong_authority {
        let typed = dg_vault_v2::VaultV2::try_deserialize(&mut after.data.as_slice()).unwrap();
        assert_eq!(typed.owner, owner.pubkey());
        assert_eq!(typed.balance, 1_000_000);
        assert_eq!(typed.withdrawal_limit, 500_000);
        let model =
            dg_analysis::normalize(&std::fs::read(base.join(&manifest.versions.v2.idl)).unwrap())
                .unwrap();
        let decoded = dg_state::decode_account(
            &model,
            "VaultV2",
            &after.data,
            DecodeLimits::default(),
            TrailingBytes::Reject,
        )
        .unwrap();
        assert_eq!(
            execution.after_accounts[0].decoded.as_ref().unwrap(),
            &decoded
        );
    } else {
        assert!(execution.after_accounts[0].decode_error.is_some());
    }
    assert!(!execution.preservation_evaluated);
    let json = serde_json::to_value(execution).unwrap();
    assert!(json.get("findings").is_none() && json.get("verdict").is_none());
}

#[test]
fn configured_vault_migration_matches_typed_reference() {
    compare(false, false);
}
#[test]
fn faulty_vault_produces_matching_evidence_without_safety_verdict() {
    compare(true, false);
}
#[test]
fn vault_program_error_matches_typed_reference() {
    compare(false, true);
}
