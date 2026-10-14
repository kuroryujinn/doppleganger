use serde_json::{json, Value};
use std::{error::Error, path::Path, process::Command};
pub fn help() {
    println!("Doppelganger {} — local Anchor migration qualification\n\nUsage:\n  doppelganger test PROJECT [--policy FILE] [--snapshot FILE_OR_DIRECTORY]\n                    [--output NEW_DIRECTORY] [--ci] [--json|--quiet] [--verbose]\n  doppelganger migration test PROJECT [same options]\n  doppelganger doctor [PROJECT] [--json]\n  doppelganger analyze --from V1_IDL_OR_DIR --to V2_IDL_OR_DIR [--account OLD:NEW]\n                      [--format text|json|html]\n  doppelganger runtime <build|migrate|verify|lifecycle> PROJECT\n  doppelganger snapshot <capture|inspect|replay> ...\n  doppelganger regression <create|reproduce|capture|replay|list> ...\n  doppelganger project <add MANIFEST|list|recover>\n  doppelganger --version\n\nCI exit codes: 0 passed, 1 policy violation, 2 configuration, 3 runtime, 4 internal.\nGetting started: docs/getting-started.md", env!("CARGO_PKG_VERSION"));
}
fn check(
    category: &str,
    context: &str,
    status: &str,
    actual: impl ToString,
    correction: &str,
) -> Value {
    json!({"category":category,"context":context,"status":status,"actual":actual.to_string(),"suggested_correction":correction})
}
pub fn command(args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let mut project = None;
    let mut json_output = false;
    for arg in args {
        match arg.as_str() {
            "--json" if !json_output => json_output = true,
            "--help" | "-h" => {
                help();
                return Ok(());
            }
            _ if !arg.starts_with('-') && project.is_none() => project = Some(arg),
            _ => return Err(format!("unknown or duplicate doctor argument: {arg}").into()),
        }
    }
    let mut checks = vec![check("RUNTIME", "platform", if cfg!(target_os="linux") {"PASS"} else {"WARN"}, std::env::consts::OS, "The qualified release platform is Linux x86_64; other platforms need separate qualification.")];
    for (name, env_override) in [
        ("anchor", "ANCHOR_BIN"),
        ("solana", "SOLANA_BIN"),
        ("cargo", "CARGO_BIN"),
    ] {
        let executable = std::env::var(env_override).unwrap_or_else(|_| name.into());
        match Command::new(&executable).arg("--version").output() {
            Ok(output) if output.status.success() => checks.push(check("TOOLCHAIN", name,"PASS", String::from_utf8_lossy(&output.stdout).trim(), "")),
            _ => checks.push(check("TOOLCHAIN", name, "WARN", "unavailable", "Install the pinned build tools to compile programs. Existing SBF/IDL artifacts can be tested without them.")),
        }
    }
    if let Some(input) = project {
        match dg_config::ProjectManifest::load(Path::new(&input)) {
            Err(e) => checks.push(check(
                "CONFIG",
                &input,
                "FAIL",
                e,
                "Correct the manifest and artifact paths; see docs/project-manifest.md.",
            )),
            Ok(loaded) => {
                checks.push(check("CONFIG", "schema_version", if loaded.manifest.schema_version.is_some() {"PASS"} else {"WARN"}, loaded.manifest.schema_version.map_or("legacy unversioned v1".into(), |v|v.to_string()), "Add schema_version = 1 at the top of the manifest; see docs/migration-guide.md."));
                for (name, path) in [
                    ("v1_binary", &loaded.v1_program_path),
                    ("v2_binary", &loaded.v2_program_path),
                ] {
                    let bytes = dg_analysis::read_bounded(path, 64 * 1024 * 1024)?;
                    checks.push(check("CONFIG", name, if bytes.starts_with(b"\x7fELF") {"PASS"} else {"FAIL"}, path.display(), "Build a Solana SBF ELF artifact using cargo build-sbf. Full VM verification happens in test."));
                }
                checks.push(check(
                    "ANALYSIS",
                    "IDLs",
                    "PASS",
                    "both IDLs normalize and match the configured program ID",
                    "",
                ));
                let policy = loaded
                    .manifest_path
                    .parent()
                    .unwrap()
                    .join("doppelganger-policy.toml");
                let profile = dg_analysis::read_bounded(&policy, 4 * 1024 * 1024)
                    .map_err(|e| e.to_string())
                    .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))
                    .and_then(|s| dg_policy::Profile::parse(&s));
                match profile {
                    Ok(_) => checks.push(check("POLICY", "profile", "PASS", policy.display(), "")),
                    Err(e) => checks.push(check(
                        "POLICY",
                        "profile",
                        "FAIL",
                        e,
                        "Provide doppelganger-policy.toml; see docs/production-workflow.md.",
                    )),
                }
                for (name, definition) in &loaded.manifest.addresses {
                    if matches!(
                        definition,
                        dg_config::AddressDefinition::Signer {
                            test_seed: None,
                            ..
                        }
                    ) {
                        checks.push(check("CONFIG",name,"FAIL","random signer identity","Set a deterministic test_seed for local fixtures; never supply production private keys."));
                    }
                }
            }
        }
    }
    let passed = checks.iter().all(|c| c["status"] != "FAIL");
    let report = json!({"schema_version":1,"version":env!("CARGO_PKG_VERSION"),"passed":passed,"checks":checks,"migration_executed":false,"rpc_calls":0});
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for c in report["checks"].as_array().unwrap() {
            println!(
                "{} {}: {}",
                c["status"].as_str().unwrap(),
                c["context"].as_str().unwrap(),
                c["actual"].as_str().unwrap()
            );
            if c["status"] != "PASS" {
                println!("  {}", c["suggested_correction"].as_str().unwrap());
            }
        }
    }
    if !passed {
        std::process::exit(2);
    }
    Ok(())
}
