//! R5 acceptance only: compare V1 setup with the independent, typed reference recipe.
//! No V2 program is loaded and no migration instruction is submitted.
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use dg_config::{addresses::AddressDefinition, ProjectManifest};
use dg_runtime::RuntimeWorldBuilder;
use dg_state::{raw::RawAccountSnapshot, DecodeLimits, TrailingBytes};
use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;

#[test]
fn configured_v1_world_matches_hardcoded_reference_setup() {
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
    // Prove the V1-only build does not require V2 files or resolve migration behavior.
    manifest.versions.v2.program_path = "intentionally-missing-v2.so".into();
    manifest.versions.v2.idl = "intentionally-missing-v2.json".into();
    manifest.migration.instruction = "not_a_real_instruction".into();
    let generic = RuntimeWorldBuilder::default()
        .build_manifest(&manifest, &base)
        .unwrap_or_else(|e| panic!("{e:#?}"));

    // Same hardcoded V1 recipe as dg-cli::run, kept independent of fixture compilation.
    let owner = Keypair::new_from_array([11; 32]);
    let stranger = Keypair::new_from_array([12; 32]);
    let program_id = dg_vault_v1::id();
    let (account, _) =
        Address::find_program_address(&[b"vault", owner.pubkey().as_ref()], &program_id);
    let mut reference = LiteSVM::new();
    reference.airdrop(&owner.pubkey(), 10_000_000_000).unwrap();
    reference
        .airdrop(&stranger.pubkey(), 1_000_000_000)
        .unwrap();
    reference
        .add_program_from_file(program_id, root.join("target/deploy/v1/dg_vault_v1.so"))
        .unwrap();
    let initialize = Instruction {
        program_id,
        accounts: dg_vault_v1::accounts::Initialize {
            vault: account,
            owner: owner.pubkey(),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: dg_vault_v1::instruction::Initialize {}.data(),
    };
    let deposit = Instruction {
        program_id,
        accounts: dg_vault_v1::accounts::Deposit {
            vault: account,
            owner: owner.pubkey(),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: dg_vault_v1::instruction::Deposit { amount: 1_000_000 }.data(),
    };
    for instruction in [initialize, deposit] {
        reference.expire_blockhash();
        let tx = Transaction::new(
            &[&owner],
            Message::new(&[instruction], Some(&owner.pubkey())),
            reference.latest_blockhash(),
        );
        reference.send_transaction(tx).unwrap();
    }
    let actual = reference.get_account(&account).unwrap();
    let typed = dg_vault_v1::VaultV1::try_deserialize(&mut actual.data.as_slice()).unwrap();
    assert_eq!(typed.owner, owner.pubkey());
    assert_eq!(typed.balance, 1_000_000);
    let raw = RawAccountSnapshot::capture(
        account.to_string(),
        actual.owner.to_string(),
        actual.lamports,
        actual.executable,
        actual.data,
        DecodeLimits::default(),
    )
    .unwrap();
    let observed = &generic.observed_accounts[0];
    assert_eq!(
        raw, observed.snapshot,
        "address, owner, lamports, data length and bytes must all match"
    );
    let model =
        dg_analysis::normalize(&std::fs::read(root.join("target/idl/v1.json")).unwrap()).unwrap();
    assert_eq!(
        dg_state::decode_account(
            &model,
            "VaultV1",
            &raw.raw_data,
            DecodeLimits::default(),
            TrailingBytes::Reject
        )
        .unwrap(),
        *observed.decoded.as_ref().unwrap()
    );
    assert_eq!(generic.executed_actions.len(), 2);
    assert!(generic.executed_actions.iter().all(|r| r.success));
}
