use dg_runtime::{MigrationExecutor, Phase, RuntimeWorldBuilder};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

pub fn command(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let action = args.next().unwrap_or_default();
    if !matches!(
        action.as_str(),
        "build" | "migrate" | "verify" | "lifecycle"
    ) {
        return Err(
            "usage: doppelganger runtime build|migrate|verify|lifecycle MANIFEST_OR_DIRECTORY [--output report.json]".into(),
        );
    }
    let input = args
        .next()
        .ok_or("runtime requires a manifest or directory")?;
    let output = match args.next() {
        Some(flag) if flag == "--output" => {
            Some(PathBuf::from(args.next().ok_or("--output needs a path")?))
        }
        Some(flag) => return Err(format!("unknown runtime option {flag}").into()),
        None => None,
    };
    if args.next().is_some() {
        return Err("too many runtime arguments".into());
    }
    let path = Path::new(&input);
    let path = if path.is_dir() {
        path.join("doppelganger.toml")
    } else {
        path.to_owned()
    };
    let manifest = dg_config::ProjectManifest::parse(&fs::read_to_string(&path)?)?;
    let base = path.parent().unwrap_or(Path::new("."));
    if action == "lifecycle" {
        let report = dg_runtime::lifecycle::LifecycleEngine.run(&manifest, base)?;
        println!(
            "DOPPELGANGER LIFECYCLE TEST\nProject: {}\nScenarios: {}",
            report.project, report.total
        );
        for scenario in &report.scenarios {
            println!(
                "{}  {}",
                serde_json::to_value(scenario.status)?
                    .as_str()
                    .unwrap_or("ERROR"),
                scenario.id
            );
            for diagnostic in scenario
                .diagnostics
                .iter()
                .chain(scenario.steps.iter().flat_map(|s| &s.diagnostics))
            {
                println!("  {}: {}", diagnostic.code, diagnostic.message);
            }
        }
        println!("Passed: {}\nFailed: {}\nErrors: {}\nPreservation: not evaluated by lifecycle engine\nPolicy: not evaluated", report.passed, report.failed, report.errors);
        if let Some(path) = output {
            write_report(&path, &serde_json::to_vec_pretty(&report)?)?;
            println!("Lifecycle report: {}", path.display());
        }
        return Ok(());
    }
    let mut result = match RuntimeWorldBuilder::default().build_manifest(&manifest, base) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{}", serde_json::to_string_pretty(&error)?);
            return Err(error);
        }
    };
    if action != "build" {
        let execution =
            match MigrationExecutor::default().execute_manifest(&mut result.world, &manifest, base)
            {
                Ok(execution) => execution,
                Err(error) => {
                    let json = serde_json::to_vec_pretty(&error)?;
                    if let Some(path) = &output {
                        write_report(path, &json)?;
                    }
                    eprintln!("{}", String::from_utf8_lossy(&json));
                    eprintln!("Preservation evaluation: not executed.");
                    return Err(error);
                }
            };
        let transaction = execution
            .transaction
            .as_ref()
            .ok_or("missing transaction evidence")?;
        if action == "verify" {
            let preservation = dg_runtime::preservation::evaluate(
                &execution,
                &manifest.preservation,
                &result.world.model,
            )?;
            println!("DOPPELGANGER PRESERVATION CHECK\nProject: {}\nMigration: {}\nRules evaluated: {}\nPassed: {}\nFailed: {}\nErrors: {}\nNot applicable: {}", preservation.project, if transaction.success {"success"} else {"program error"}, preservation.rules_total, preservation.passed, preservation.failed, preservation.errors, preservation.not_applicable);
            for finding in &preservation.findings {
                let e = &finding.evaluation;
                println!("{}\n  Account: {}\n  Field: {}\n  Rule: {:?}\n  Status: {:?}\n  Category: {}\n  Before: {}\n  After: {}\n  Expected: {}\n  {}", finding.id,e.account,e.field.as_deref().unwrap_or("(account)"),e.rule,e.status,e.category.as_deref().unwrap_or(""),serde_json::to_string(&e.before)?,serde_json::to_string(&e.after)?,serde_json::to_string(&e.expected)?,e.explanation);
            }
            if let Some(path) = output {
                let report = serde_json::json!({"schema_version":1,"project":preservation.project,"execution":execution,"preservation":preservation,"preservation_evaluated":true,"lifecycle_evaluated":false,"policy_evaluated":false});
                write_report(&path, &serde_json::to_vec_pretty(&report)?)?;
                println!("Preservation report: {}", path.display());
            }
            println!("Migration execution: completed\nLifecycle testing: not executed\nPolicy evaluation: not executed");
            return Ok(());
        }
        println!("DOPPELGANGER MIGRATION EXECUTION\nProject: {}\nV1 world: built\nCandidate V2: loaded\nMigration: {}\nResult: {}", execution.project, transaction.instruction_name, if transaction.success {"success"} else {"program error"});
        if let Some(error) = &transaction.error {
            println!("Program error: {error}");
        }
        println!("Observed accounts: {}", execution.after_accounts.len());
        for (before, after) in execution
            .before_accounts
            .iter()
            .zip(&execution.after_accounts)
        {
            println!("{}", before.name);
            for (label, account) in [("before", before), ("after", after)] {
                let size = account
                    .snapshot
                    .as_ref()
                    .map(|s| s.data_length.to_string())
                    .unwrap_or_else(|| "absent".into());
                let decode = account
                    .decoded
                    .as_ref()
                    .map(|d| d.account_type.as_str())
                    .unwrap_or(if account.decode_error.is_some() {
                        "failed (raw retained)"
                    } else {
                        "not decoded"
                    });
                println!("  {label} length: {size}\n  {label} decode: {decode}");
            }
        }
        if let Some(path) = output {
            write_report(&path, &serde_json::to_vec_pretty(&execution)?)?;
            println!("Execution report: {}", path.display());
        }
        println!("Preservation evaluation: not executed.");
        return Ok(());
    }
    println!(
        "DOPPELGANGER RUNTIME WORLD\nProject: {input}\nProgram: {} (V1 loaded)",
        result.world.artifact.program_id
    );
    println!("Signers: {} funded identities", result.world.funded_signers);
    for (phase, label) in [
        (Phase::Initialize, "Initialization"),
        (Phase::Setup, "Setup"),
    ] {
        let records: Vec<_> = result
            .executed_actions
            .iter()
            .filter(|record| record.phase == phase)
            .collect();
        println!(
            "{label} actions: {}/{} passed",
            records.iter().filter(|record| record.success).count(),
            records.len()
        );
    }
    println!("Observed accounts: {}", result.observed_accounts.len());
    for account in &result.observed_accounts {
        println!("{}\n  address: {}\n  owner: {}\n  lamports: {}\n  data length: {}\n  snapshot: captured",account.binding.name,account.snapshot.address,account.snapshot.owner,account.snapshot.lamports,account.snapshot.data_length);
        if let Some(error) = &account.decode_error {
            println!("  decode: {error}");
        } else {
            println!("  decode: {}", account.binding.account_type);
        }
    }
    if let Some(path) = output {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &path,
            serde_json::to_vec_pretty(
                &serde_json::json!({"schema_version":1,"migration_executed":false,"program":result.world.artifact,"executed_actions":result.executed_actions,"observed_accounts":result.observed_accounts}),
            )?,
        )?;
        println!("Snapshot report: {}", path.display());
    }
    println!("No migration executed.");
    Ok(())
}

fn write_report(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}
