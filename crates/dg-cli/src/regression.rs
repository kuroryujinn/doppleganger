use dg_regression::{CandidateOverride, RegressionError, Source};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    error::Error,
    path::{Path, PathBuf},
};

fn invalid(message: impl ToString) -> RegressionError {
    RegressionError {
        code: "INVALID_REGRESSION_SCHEMA".into(),
        context: "arguments".into(),
        message: message.to_string(),
    }
}

pub fn command(args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let mut args = args.peekable();
    if args
        .peek()
        .is_some_and(|a| matches!(a.as_str(), "capture" | "replay"))
    {
        let action = args.next().unwrap();
        return crate::bundle::command(&action, args);
    }
    match run(args) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"schema_version":1,"result":"ERROR","diagnostics":[error]})
                )?
            );
            Err(error.into())
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> dg_regression::Result<Value> {
    let action = args
        .next()
        .ok_or_else(|| invalid("expected create, list, inspect, or reproduce"))?;
    if action == "--help" {
        return Ok(json!({"usage":[
            "regression create --from-report FILE --manifest FILE_OR_DIR (--finding ID | --scenario ID) [--store DIR]",
            "regression list [--store DIR]",
            "regression inspect ID_OR_DIR [--store DIR]",
            "regression reproduce ID_OR_DIR [--candidate BINARY] [--candidate-idl IDL] [--store DIR]"
        ]}));
    }
    let target = if matches!(action.as_str(), "inspect" | "reproduce") {
        Some(
            args.next()
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| invalid("missing regression ID or directory"))?,
        )
    } else {
        None
    };
    let allowed: &[&str] = match action.as_str() {
        "create" => &[
            "--from-report",
            "--manifest",
            "--finding",
            "--scenario",
            "--store",
        ],
        "list" | "inspect" => &["--store"],
        "reproduce" => &["--candidate", "--candidate-idl", "--store"],
        _ => return Err(invalid("expected create, list, inspect, or reproduce")),
    };
    let mut options = BTreeMap::new();
    while let Some(flag) = args.next() {
        if !allowed.contains(&flag.as_str()) {
            return Err(invalid(format!("unknown option {flag}")));
        }
        let value = args
            .next()
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| invalid(format!("{flag} requires a value")))?;
        if options.insert(flag.clone(), value).is_some() {
            return Err(invalid(format!("duplicate option {flag}")));
        }
    }
    let store = PathBuf::from(
        options
            .get("--store")
            .map(String::as_str)
            .unwrap_or(".doppelganger/regressions"),
    );
    let required = |key: &str| {
        options
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| invalid(format!("missing {key}")))
    };
    let directory = target.map(|t| {
        let path = PathBuf::from(&t);
        if path.is_dir() || path.components().count() > 1 {
            path
        } else {
            store.join(t)
        }
    });
    match action.as_str() {
        "create" => {
            let source = match (options.get("--finding"), options.get("--scenario")) {
                (Some(id), None) => Source::Preservation(id.clone()),
                (None, Some(id)) => Source::Lifecycle(id.clone()),
                _ => return Err(invalid("supply exactly one of --finding or --scenario")),
            };
            Ok(json!(dg_regression::create(
                Path::new(required("--from-report")?),
                Path::new(required("--manifest")?),
                source,
                &store
            )?))
        }
        "list" => Ok(json!(dg_regression::list(&store)?)),
        "inspect" => {
            let directory = directory
                .as_deref()
                .ok_or_else(|| invalid("missing directory"))?;
            let loaded = dg_regression::load(directory)?;
            let text = std::fs::read_to_string(directory.join("manifest.toml")).map_err(invalid)?;
            let config: toml::Value = toml::from_str(&text).map_err(invalid)?;
            Ok(
                json!({"case":loaded.case,"artifacts":loaded.artifacts,"expected":loaded.expected,"before":loaded.before,
                "config":config}),
            )
        }
        "reproduce" => Ok(json!(dg_regression::reproduce(
            directory
                .as_deref()
                .ok_or_else(|| invalid("missing directory"))?,
            &CandidateOverride {
                binary: options.get("--candidate").map(PathBuf::from),
                idl: options.get("--candidate-idl").map(PathBuf::from),
            }
        )?)),
        _ => Err(invalid("unsupported command")),
    }
}
