use dg_snapshot::{
    replay::{self, CandidateOverride},
    rpc::HttpRpc,
    SnapshotError,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

fn invalid(message: impl ToString) -> SnapshotError {
    SnapshotError {
        code: "CONFIG".into(),
        context: "arguments".into(),
        message: message.to_string(),
    }
}

pub fn command(args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match run(args) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        Err(e) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"schema_version":1,"result":"ERROR","diagnostics":[e]})
                )?
            );
            Err(e.into())
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> dg_snapshot::Result<Value> {
    let action = args
        .next()
        .ok_or_else(|| invalid("expected capture, inspect, or replay"))?;
    if action == "--help" {
        return Ok(json!({"usage":[
            "snapshot capture PROJECT [--output DIRECTORY]",
            "snapshot inspect SNAPSHOT_OR_ID [--store DIRECTORY]",
            "snapshot replay SNAPSHOT_OR_ID --project PROJECT [--candidate BINARY] [--candidate-idl IDL] [--lifecycle] [--output REPORT] [--store DIRECTORY]"
        ]}));
    }
    let target = args
        .next()
        .filter(|s| !s.starts_with("--"))
        .ok_or_else(|| invalid("missing project or snapshot"))?;
    let allowed: &[&str] = match action.as_str() {
        "capture" => &["--output"],
        "inspect" => &["--store"],
        "replay" => &[
            "--project",
            "--candidate",
            "--candidate-idl",
            "--output",
            "--store",
            "--lifecycle",
        ],
        _ => return Err(invalid("expected capture, inspect, or replay")),
    };
    let mut options = BTreeMap::new();
    while let Some(flag) = args.next() {
        if !allowed.contains(&flag.as_str()) {
            return Err(invalid(format!("unknown option {flag}")));
        }
        let value = if flag == "--lifecycle" {
            String::new()
        } else {
            args.next()
                .filter(|s| !s.starts_with("--"))
                .ok_or_else(|| invalid(format!("{flag} requires a value")))?
        };
        if options.insert(flag.clone(), value).is_some() {
            return Err(invalid(format!("duplicate option {flag}")));
        }
    }
    if action == "capture" {
        let (m, base) = dg_snapshot::load_project(Path::new(&target))?;
        let c = m
            .snapshot
            .as_ref()
            .ok_or_else(|| invalid("snapshot configuration is required"))?;
        let snapshot = dg_snapshot::capture(&m, &base, &HttpRpc::from_config(c)?)?;
        let directory = options
            .get("--output")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(".doppelganger/snapshots").join(&snapshot.snapshot_id));
        let path = dg_snapshot::save(&snapshot, &directory)?;
        // A capture with the same identity reuses the original immutable artifact.
        let snapshot = dg_snapshot::load(&path)?;
        return Ok(
            json!({"snapshot":snapshot,"path":path,"remote_transactions_sent":false,"rpc_write_calls":0}),
        );
    }
    let path = PathBuf::from(&target);
    let path = if path.exists() || path.components().count() > 1 {
        path
    } else {
        Path::new(
            options
                .get("--store")
                .map(String::as_str)
                .unwrap_or(".doppelganger/snapshots"),
        )
        .join(target)
    };
    let snapshot = dg_snapshot::load(&path)?;
    if action == "inspect" {
        return Ok(json!(snapshot));
    }
    let project = options
        .get("--project")
        .ok_or_else(|| invalid("--project is required"))?;
    let (m, base) = dg_snapshot::load_project(Path::new(project))?;
    let report = replay::replay(
        &snapshot,
        &m,
        &base,
        &CandidateOverride {
            binary: options.get("--candidate").map(PathBuf::from),
            idl: options.get("--candidate-idl").map(PathBuf::from),
        },
        options.contains_key("--lifecycle"),
    )?;
    if let Some(path) = options.get("--output") {
        let path = Path::new(path);
        if path.is_file() && dg_snapshot::load(path).is_ok() {
            return Err(SnapshotError {
                code: "SNAPSHOT_EXISTS".into(),
                context: "output".into(),
                message: "replay output cannot overwrite an immutable snapshot".into(),
            });
        }
        let write = || -> Result<(), Box<dyn Error>> {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, serde_json::to_vec_pretty(&report)?)?;
            Ok(())
        };
        write().map_err(|_| invalid("cannot write replay report"))?;
    }
    Ok(report)
}
