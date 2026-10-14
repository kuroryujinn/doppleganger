//! Typed old-demo instruction recipe, independent of the R5/R6 encoder and R8 recipes.
mod r8_vault_support;
use anchor_lang::{InstructionData, ToAccountMetas};
use dg_runtime::lifecycle::{LifecycleEngine, ScenarioStatus};
use dg_state::{raw::RawAccountSnapshot, DecodeLimits};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

#[allow(clippy::result_large_err)]
fn send(
    vm: &mut LiteSVM,
    owner: &Keypair,
    other: Option<&Keypair>,
    ix: Instruction,
) -> TransactionResult {
    vm.expire_blockhash();
    let mut signers = vec![owner];
    if let Some(other) = other {
        signers.push(other);
    }
    vm.send_transaction(Transaction::new(
        &signers,
        Message::new(&[ix], Some(&owner.pubkey())),
        vm.latest_blockhash(),
    ))
}
fn snapshot(vm: &LiteSVM, vault: Address) -> RawAccountSnapshot {
    let a = vm.get_account(&vault).unwrap();
    RawAccountSnapshot::capture(
        vault.to_string(),
        a.owner.to_string(),
        a.lamports,
        a.executable,
        a.data,
        DecodeLimits::default(),
    )
    .unwrap()
}
fn compare(id: &str) {
    let (mut manifest, base) = r8_vault_support::manifest(false);
    manifest.scenario.retain(|s| s.id.as_deref() == Some(id));
    let report = LifecycleEngine.run(&manifest, &base).unwrap();
    assert_eq!(
        (report.total, report.passed, report.failed, report.errors),
        (1, 1, 0, 0),
        "{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    let scenario = &report.scenarios[0];
    assert_eq!(scenario.status, ScenarioStatus::Pass);
    let owner = Keypair::new_from_array([11; 32]);
    let stranger = Keypair::new_from_array([12; 32]);
    let program_id = dg_vault_v1::id();
    let (vault, _) =
        Address::find_program_address(&[b"vault", owner.pubkey().as_ref()], &program_id);
    let mut vm = LiteSVM::new();
    vm.airdrop(&owner.pubkey(), 10_000_000_000).unwrap();
    vm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    vm.add_program_from_file(program_id, base.join(&manifest.versions.v1.program_path))
        .unwrap();
    send(
        &mut vm,
        &owner,
        None,
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
        &mut vm,
        &owner,
        None,
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
    assert_eq!(
        scenario.steps[0].after[0].snapshot.as_ref().unwrap(),
        &snapshot(&vm, vault)
    );
    vm.add_program_from_file(program_id, base.join(&manifest.versions.v2.program_path))
        .unwrap();
    let wrong = matches!(id, "unauthorized-migration" | "rollback");
    let migration = Instruction {
        program_id,
        accounts: dg_vault_v2::accounts::Migrate {
            vault,
            owner: if wrong {
                stranger.pubkey()
            } else {
                owner.pubkey()
            },
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: dg_vault_v2::instruction::Migrate {}.data(),
    };
    let withdraw = |amount| Instruction {
        program_id,
        accounts: dg_vault_v2::accounts::Operate {
            vault,
            owner: owner.pubkey(),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: dg_vault_v2::instruction::Withdraw { amount }.data(),
    };
    let operations = match id {
        "unmigrated-v2-operation" => vec![withdraw(1)],
        "repeat-migration" => vec![migration.clone(), migration],
        "post-migration-operation" => vec![migration, withdraw(100_000)],
        "post-migration-limit" => vec![migration, withdraw(500_001)],
        _ => vec![migration],
    };
    let steps: Vec<_> = scenario
        .steps
        .iter()
        .filter(|s| s.execution.is_some())
        .collect();
    assert_eq!(steps.len(), operations.len());
    for (step, ix) in steps.into_iter().zip(operations) {
        assert_eq!(
            step.before[0].snapshot.as_ref().unwrap(),
            &snapshot(&vm, vault)
        );
        let tx = send(&mut vm, &owner, wrong.then_some(&stranger), ix);
        let actual = step.execution.as_ref().unwrap();
        assert_eq!(actual.success, tx.is_ok());
        assert_eq!(
            actual.transaction_error,
            tx.as_ref().err().map(|e| e.err.clone())
        );
        let metadata = match &tx {
            Ok(m) => m,
            Err(e) => &e.meta,
        };
        assert_eq!(actual.logs, metadata.logs);
        assert_eq!(actual.compute_units, Some(metadata.compute_units_consumed));
        assert_eq!(
            step.after[0].snapshot.as_ref().unwrap(),
            &snapshot(&vm, vault)
        );
    }
}
#[test]
fn normal_migration_matches_old_typed_recipe() {
    compare("normal-migration");
}
#[test]
fn unauthorized_matches_old_typed_recipe() {
    compare("unauthorized-migration");
}
#[test]
fn repeated_migration_matches_old_typed_recipe() {
    compare("repeat-migration");
}
#[test]
fn rollback_matches_old_typed_recipe() {
    compare("rollback");
}
#[test]
fn unmigrated_operation_matches_old_typed_recipe() {
    compare("unmigrated-v2-operation");
}
#[test]
fn post_migration_operation_matches_old_typed_recipe() {
    compare("post-migration-operation");
}
#[test]
fn post_migration_limit_matches_old_typed_recipe() {
    compare("post-migration-limit");
}
