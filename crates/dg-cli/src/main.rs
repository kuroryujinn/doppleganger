mod bundle;
mod doctor;
mod preflight;
mod production;
mod project;
mod registry;
mod regression;
mod runtime;
mod schema;
mod snapshot;

use anchor_lang::{AccountDeserialize, InstructionData, Space, ToAccountMetas};
use dg_config::{InstructionBinding, ProjectManifest};
use litesvm::{types::TransactionResult, LiteSVM};
use schema::{AccountDiff, Schema};
use serde::Serialize;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_rent::Rent;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

const DEPOSIT: u64 = 1_000_000;
const WITHDRAW: u64 = 100_000;

#[derive(Serialize)]
struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
}

#[derive(Serialize)]
struct Report {
    program_id: String,
    account_address: String,
    schema: Schema,
    checks: Vec<Check>,
}

#[derive(Serialize)]
struct ProjectReport {
    name: String,
    program_id: String,
    coverage: &'static str,
    accounts: Vec<AccountDiff>,
    runtime: Option<Report>,
}

impl Report {
    fn check(&mut self, name: &'static str, passed: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name,
            passed,
            detail: detail.into(),
        });
    }

    fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }
}

// Preserve the reference demo's LiteSVM result type, including full failure metadata.
#[allow(clippy::result_large_err)]
fn send(svm: &mut LiteSVM, signer: &Keypair, instruction: Instruction) -> TransactionResult {
    svm.expire_blockhash();
    let tx = Transaction::new(
        &[signer],
        Message::new(&[instruction], Some(&signer.pubkey())),
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx)
}

fn instruction(accounts: Vec<solana_instruction::AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: dg_vault_v1::id(),
        accounts,
        data,
    }
}

fn vault_migration_sources(vault: Address, owner: Address) -> BTreeMap<String, Address> {
    BTreeMap::from([
        ("vault".to_owned(), vault),
        ("owner".to_owned(), owner),
        ("system_program".to_owned(), anchor_lang::system_program::ID),
    ])
}

fn vault_migration_binding() -> Result<InstructionBinding, Box<dyn Error>> {
    let manifest = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/vault-migration/doppelganger.toml"
    ));
    Ok(ProjectManifest::load(manifest)?.migration)
}

fn migration_command(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let action = args
        .next()
        .ok_or("usage: doppelganger migration <inspect|check> [manifest-or-project-name]")?;
    if action == "test" {
        return production::command(args);
    }
    let input = args
        .next()
        .unwrap_or_else(|| "doppelganger.toml".to_owned());
    if args.next().is_some() {
        return Err("too many migration arguments".into());
    }
    let path = if Path::new(&input).exists() {
        PathBuf::from(&input)
    } else {
        project::registered_manifest(&input)?
    };
    let loaded = ProjectManifest::load(&path)?;
    match action.as_str() {
        "inspect" => {
            if !loaded.manifest.addresses.is_empty()
                || loaded.manifest.fixture.as_ref().is_some_and(|f| {
                    !f.account.is_empty() || !f.initialize.is_empty() || !f.setup.is_empty()
                })
            {
                let programs = std::collections::BTreeMap::from([
                    ("v1".into(), loaded.manifest.versions.v1.program_id.parse()?),
                    ("v2".into(), loaded.manifest.versions.v2.program_id.parse()?),
                ]);
                let resolved =
                    dg_config::ResolvedAddresses::resolve(&loaded.manifest.addresses, &programs)?;
                if let Some(fixture) = &loaded.manifest.fixture {
                    let plan = fixture.instructions(
                        &loaded.v1,
                        &loaded.manifest.versions.v1.program_id,
                        &resolved,
                    )?;
                    println!(
                        "Fixture plan: {} initialization, {} setup instructions (not executed)",
                        plan.initialize.len(),
                        plan.setup.len()
                    );
                }
                for (name, address) in &resolved.addresses {
                    println!("Address {name}: {address}");
                }
            }
            println!("Project: {}", loaded.manifest.project.name);
            println!("Program: {}", loaded.migration.program_id);
            println!("Instruction: {}", loaded.migration.instruction);
            println!("Discriminator: {:?}", loaded.migration.discriminator);
            for account in &loaded.migration.accounts {
                println!(
                    "Account {} <- {} signer={} writable={}",
                    account.idl_name, account.source, account.signer, account.writable
                );
            }
            for argument in &loaded.migration.arguments {
                println!(
                    "Argument {}: {:?} ({} bytes)",
                    argument.name,
                    argument.ty,
                    argument.encoded.len()
                );
            }
        }
        "check" => {
            if loaded.manifest.fixture.as_ref().map(|f| f.name.as_str()) != Some("vault")
                || loaded.manifest.fixture.as_ref().is_some_and(|f| {
                    !f.account.is_empty() || !f.initialize.is_empty() || !f.setup.is_empty()
                })
                || !loaded.manifest.addresses.is_empty()
                || loaded.migration.program_id != dg_vault_v1::id()
                || !loaded.v1.accounts.contains_key("VaultV1")
                || !loaded.v2.accounts.contains_key("VaultV2")
            {
                return Err("runtime fixture is not supported; use migration inspect to validate the generic binding".into());
            }
            let report = run(
                loaded.v1_program_path,
                loaded.v2_program_path,
                loaded.v1_idl_path,
                loaded.v2_idl_path,
                &loaded.migration,
            )?;
            fs::create_dir_all("report")?;
            let output = PathBuf::from(format!(
                "report/{}-migration.json",
                loaded.manifest.project.name
            ));
            fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
            let html = output.with_extension("html");
            write_html(&html, &report)?;
            println!(
                "{} checks, {} failed",
                report.checks.len(),
                report.checks.iter().filter(|c| !c.passed).count()
            );
            println!("Report: {}", output.display());
            println!("HTML: {}", html.display());
            if !report.passed() {
                std::process::exit(1);
            }
        }
        _ => {
            return Err(
                "usage: doppelganger migration <inspect|check> [manifest-or-project-name]".into(),
            )
        }
    }
    Ok(())
}

fn result_detail(result: &TransactionResult) -> String {
    match result {
        Ok(meta) => format!(
            "ok; compute_units={}; fee={}",
            meta.compute_units_consumed, meta.fee
        ),
        Err(failed) => {
            let reason = failed
                .meta
                .logs
                .iter()
                .find(|line| line.contains("AnchorError"))
                .map(String::as_str)
                .unwrap_or("transaction failed");
            format!("{:?}; {reason}", failed.err)
        }
    }
}

fn anchor_error(result: &TransactionResult, code: &str) -> bool {
    result.as_ref().err().is_some_and(|failed| {
        failed
            .meta
            .logs
            .iter()
            .any(|line| line.contains(&format!("Error Code: {code}.")))
    })
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn write_html(path: &PathBuf, report: &Report) -> Result<(), Box<dyn Error>> {
    let failures = report.checks.iter().filter(|check| !check.passed).count();
    let mut rows = String::new();
    for check in &report.checks {
        let status = if check.passed { "PASS" } else { "FAIL" };
        rows.push_str(&format!(
            "<tr><td class=\"{}\">{status}</td><th scope=\"row\">{}</th><td>{}</td></tr>",
            status.to_lowercase(),
            html_escape(check.name),
            html_escape(&check.detail)
        ));
    }
    let schema = &report.schema;
    let html = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Doppelganger compatibility report</title><style>body{{font:14px system-ui,sans-serif;color:#172329;background:#fff;margin:0}}header{{background:#172329;color:#fff;padding:24px max(24px,calc((100vw - 1100px)/2))}}main{{max-width:1100px;margin:auto;padding:24px}}h1{{font-size:24px;margin:0 0 8px}}h2{{font-size:18px;margin:32px 0 12px}}p{{margin:6px 0}}code{{font-family:ui-monospace,monospace;overflow-wrap:anywhere}}table{{border-collapse:collapse;width:100%;text-align:left}}th,td{{padding:10px;border-bottom:1px solid #d9e0e3;vertical-align:top}}th{{font-weight:600}}td:last-child{{overflow-wrap:anywhere}}.pass{{color:#087348;font-weight:700}}.fail{{color:#b12b28;font-weight:700}}.summary{{font-size:16px;font-weight:650}}@media(max-width:700px){{th,td{{font-size:12px;padding:8px}}}}</style><header><h1>Doppelganger</h1><p>State migration compatibility report</p></header><main><p class=\"summary\">{} checks · {} failed</p><p>Program: <code>{}</code></p><p>Vault: <code>{}</code></p><h2>Schema</h2><table><tbody><tr><th>Account</th><td>{} ({}) → {} ({})</td></tr><tr><th>Added fields</th><td>{}</td></tr><tr><th>Removed fields</th><td>{}</td></tr><tr><th>Changed fields</th><td>{}</td></tr><tr><th>Field order changed</th><td>{}</td></tr><tr><th>Discriminator changed</th><td>{}</td></tr><tr><th>PDA seeds changed</th><td>{}</td></tr></tbody></table><h2>Checks</h2><table><thead><tr><th>Status</th><th>Check</th><th>Evidence</th></tr></thead><tbody>{rows}</tbody></table></main></html>",
        report.checks.len(), failures,
        html_escape(&report.program_id), html_escape(&report.account_address),
        html_escape(&schema.v1_account), schema.v1_bytes,
        html_escape(&schema.v2_account), schema.v2_bytes,
        html_escape(&schema.added_fields.join(", ")),
        html_escape(&schema.removed_fields.join(", ")),
        html_escape(&schema.changed_fields.join(", ")),
        schema.reordered_fields, schema.discriminator_changed, schema.pda_seeds_changed,
    );
    fs::write(path, html)?;
    Ok(())
}

fn write_project_html(path: &Path, report: &ProjectReport) -> Result<(), Box<dyn Error>> {
    let mut body = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} migration report</title><style>body{{font:14px system-ui,sans-serif;color:#172329;margin:0}}header{{background:#172329;color:white;padding:24px max(24px,calc((100vw - 1000px)/2))}}main{{max-width:1000px;margin:auto;padding:24px}}h1{{font-size:24px;margin:0}}h2{{font-size:18px;margin:28px 0 10px}}table{{border-collapse:collapse;width:100%;text-align:left}}th,td{{padding:9px;border-bottom:1px solid #d9e0e3;vertical-align:top;overflow-wrap:anywhere}}.pass{{color:#087348;font-weight:700}}.fail{{color:#b12b28;font-weight:700}}code{{overflow-wrap:anywhere}}</style><header><h1>{}</h1><p>State migration project report</p></header><main><p><strong>Coverage:</strong> {}</p><p><strong>Program:</strong> <code>{}</code></p><h2>Account Schemas</h2><table><thead><tr><th>V1 → V2</th><th>Added</th><th>Removed</th><th>Type changes</th><th>Reordered</th><th>Discriminator</th><th>PDA seeds</th></tr></thead><tbody>",
        html_escape(&report.name), html_escape(&report.name), report.coverage, html_escape(&report.program_id)
    );
    for account in &report.accounts {
        let pda = match account.pda_seeds_changed {
            Some(true) => "changed",
            Some(false) => "stable",
            None => "not checked",
        };
        write!(body, "<tr><th>{} → {}</th><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&account.v1_account), html_escape(&account.v2_account),
            html_escape(&account.added_fields.join(", ")), html_escape(&account.removed_fields.join(", ")),
            html_escape(&account.changed_fields.join(", ")), account.reordered_fields,
            account.discriminator_changed, pda)?;
    }
    body.push_str("</tbody></table>");
    if let Some(runtime) = &report.runtime {
        body.push_str("<h2>LiteSVM Checks</h2><table><thead><tr><th>Status</th><th>Check</th><th>Evidence</th></tr></thead><tbody>");
        for check in &runtime.checks {
            let status = if check.passed { "PASS" } else { "FAIL" };
            write!(
                body,
                "<tr><td class=\"{}\">{status}</td><th>{}</th><td>{}</td></tr>",
                status.to_lowercase(),
                html_escape(check.name),
                html_escape(&check.detail)
            )?;
        }
        body.push_str("</tbody></table>");
    } else {
        body.push_str(
            "<h2>Runtime</h2><p>No migration transaction was executed for this project.</p>",
        );
    }
    body.push_str("</main></html>");
    fs::write(path, body)?;
    Ok(())
}

fn check_project(name: &str, output: Option<PathBuf>) -> Result<bool, Box<dyn Error>> {
    let (manifest_path, project) = project::registered(name)?;
    let v1_idl = project.artifact(&manifest_path, &project.v1.idl)?;
    let v2_idl = project.artifact(&manifest_path, &project.v2.idl)?;
    let accounts =
        schema::compare_project(&v1_idl, &v2_idl, &project.program_id, &project.accounts)?;
    let runtime = if project.scenario.as_deref() == Some("vault") {
        if project.program_id != dg_vault_v1::id().to_string()
            || project.accounts.len() != 1
            || project.accounts[0].v1 != "VaultV1"
            || project.accounts[0].v2 != "VaultV2"
        {
            return Err(
                "vault scenario requires the bundled VaultV1 → VaultV2 fixture and program ID"
                    .into(),
            );
        }
        let v1_binary = project
            .v1
            .binary
            .as_ref()
            .ok_or("vault scenario needs V1 binary")?;
        let v2_binary = project
            .v2
            .binary
            .as_ref()
            .ok_or("vault scenario needs V2 binary")?;
        Some(run(
            project.artifact(&manifest_path, v1_binary)?,
            project.artifact(&manifest_path, v2_binary)?,
            v1_idl,
            v2_idl,
            &vault_migration_binding()?,
        )?)
    } else {
        if let Some(binary) = &project.v1.binary {
            project.artifact(&manifest_path, binary)?;
        }
        if let Some(binary) = &project.v2.binary {
            project.artifact(&manifest_path, binary)?;
        }
        None
    };
    let coverage = match runtime.as_ref() {
        Some(report) if report.passed() => "runtime passed",
        Some(_) => "runtime failed",
        None => "schema only; migration not executed",
    };
    let report = ProjectReport {
        name: project.name,
        program_id: project.program_id,
        coverage,
        accounts,
        runtime,
    };
    let output = output.unwrap_or_else(|| PathBuf::from(format!("report/{}.json", report.name)));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    let html = output.with_extension("html");
    write_project_html(&html, &report)?;
    println!("Project: {}", report.name);
    println!("Coverage: {}", coverage);
    for account in &report.accounts {
        println!("{} → {}: +{} -{} type_changes={} reordered={} discriminator_changed={} pda_seeds_changed={:?}",
            account.v1_account, account.v2_account,
            account.added_fields.len(), account.removed_fields.len(), account.changed_fields.len(),
            account.reordered_fields, account.discriminator_changed, account.pda_seeds_changed);
    }
    println!("Report: {}", output.display());
    println!("HTML: {}", html.display());
    Ok(report.runtime.as_ref().is_none_or(Report::passed))
}

fn run(
    v1_path: PathBuf,
    v2_path: PathBuf,
    v1_idl: PathBuf,
    v2_idl: PathBuf,
    migration: &InstructionBinding,
) -> Result<Report, Box<dyn Error>> {
    let mut svm = LiteSVM::new();
    let owner = Keypair::new();
    let stranger = Keypair::new();
    let program_id = dg_vault_v1::id();
    let (vault, _) =
        Address::find_program_address(&[b"vault", owner.pubkey().as_ref()], &program_id);
    svm.airdrop(&owner.pubkey(), 10_000_000_000)
        .map_err(|e| format!("owner funding failed: {e:?}"))?;
    svm.airdrop(&stranger.pubkey(), 1_000_000_000)
        .map_err(|e| format!("stranger funding failed: {e:?}"))?;
    svm.add_program_from_file(program_id, &v1_path)?;

    let init = instruction(
        dg_vault_v1::accounts::Initialize {
            vault,
            owner: owner.pubkey(),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        dg_vault_v1::instruction::Initialize {}.data(),
    );
    send(&mut svm, &owner, init).map_err(|e| format!("V1 initialize failed: {e:?}"))?;
    let deposit = instruction(
        dg_vault_v1::accounts::Deposit {
            vault,
            owner: owner.pubkey(),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        dg_vault_v1::instruction::Deposit { amount: DEPOSIT }.data(),
    );
    send(&mut svm, &owner, deposit).map_err(|e| format!("V1 deposit failed: {e:?}"))?;

    let before = svm.get_account(&vault).ok_or("V1 vault missing")?;
    let old = dg_vault_v1::VaultV1::try_deserialize(&mut before.data.as_slice())?;
    let rent: Rent = svm.get_sysvar();
    let mut report = Report {
        program_id: program_id.to_string(),
        account_address: vault.to_string(),
        schema: schema::compare(
            &v1_idl,
            &v2_idl,
            "VaultV1",
            "VaultV2",
            before.data.len(),
            8 + dg_vault_v2::VaultV2::INIT_SPACE,
            &program_id.to_string(),
        )?,
        checks: Vec::new(),
    };
    report.check(
        "v1_state_created",
        old.owner == owner.pubkey() && old.balance == DEPOSIT,
        format!("owner={}, balance={}", old.owner, old.balance),
    );
    report.check(
        "v1_sol_backing",
        before.lamports == old.balance + rent.minimum_balance(before.data.len()),
        format!(
            "vault_lamports={}, recorded_balance={}",
            before.lamports, old.balance
        ),
    );
    report.check(
        "program_owner",
        before.owner == program_id,
        format!("account_owner={}", before.owner),
    );
    report.check(
        "pda_seeds_stable",
        !report.schema.pda_seeds_changed,
        "V1 deposit and V2 deposit/migrate/withdraw use the same vault PDA seeds",
    );

    svm.add_program_from_file(program_id, &v2_path)?;
    let operate_accounts = dg_vault_v2::accounts::Operate {
        vault,
        owner: owner.pubkey(),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let direct_read = send(
        &mut svm,
        &owner,
        instruction(
            operate_accounts.clone(),
            dg_vault_v2::instruction::Withdraw { amount: 1 }.data(),
        ),
    );
    report.check(
        "direct_v2_read_rejected",
        anchor_error(&direct_read, "AccountDiscriminatorMismatch"),
        result_detail(&direct_read),
    );
    report.check(
        "failed_read_kept_state",
        svm.get_account(&vault)
            .as_ref()
            .is_some_and(|a| a.data == before.data && a.lamports == before.lamports),
        "V1 bytes and lamports remain unchanged",
    );

    let wrong_owner = send(
        &mut svm,
        &stranger,
        migration.build(&vault_migration_sources(vault, stranger.pubkey()))?,
    );
    report.check(
        "unauthorized_migration_rejected",
        anchor_error(&wrong_owner, "ConstraintSeeds"),
        result_detail(&wrong_owner),
    );
    report.check(
        "unauthorized_attempt_kept_state",
        svm.get_account(&vault)
            .as_ref()
            .is_some_and(|a| a.data == before.data && a.lamports == before.lamports),
        "V1 bytes and lamports remain unchanged",
    );

    let migrate = migration.build(&vault_migration_sources(vault, owner.pubkey()))?;
    let first_migration = send(&mut svm, &owner, migrate.clone());
    report.check(
        "migration_executed",
        first_migration.is_ok(),
        result_detail(&first_migration),
    );
    if first_migration.is_err() {
        return Ok(report);
    }
    let Some(after) = svm.get_account(&vault) else {
        report.check(
            "v2_account_present",
            false,
            "vault account missing after migration",
        );
        return Ok(report);
    };
    let new = match dg_vault_v2::VaultV2::try_deserialize(&mut after.data.as_slice()) {
        Ok(new) => {
            report.check(
                "v2_deserialization",
                true,
                "V2 reads migrated account bytes",
            );
            new
        }
        Err(error) => {
            report.check("v2_deserialization", false, error.to_string());
            return Ok(report);
        }
    };
    report.check(
        "account_resized",
        after.data.len() == report.schema.v2_bytes,
        format!(
            "before={} bytes, after={} bytes",
            before.data.len(),
            after.data.len()
        ),
    );
    report.check(
        "owner_preserved",
        new.owner == old.owner,
        format!("before={}, after={}", old.owner, new.owner),
    );
    report.check(
        "balance_preserved",
        new.balance == old.balance,
        format!("before={}, after={}", old.balance, new.balance),
    );
    report.check(
        "limit_initialized",
        new.withdrawal_limit == 500_000,
        format!("limit={}", new.withdrawal_limit),
    );
    report.check(
        "account_address_preserved",
        svm.get_account(&vault).is_some(),
        format!("pda={vault}"),
    );
    report.check(
        "sol_backing_preserved",
        after.lamports == new.balance + rent.minimum_balance(after.data.len()),
        format!(
            "vault_lamports={}, rent_reserve={}, balance={}",
            after.lamports,
            rent.minimum_balance(after.data.len()),
            new.balance
        ),
    );

    let repeated = send(&mut svm, &owner, migrate);
    report.check(
        "repeat_migration_rejected",
        anchor_error(&repeated, "AccountDiscriminatorMismatch"),
        result_detail(&repeated),
    );
    report.check(
        "repeat_attempt_kept_state",
        svm.get_account(&vault)
            .as_ref()
            .is_some_and(|a| a.data == after.data && a.lamports == after.lamports),
        "V2 bytes and lamports remain unchanged",
    );

    let over_limit = send(
        &mut svm,
        &owner,
        instruction(
            operate_accounts.clone(),
            dg_vault_v2::instruction::Withdraw { amount: 500_001 }.data(),
        ),
    );
    report.check(
        "withdrawal_limit_enforced",
        anchor_error(&over_limit, "LimitExceeded"),
        result_detail(&over_limit),
    );
    report.check(
        "failed_withdrawal_kept_state",
        svm.get_account(&vault)
            .as_ref()
            .is_some_and(|a| a.data == after.data && a.lamports == after.lamports),
        "V2 bytes and lamports remain unchanged",
    );

    let withdrawal = send(
        &mut svm,
        &owner,
        instruction(
            operate_accounts,
            dg_vault_v2::instruction::Withdraw { amount: WITHDRAW }.data(),
        ),
    );
    report.check(
        "v2_withdrawal_executed",
        withdrawal.is_ok(),
        result_detail(&withdrawal),
    );
    if withdrawal.is_ok() {
        let final_account = svm
            .get_account(&vault)
            .ok_or("vault missing after withdrawal")?;
        let final_state =
            match dg_vault_v2::VaultV2::try_deserialize(&mut final_account.data.as_slice()) {
                Ok(state) => state,
                Err(error) => {
                    report.check("post_withdrawal_deserialization", false, error.to_string());
                    return Ok(report);
                }
            };
        report.check(
            "post_migration_sol_backing",
            final_state.balance == DEPOSIT - WITHDRAW
                && final_account.lamports
                    == final_state.balance + rent.minimum_balance(final_account.data.len()),
            format!(
                "balance={}, vault_lamports={}",
                final_state.balance, final_account.lamports
            ),
        );
    }
    Ok(report)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("--version") {
        println!("doppelganger {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if matches!(command.as_deref(), Some("--help" | "-h") | None) {
        doctor::help();
        return Ok(());
    }
    if command.as_deref() == Some("doctor") {
        return doctor::command(args);
    }
    if command.as_deref() == Some("test") {
        return production::command(args);
    }
    if command.as_deref() == Some("snapshot") {
        return snapshot::command(args);
    }
    if command.as_deref() == Some("regression") {
        return regression::command(args);
    }
    if command.as_deref() == Some("runtime") {
        return runtime::command(args);
    }
    if command.as_deref() == Some("migration") {
        return migration_command(args);
    }
    if command.as_deref() == Some("analyze") {
        return preflight::direct(args);
    }
    if command.as_deref() == Some("project") {
        match args.next().as_deref() {
            Some("analyze") => {
                let name = args.next().ok_or("missing project name")?;
                let format = match args.next() {
                    Some(flag) if flag == "--format" => args.next().ok_or("--format needs a value")?,
                    Some(flag) => return Err(format!("unknown option {flag}").into()),
                    None => "text".to_owned(),
                };
                if args.next().is_some() { return Err("too many arguments".into()); }
                preflight::registered(&name,&format)?;
            }
            Some("add") => {
                let manifest = args.next().ok_or("missing manifest path")?;
                if args.next().is_some() {
                    return Err("too many arguments".into());
                }
                let name = project::add(Path::new(&manifest))?;
                println!("Added project: {}", name);
            }
            Some("recover") => {
                if args.next().is_some() { return Err("too many arguments".into()); }
                registry::recover()?; println!("Registry restored from validated backup");
            }
            Some("list") => {
                if args.next().is_some() {
                    return Err("too many arguments".into());
                }
                for (name, manifest) in project::list()? {
                    println!("{name}\t{}", manifest.display());
                }
            }
            Some("check") => {
                let name = args.next().ok_or("missing project name")?;
                let output = args.next().map(PathBuf::from);
                if args.next().is_some() {
                    return Err("too many arguments".into());
                }
                if !check_project(&name, output)? {
                    std::process::exit(1);
                }
            }
            _ => {
                return Err(
                    "usage: doppelganger project <add MANIFEST|list|analyze NAME [--format text|json|html]|check NAME [report.json]>"
                        .into(),
                )
            }
        }
        return Ok(());
    }
    if command.as_deref() != Some("demo") {
        eprintln!(
            "usage: doppelganger analyze --from V1_IDL_OR_DIR --to V2_IDL_OR_DIR [--account OLD:NEW] [--format text|json|html]\n       doppelganger runtime <build|migrate|verify|lifecycle> MANIFEST_OR_DIRECTORY [--output report.json]\n       doppelganger migration <inspect|check> [manifest-or-project-name]\n       doppelganger demo <v1.so> <v2.so> <v1-idl.json> <v2-idl.json> [report.json]\n       doppelganger project <add MANIFEST|list|analyze NAME [--format text|json|html]|check NAME [report.json]>"
        );
        std::process::exit(2);
    }
    let v1 = args.next().ok_or("missing V1 program path")?;
    let v2 = args.next().ok_or("missing V2 program path")?;
    let v1_idl = args.next().ok_or("missing V1 IDL path")?;
    let v2_idl = args.next().ok_or("missing V2 IDL path")?;
    let output = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "report/compatibility.json".to_string()),
    );
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let report = run(
        v1.into(),
        v2.into(),
        v1_idl.into(),
        v2_idl.into(),
        &vault_migration_binding()?,
    )?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    let html_output = output.with_extension("html");
    write_html(&html_output, &report)?;
    for check in &report.checks {
        println!(
            "{}  {}",
            if check.passed { "PASS" } else { "FAIL" },
            check.name
        );
    }
    println!("Report: {}", output.display());
    println!("HTML: {}", html_output.display());
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}
