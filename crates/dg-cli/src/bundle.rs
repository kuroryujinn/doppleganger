//! Portable, immutable reproduction of a complete production finding, including policy/graph failures.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Case {
    schema_version: u32,
    id: String,
    hashes: BTreeMap<String, String>,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn identity(hashes: &BTreeMap<String, String>) -> Result<String, Box<dyn Error>> {
    Ok(format!("CASE-{}", digest(&serde_json::to_vec(hashes)?)))
}
fn read(path: &Path) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(dg_analysis::read_bounded(path, 128 * 1024 * 1024)?)
}
fn normalized(report: &Value) -> Value {
    let scenarios=report["lifecycle"]["scenarios"].as_array().map(|items|items.iter().map(|s| {
        let steps=s["steps"].as_array().map(|steps|steps.iter().map(|step| {let mut step=step.clone();if let Some(object)=step.as_object_mut() { object.remove("program_swap"); }step}).collect::<Vec<_>>()).unwrap_or_default();
        json!({"id":s["id"],"definition":s["definition"],"status":s["status"],"diagnostics":s["diagnostics"],"steps":steps})
    }).collect::<Vec<_>>()).unwrap_or_default();
    json!({"result":report["result"],"exit_code":report["exit_code"],"analysis":report["analysis"]["findings"],"policy":report["policy"],"preservation":report["preservation"],"relationships":report["relationships"],"transaction":report["execution"]["transaction"],"before_accounts":report["execution"]["before_accounts"],"after_accounts":report["execution"]["after_accounts"],"lifecycle":scenarios})
}
fn run(directory: &Path) -> Value {
    crate::production::execute_paths(
        &directory.join("manifest.toml"),
        &directory.join("policy.toml"),
        directory
            .join("snapshot.json")
            .is_file()
            .then(|| directory.join("snapshot.json")),
    )
}
fn capture(report_path: &Path, directory: &Path) -> Result<Value, Box<dyn Error>> {
    let report_bytes = read(report_path)?;
    let report: Value = serde_json::from_slice(&report_bytes)?;
    if report["schema_version"] != 1
        || !matches!(report["exit_code"].as_i64(), Some(1 | 3))
        || !report["artifacts"].is_object()
    {
        return Err(
            "capture needs a complete version-1 failed production report with artifact identities"
                .into(),
        );
    }
    let roles = [
        ("manifest", "manifest.toml"),
        ("policy", "policy.toml"),
        ("v1_binary", "v1.bin"),
        ("v2_binary", "v2.bin"),
        ("v1_idl", "v1.json"),
        ("v2_idl", "v2.json"),
    ];
    let mut files = BTreeMap::new();
    for (role, name) in roles.into_iter().chain(
        report["artifacts"]["snapshot"]
            .is_object()
            .then_some(("snapshot", "snapshot.json")),
    ) {
        let source = report["artifacts"][role]["path"]
            .as_str()
            .ok_or("missing source artifact path")?;
        let data = read(Path::new(source))?;
        if digest(&data)
            != report["artifacts"][role]["sha256"]
                .as_str()
                .ok_or("missing source hash")?
        {
            return Err(format!("artifact changed since report: {role}").into());
        }
        files.insert(name.to_string(), data);
    }
    let mut manifest: toml::Value = toml::from_str(std::str::from_utf8(&files["manifest.toml"])?)?;
    for (version, binary, idl) in [("v1", "v1.bin", "v1.json"), ("v2", "v2.bin", "v2.json")] {
        manifest["versions"][version]["program_path"] = toml::Value::String(binary.into());
        manifest["versions"][version]["idl"] = toml::Value::String(idl.into());
    }
    files.insert(
        "manifest.toml".into(),
        toml::to_string(&manifest)?.into_bytes(),
    );
    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(directory)?;
    let result = (|| -> Result<Value, Box<dyn Error>> {
        for (name, data) in &files {
            fs::write(directory.join(name), data)?;
        }
        let fresh = run(directory);
        let expected = normalized(&report);
        if expected != normalized(&fresh) {
            return Err(
                "captured inputs no longer reproduce the report's normalized state and findings"
                    .into(),
            );
        }
        let data = serde_json::to_vec_pretty(&expected)?;
        fs::write(directory.join("expected.json"), &data)?;
        files.insert("expected.json".into(), data);
        let hashes = files
            .iter()
            .map(|(name, data)| (name.clone(), digest(data)))
            .collect::<BTreeMap<_, _>>();
        let case = Case {
            schema_version: 1,
            id: identity(&hashes)?,
            hashes,
        };
        fs::write(
            directory.join("case.json"),
            serde_json::to_vec_pretty(&case)?,
        )?;
        Ok(
            json!({"schema_version":1,"result":"CAPTURED","case_id":case.id,"directory":directory,"portable":true,"original_report_sha256":digest(&report_bytes)}),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(directory);
    }
    result
}
fn replay(directory: &Path, output: &Path) -> Result<Value, Box<dyn Error>> {
    let directory = directory.canonicalize()?;
    let case: Case = serde_json::from_slice(&read(&directory.join("case.json"))?)?;
    if case.schema_version != 1 || case.id != identity(&case.hashes)? {
        return Err("invalid reproduction case version or identity".into());
    }
    let mut required = vec![
        "expected.json",
        "manifest.toml",
        "policy.toml",
        "v1.bin",
        "v1.json",
        "v2.bin",
        "v2.json",
    ];
    if case.hashes.contains_key("snapshot.json") {
        required.push("snapshot.json");
    }
    required.sort();
    if required != case.hashes.keys().map(String::as_str).collect::<Vec<_>>() {
        return Err("invalid case file inventory".into());
    }
    for (name, expected) in &case.hashes {
        let path = directory.join(name).canonicalize()?;
        if !path.starts_with(&directory) || digest(&read(&path)?) != *expected {
            return Err(format!("case artifact integrity failed: {name}").into());
        }
    }
    let expected: Value = serde_json::from_slice(&read(&directory.join("expected.json"))?)?;
    let report = run(&directory);
    let matched = normalized(&report) == expected;
    crate::production::save_report(output, &report)?;
    Ok(
        json!({"schema_version":1,"case_id":case.id,"result":if matched {"REPRODUCED"}else{"MISMATCH"},"matched":matched,"migration_exit_code":report["exit_code"],"report":output.join("report.json")}),
    )
}
pub fn command(action: &str, mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let target = if action == "replay" {
        Some(PathBuf::from(
            args.next().ok_or("replay requires a case directory")?,
        ))
    } else {
        None
    };
    let mut options = BTreeMap::new();
    while let Some(flag) = args.next() {
        if !["--from-report", "--store", "--output"].contains(&flag.as_str()) {
            return Err(format!("unknown bundle option: {flag}").into());
        }
        let value = args
            .next()
            .filter(|v| !v.starts_with('-'))
            .ok_or("option needs a path")?;
        if options.insert(flag, value).is_some() {
            return Err("duplicate bundle option".into());
        }
    }
    let required = |key: &str| -> Result<&Path, Box<dyn Error>> {
        options
            .get(key)
            .map(|v| Path::new(v.as_str()))
            .ok_or_else(|| format!("missing {key}").into())
    };
    let result = if action == "capture" {
        if options.contains_key("--output") {
            return Err("capture accepts --from-report and --store".into());
        }
        capture(required("--from-report")?, required("--store")?)
    } else {
        if options.contains_key("--from-report") || options.contains_key("--store") {
            return Err("replay accepts a case directory and --output".into());
        }
        replay(target.as_ref().unwrap(), required("--output")?)
    };
    match result {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report["matched"] == false {
                std::process::exit(1);
            }
            Ok(())
        }
        Err(e) => {
            println!(
                "{}",
                json!({"schema_version":1,"result":"ERROR","category":"REGRESSION","message":e.to_string(),"suggested_correction":"Use an intact case and unchanged source artifacts; capture only complete production findings."})
            );
            std::process::exit(2)
        }
    }
}
