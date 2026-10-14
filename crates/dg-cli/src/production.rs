//! The production CLI joins existing engines; policy determines process success.
use dg_config::{addresses::AddressDefinition, LoadedProject, ProjectManifest};
use dg_policy::{
    relationships::{self, Account},
    Check, CheckStatus, Coverage, Profile,
};
use dg_runtime::{
    lifecycle::LifecycleEngine, preservation, MigrationExecutor, RuntimeWorldBuilder,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
struct Failure {
    code: String,
    category: String,
    context: String,
    message: String,
    exit_code: i32,
    evidence: Value,
    suggested_correction: String,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.context, self.message)
    }
}
impl Error for Failure {}
type Result<T> = std::result::Result<T, Box<Failure>>;
fn fail(
    category: &str,
    context: impl ToString,
    message: impl ToString,
    exit_code: i32,
) -> Box<Failure> {
    Box::new(Failure {
        code: format!("{category}_ERROR"),
        category: category.into(),
        context: context.to_string(),
        message: message.to_string(),
        exit_code,
        evidence: Value::Null,
        suggested_correction: match category {
            "CONFIG" => "Check schema versions, bindings, artifact paths and policy; run doppelganger doctor PROJECT.",
            "SNAPSHOT" => "Re-import a valid snapshot with matching program/IDL identity; do not edit its hashed contents.",
            "RUNTIME" => "Inspect transaction logs and account evidence; verify the configured instruction, signers and account funding.",
            _ => "Use a fresh writable output directory and stable input files; preserve this diagnostic when reporting a bug.",
        }.into(),
    })
}
fn engine_failure(code: &str, context: &str, message: &str) -> Box<Failure> {
    let configuration = matches!(
        code,
        "CONFIG"
            | "ADDRESS"
            | "IDL"
            | "PROGRAM_ID"
            | "INSTRUCTION"
            | "ARGUMENT"
            | "ACCOUNT_BINDING"
            | "MISSING_SIGNER"
    );
    let mut error = fail(
        if configuration { "CONFIG" } else { "RUNTIME" },
        context,
        message,
        if configuration { 2 } else { 3 },
    );
    error.code = code.into();
    error
}
struct Options {
    input: PathBuf,
    policy: Option<PathBuf>,
    snapshot: Option<PathBuf>,
    output: Option<PathBuf>,
    json: bool,
    quiet: bool,
    verbose: bool,
}
fn options(args: impl Iterator<Item = String>) -> Result<Options> {
    let mut args = args.peekable();
    let input = args.next().filter(|a| !a.starts_with('-')).ok_or_else(|| fail("CONFIG", "arguments", "test PROJECT [--policy FILE] [--snapshot FILE_OR_DIRECTORY] [--output NEW_DIRECTORY] [--json] [--ci]", 2))?;
    let mut o = Options {
        input: input.into(),
        policy: None,
        snapshot: None,
        output: None,
        json: false,
        quiet: false,
        verbose: false,
    };
    let mut seen = BTreeSet::new();
    while let Some(flag) = args.next() {
        if !seen.insert(flag.clone()) {
            return Err(fail("CONFIG", &flag, "duplicate option", 2));
        }
        match flag.as_str() {
            "--ci" => {}
            "--json" => o.json = true,
            "--quiet" => o.quiet = true,
            "--verbose" => o.verbose = true,
            "--policy" | "--snapshot" | "--output" => {
                let value = args
                    .next()
                    .filter(|a| !a.starts_with('-'))
                    .ok_or_else(|| fail("CONFIG", &flag, "option requires a path", 2))?;
                match flag.as_str() {
                    "--policy" => o.policy = Some(value.into()),
                    "--snapshot" => o.snapshot = Some(value.into()),
                    _ => o.output = Some(value.into()),
                }
            }
            _ => return Err(fail("CONFIG", &flag, "unknown option", 2)),
        }
    }
    if o.json && o.quiet {
        return Err(fail(
            "CONFIG",
            "arguments",
            "--json and --quiet are mutually exclusive",
            2,
        ));
    }
    Ok(o)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn serialize<T: Serialize>(data: &T) -> Result<Value> {
    serde_json::to_value(data).map_err(|e| fail("INTERNAL", "serialization", e, 4))
}
fn artifact(path: &Path) -> Result<Value> {
    let path = path
        .canonicalize()
        .map_err(|e| fail("CONFIG", path.display(), e, 2))?;
    let bytes = dg_analysis::read_bounded(&path, 128 * 1024 * 1024)
        .map_err(|e| fail("CONFIG", path.display(), e, 2))?;
    Ok(json!({"path": path,"sha256":digest(&bytes)}))
}
fn artifacts(
    loaded: &LoadedProject,
    policy: &Path,
    snapshot: Option<&Path>,
) -> Result<BTreeMap<String, Value>> {
    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("manifest", loaded.manifest_path.as_path()),
        ("v1_binary", &loaded.v1_program_path),
        ("v2_binary", &loaded.v2_program_path),
        ("v1_idl", &loaded.v1_idl_path),
        ("v2_idl", &loaded.v2_idl_path),
        ("policy", policy),
    ] {
        artifacts.insert(name.into(), artifact(path)?);
    }
    if let Some(path) = snapshot {
        artifacts.insert("snapshot".into(), artifact(path)?);
    }
    Ok(artifacts)
}
fn account(e: &dg_runtime::migration::AccountEvidence) -> Account {
    Account {
        name: e.name.clone(),
        address: e.address.clone(),
        owner: e.snapshot.as_ref().map(|s| s.owner.clone()),
        exists: e.snapshot.is_some(),
        fields: e.decoded.as_ref().map(|d| d.fields.clone()),
        decode_error: e.decode_error.as_ref().map(ToString::to_string),
    }
}
fn json_accounts(v: &Value) -> Result<Vec<Account>> {
    v.as_array()
        .ok_or_else(|| {
            fail(
                "INTERNAL",
                "snapshot.execution",
                "account evidence is not an array",
                4,
            )
        })?
        .iter()
        .map(|a| {
            Ok(Account {
                name: a["name"]
                    .as_str()
                    .ok_or_else(|| fail("INTERNAL", "account.name", "missing name", 4))?
                    .into(),
                address: a["address"]
                    .as_str()
                    .ok_or_else(|| fail("INTERNAL", "account.address", "missing address", 4))?
                    .into(),
                owner: a["snapshot"]["owner"].as_str().map(str::to_owned),
                exists: !a["snapshot"].is_null(),
                fields: if a["decoded"].is_null() {
                    None
                } else {
                    Some(
                        serde_json::from_value(a["decoded"]["fields"].clone())
                            .map_err(|e| fail("INTERNAL", "account.fields", e, 4))?,
                    )
                },
                decode_error: (!a["decode_error"].is_null()).then(|| a["decode_error"].to_string()),
            })
        })
        .collect()
}
fn checks(preservation: &Value, lifecycle: &Value) -> Result<Vec<Check>> {
    let mut checks = vec![];
    let evaluations = preservation["evaluations"]
        .as_array()
        .ok_or_else(|| fail("INTERNAL", "preservation", "missing evaluations", 4))?;
    for e in evaluations {
        checks.push(Check {
            id: format!("PRESERVE-{}", e["rule_id"].as_str().unwrap_or("unknown")),
            category: "PRESERVATION".into(),
            status: status(&e["status"])?,
            message: e["explanation"]
                .as_str()
                .unwrap_or("Preservation check")
                .into(),
        });
    }
    let scenarios = lifecycle["scenarios"]
        .as_array()
        .ok_or_else(|| fail("INTERNAL", "lifecycle", "missing scenarios", 4))?;
    for s in scenarios {
        checks.push(Check {
            id: s["id"].as_str().unwrap_or("unknown").into(),
            category: "LIFECYCLE".into(),
            status: status(&s["status"])?,
            message: format!("Lifecycle scenario {}: {}", s["id"], s["status"]),
        });
    }
    Ok(checks)
}
fn status(v: &Value) -> Result<CheckStatus> {
    match v.as_str() {
        Some("PASS") => Ok(CheckStatus::Pass),
        Some("FAIL") => Ok(CheckStatus::Fail),
        Some("ERROR") => Ok(CheckStatus::Error),
        Some("NOT_APPLICABLE") => Ok(CheckStatus::NotApplicable),
        _ => Err(fail(
            "INTERNAL",
            "check.status",
            "unknown evaluation status",
            4,
        )),
    }
}
fn execute(o: &Options) -> Result<Value> {
    let loaded =
        ProjectManifest::load(&o.input).map_err(|e| fail("CONFIG", e.field, e.message, 2))?;
    let base = loaded
        .manifest_path
        .parent()
        .ok_or_else(|| fail("CONFIG", "manifest", "missing parent directory", 2))?;
    let policy_path = o
        .policy
        .clone()
        .unwrap_or_else(|| base.join("doppelganger-policy.toml"));
    let text = dg_analysis::read_bounded(&policy_path, 4 * 1024 * 1024)
        .and_then(|b| String::from_utf8(b).map_err(std::io::Error::other))
        .map_err(|e| fail("CONFIG", policy_path.display(), e, 2))?;
    let profile = Profile::parse(&text).map_err(|e| fail("CONFIG", "policy", e, 2))?;
    for (name, definition) in &loaded.manifest.addresses {
        if matches!(
            definition,
            AddressDefinition::Signer {
                test_seed: None,
                ..
            }
        ) {
            return Err(fail(
                "CONFIG",
                name,
                "Production tests require deterministic local test identities (test_seed)",
                2,
            ));
        }
    }
    let snapshot_path = o.snapshot.as_ref().map(|p| {
        if p.is_dir() {
            p.join("snapshot.json")
        } else {
            p.clone()
        }
    });
    let input_artifacts = artifacts(&loaded, &policy_path, snapshot_path.as_deref())?;
    let mut mappings = BTreeMap::new();
    for target in MigrationExecutor::targets(&loaded.manifest) {
        if let (Some(old), Some(new)) = (target.v1_type, target.v2_type) {
            if mappings.get(&old).is_some_and(|prior| prior != &new) {
                return Err(fail(
                    "CONFIG",
                    "migration_targets",
                    "conflicting account type mappings",
                    2,
                ));
            }
            if !loaded.v1.accounts.contains_key(&old) || !loaded.v2.accounts.contains_key(&new) {
                return Err(fail(
                    "CONFIG",
                    "migration_targets",
                    "unknown account schema mapping",
                    2,
                ));
            }
            mappings.insert(old, new);
        }
    }
    let mappings: Vec<_> = mappings.into_iter().collect();
    let analysis = dg_analysis::analyze(
        &loaded.manifest.project.name,
        &loaded.v1,
        &loaded.v2,
        &mappings,
    );
    let (execution, preservation, lifecycle, before, after, bindings, source) =
        if let Some(path) = &snapshot_path {
            let snapshot =
                dg_snapshot::load(path).map_err(|e| fail("CONFIG", e.context, e.message, 2))?;
            let built = dg_snapshot::replay::build_world(&snapshot, &loaded.manifest, base)
                .map_err(|e| fail("RUNTIME", e.context, e.message, 3))?;
            let bindings = built.world.resolved.strings();
            let report = dg_snapshot::replay::replay(
                &snapshot,
                &loaded.manifest,
                base,
                &Default::default(),
                true,
            )
            .map_err(|e| fail("RUNTIME", e.context, e.message, 3))?;
            let before = json_accounts(&report["execution"]["before_accounts"])?;
            let after = json_accounts(&report["execution"]["after_accounts"])?;
            (
                report["execution"].clone(),
                report["preservation"].clone(),
                report["lifecycle"].clone(),
                before,
                after,
                bindings,
                json!({"kind":"snapshot","snapshot":snapshot,"remote_transactions_sent":false}),
            )
        } else {
            let mut built = RuntimeWorldBuilder::default()
                .build_manifest(&loaded.manifest, base)
                .map_err(|e| {
                    let mut f = engine_failure(&e.code, &e.context, &e.message);
                    f.evidence = serialize(e.as_ref()).unwrap_or(Value::Null);
                    f
                })?;
            let execution = MigrationExecutor::default()
                .execute_manifest(&mut built.world, &loaded.manifest, base)
                .map_err(|e| {
                    let mut f = engine_failure(&e.code, &e.context, &e.message);
                    f.evidence = serialize(e.as_ref()).unwrap_or(Value::Null);
                    f
                })?;
            let preservation = preservation::evaluate(
                &execution,
                &loaded.manifest.preservation,
                &built.world.model,
            )
            .map_err(|e| fail("CONFIG", e.field, e.message, 2))?;
            let lifecycle = LifecycleEngine
                .run(&loaded.manifest, base)
                .map_err(|e| fail("CONFIG", e.field, e.message, 2))?;
            let before = execution.before_accounts.iter().map(account).collect();
            let after = execution.after_accounts.iter().map(account).collect();
            (
                serialize(&execution)?,
                serialize(&preservation)?,
                serialize(&lifecycle)?,
                before,
                after,
                built.world.resolved.strings(),
                json!({"kind":"fixture","remote_transactions_sent":false}),
            )
        };
    let pdas = loaded
        .manifest
        .addresses
        .iter()
        .filter_map(|(name, d)| matches!(d, AddressDefinition::Pda { .. }).then_some(name.clone()))
        .collect();
    let relationships =
        relationships::evaluate(&profile.relationship, &before, &after, &bindings, &pdas)
            .map_err(|e| fail("CONFIG", "relationships", e, 2))?;
    let mut checks = checks(&preservation, &lifecycle)?;
    checks.push(Check {
        id: "MIGRATION".into(),
        category: "MIGRATION".into(),
        status: if execution["transaction"]["success"] == true {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        message: "Configured migration must succeed".into(),
    });
    checks.extend(relationships.edges.iter().map(|e| Check {
        id: e.id.clone(),
        category: "RELATIONSHIP".into(),
        status: e.status,
        message: e.message.clone(),
    }));
    let coverage = Coverage {
        preservation: loaded.manifest.preservation.len(),
        relationships: profile.relationship.len(),
        lifecycle: loaded.manifest.scenario.len(),
    };
    let policy = dg_policy::evaluate(&profile, &analysis.findings, &checks, coverage);
    if artifacts(&loaded, &policy_path, snapshot_path.as_deref())? != input_artifacts {
        return Err(fail(
            "INTERNAL",
            "artifacts",
            "Inputs changed during execution; result is invalid",
            4,
        ));
    }
    let content_hashes: BTreeMap<_, _> = input_artifacts
        .iter()
        .map(|(k, v)| (k, &v["sha256"]))
        .collect();
    let run_id = format!(
        "RUN-{}",
        digest(
            &serde_json::to_vec(&json!({"workflow_version":1,"engine_version":env!("CARGO_PKG_VERSION"),"artifacts":content_hashes}))
                .map_err(|e| fail("INTERNAL", "run_id", e, 4))?
        )
    );
    let exit_code = if policy.evaluation_errors > 0 {
        3
    } else if policy.passed {
        0
    } else {
        1
    };
    let mut report = json!({"schema_version":1,"engine_version":env!("CARGO_PKG_VERSION"),"workflow_version":1,"manifest_schema_version":loaded.manifest.schema_version,"legacy_unversioned_manifest":loaded.manifest.schema_version.is_none(),"run_id":run_id,"project":loaded.manifest.project.name,"result":if policy.passed {"PASS"} else if policy.evaluation_errors>0 {"ERROR"} else {"FAIL"},"exit_code":exit_code,"analysis":analysis,"execution":execution,"preservation":preservation,"relationships":relationships,"lifecycle":lifecycle,"policy":policy,"artifacts":input_artifacts,"source":source,"migration_executed_locally":true,"preservation_evaluated":true,"lifecycle_evaluated":true,"policy_evaluated":true,"remote_transactions_sent":false,"rpc_write_calls":0});
    // Keep the R11 top-level snapshot contract so R10 can reproduce this report.
    if let Some(snapshot) = report["source"]
        .as_object_mut()
        .and_then(|s| s.remove("snapshot"))
    {
        report["snapshot"] = snapshot;
    }
    Ok(report)
}
fn write_reports(directory: &Path, report: &Value) -> Result<()> {
    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent).map_err(|e| fail("INTERNAL", parent.display(), e, 4))?;
    }
    // Each result gets a fresh directory; never overwrite snapshots, inputs, or prior runs.
    fs::create_dir(directory).map_err(|e| {
        fail(
            "INTERNAL",
            directory.display(),
            format!("output must be a new directory: {e}"),
            4,
        )
    })?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|e| fail("INTERNAL", "report", e, 4))?;
    let summary=format!("# Doppelganger migration test\n\nResult: {}\n\nExit code: {}\n\nProject: {}\n\nBlocking findings: {}\n\nWarnings: {}\n\nExpected changes: {}\n\nSee report.json for complete evidence.\n",report["result"].as_str().unwrap_or("ERROR"),report["exit_code"],report["project"].as_str().unwrap_or(""),report["policy"]["blocking"],report["policy"]["warnings"],report["policy"]["expected"]);
    let mut decisions = String::new();
    if let Some(items) = report["policy"]["decisions"].as_array() {
        for d in items {
            decisions.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                crate::html_escape(d["id"].as_str().unwrap_or("")),
                crate::html_escape(d["category"].as_str().unwrap_or("")),
                crate::html_escape(d["action"].as_str().unwrap_or("")),
                crate::html_escape(d["message"].as_str().unwrap_or(""))
            ));
        }
    }
    let html=format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Doppelganger migration test</title><style>body{{font:16px system-ui;max-width:1100px;margin:32px auto;padding:0 20px}}table{{border-collapse:collapse;width:100%}}td,th{{text-align:left;padding:8px;border:1px solid #ccc}}pre{{white-space:pre-wrap;overflow-wrap:anywhere}}</style><h1>Migration test: {}</h1><p>{}</p><p>Blocking: {} · Warnings: {} · Expected changes: {}</p><table><thead><tr><th>ID</th><th>Category</th><th>Decision</th><th>Explanation</th></tr></thead><tbody>{decisions}</tbody></table><details><summary>Complete before/after evidence and diagnostics</summary><pre>{}</pre></details></html>",crate::html_escape(report["result"].as_str().unwrap_or("ERROR")),crate::html_escape(report["project"].as_str().unwrap_or("")),report["policy"]["blocking"],report["policy"]["warnings"],report["policy"]["expected"],crate::html_escape(&String::from_utf8_lossy(&bytes)));
    for (name, data) in [
        ("report.json", bytes),
        ("summary.md", summary.into_bytes()),
        ("report.html", html.into_bytes()),
    ] {
        fs::write(directory.join(name), data)
            .map_err(|e| fail("INTERNAL", directory.display(), e, 4))?;
    }
    Ok(())
}
pub fn command(args: impl Iterator<Item = String>) -> std::result::Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = args.collect();
    if arguments == ["--help"] || arguments == ["-h"] {
        crate::doctor::help();
        return Ok(());
    }
    let o = match options(arguments.into_iter()) {
        Ok(o) => o,
        Err(e) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"schema_version":1,"result":"ERROR","exit_code":e.exit_code,"diagnostics":[e]})
                )?
            );
            std::process::exit(2)
        }
    };
    if o.verbose {
        eprintln!("Validating inputs and executing local migration, preservation, relationships, lifecycle and policy: {}", o.input.display());
    }
    let report = match execute(&o) {
        Ok(r) => r,
        Err(e) => {
            json!({"schema_version":1,"result":"ERROR","exit_code":e.exit_code,"diagnostics":[e]})
        }
    };
    let directory = o.output.unwrap_or_else(|| {
        Path::new(".doppelganger/runs").join(format!(
            "{}-{}",
            report["run_id"].as_str().unwrap_or("ERROR"),
            std::process::id()
        ))
    });
    if let Err(e) = write_reports(&directory, &report) {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"schema_version":1,"result":"ERROR","exit_code":4,"diagnostics":[e]})
            )?
        );
        std::process::exit(4);
    }
    if o.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if !o.quiet {
        println!(
            "DOPPELGANGER MIGRATION TEST\nResult: {}\nExit code: {}\nReport: {}",
            report["result"].as_str().unwrap_or("ERROR"),
            report["exit_code"],
            directory.join("report.json").display()
        );
    }
    if o.verbose {
        eprintln!(
            "Report saved: {}. Blocking: {}; evaluation errors: {}",
            directory.display(),
            report["policy"]["blocking"],
            report["policy"]["evaluation_errors"]
        );
    }
    let code = report["exit_code"].as_i64().unwrap_or(4) as i32;
    if code != 0 {
        if o.quiet {
            eprintln!(
                "Migration test {} (exit {code}); evidence: {}",
                report["result"],
                directory.join("report.json").display()
            );
        }
        std::process::exit(code)
    }
    Ok(())
}

pub(crate) fn execute_paths(input: &Path, policy: &Path, snapshot: Option<PathBuf>) -> Value {
    let options = Options {
        input: input.into(),
        policy: Some(policy.into()),
        snapshot,
        output: None,
        json: false,
        quiet: true,
        verbose: false,
    };
    execute(&options).unwrap_or_else(
        |e| json!({"schema_version":1,"result":"ERROR","exit_code":e.exit_code,"diagnostics":[e]}),
    )
}
pub(crate) fn save_report(
    directory: &Path,
    report: &Value,
) -> std::result::Result<(), Box<dyn Error>> {
    write_reports(directory, report).map_err(|e| e as Box<dyn Error>)
}
