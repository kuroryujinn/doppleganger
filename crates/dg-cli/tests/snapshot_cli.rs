use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .env("DG_DEVNET_RPC_URL", "http://127.0.0.1:1")
        .args(args)
        .output()
        .unwrap()
}
fn value(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "dg-r11-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const SNAPSHOT: &str = "fixtures/devnet-snapshots/escrow-mock";
const PROJECT: &str = "examples/escrow-snapshot-offline";

#[test]
fn inspect_and_offline_replay_show_hashes_source_and_scope() {
    let inspected = value(run(&["snapshot", "inspect", SNAPSHOT]));
    assert_eq!(inspected["schema_version"], 1);
    assert_eq!(inspected["provenance"]["slot"], 42);
    assert!(inspected["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["record_sha256"].as_str().unwrap().len() == 64));
    let temp = Temp::new();
    let path = temp.0.join("good.json");
    let report = value(run(&[
        "snapshot",
        "replay",
        SNAPSHOT,
        "--project",
        PROJECT,
        "--lifecycle",
        "--output",
        path.to_str().unwrap(),
    ]));
    assert_eq!(report["preservation"]["passed"], 11);
    assert_eq!(report["lifecycle"]["passed"], 6);
    assert_eq!(
        report["remote_state_imported"], false,
        "mock fixture must not claim real Devnet provenance"
    );
    assert_eq!(report["remote_transactions_sent"], false);
    assert_eq!(report["rpc_write_calls"], 0);
    assert_eq!(report["snapshot"], inspected);
    assert_eq!(
        report,
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()
    );
}

#[test]
fn candidate_comparison_and_regression_cli_reproduce_without_rpc() {
    let temp = Temp::new();
    let report_path = temp.0.join("broken.json");
    let store = temp.0.join("regressions");
    let report = value(run(&[
        "snapshot",
        "replay",
        SNAPSHOT,
        "--project",
        PROJECT,
        "--candidate",
        "target/deploy/escrow-broken/dg_escrow_v2.so",
        "--output",
        report_path.to_str().unwrap(),
    ]));
    assert_eq!(report["execution"]["transaction"]["success"], true);
    assert_eq!(report["preservation"]["failed"], 1);
    assert_eq!(
        report["preservation"]["findings"][0]["id"],
        "PRESERVE-amount-retained"
    );
    let created = value(run(&[
        "regression",
        "create",
        "--from-report",
        report_path.to_str().unwrap(),
        "--manifest",
        PROJECT,
        "--finding",
        "PRESERVE-amount-retained",
        "--store",
        store.to_str().unwrap(),
    ]));
    let directory = created["directory"].as_str().unwrap();
    assert_eq!(created["case"]["reconstruction"], "snapshot_replay");
    assert_eq!(
        value(run(&["regression", "reproduce", directory]))["result"],
        "REPRODUCED"
    );
    assert_eq!(
        value(run(&[
            "regression",
            "reproduce",
            directory,
            "--candidate",
            "target/deploy/escrow-v2/dg_escrow_v2.so"
        ]))["result"],
        "NOT_REPRODUCED"
    );
}

#[test]
fn lifecycle_regression_embeds_snapshot_and_reproduces_offline() {
    let temp = Temp::new();
    let mut config: toml::Value = toml::from_str(
        &fs::read_to_string(root().join(PROJECT).join("doppelganger.toml")).unwrap(),
    )
    .unwrap();
    for v in ["v1", "v2"] {
        for field in ["program_path", "idl"] {
            config["versions"][v][field] = toml::Value::String(
                fs::canonicalize(
                    root()
                        .join(PROJECT)
                        .join(config["versions"][v][field].as_str().unwrap()),
                )
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            );
        }
    }
    config["scenario"][0]["expected"] = toml::Value::String("error".into());
    let manifest = temp.0.join("doppelganger.toml");
    fs::write(&manifest, toml::to_string_pretty(&config).unwrap()).unwrap();
    let report = temp.0.join("lifecycle.json");
    let store = temp.0.join("regressions");
    let r = value(run(&[
        "snapshot",
        "replay",
        SNAPSHOT,
        "--project",
        manifest.to_str().unwrap(),
        "--lifecycle",
        "--output",
        report.to_str().unwrap(),
    ]));
    assert_eq!(r["lifecycle"]["failed"], 1);
    let created = value(run(&[
        "regression",
        "create",
        "--from-report",
        report.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--scenario",
        "standard",
        "--store",
        store.to_str().unwrap(),
    ]));
    fs::remove_file(report).unwrap();
    let directory = created["directory"].as_str().unwrap();
    assert_eq!(
        value(run(&["regression", "reproduce", directory]))["result"],
        "REPRODUCED"
    );
    assert!(Path::new(directory).join("snapshots/source.json").is_file());
}

#[test]
fn real_devnet_snapshot_regression_reproduces_offline() {
    let temp = Temp::new();
    let report_path = temp.0.join("broken.json");
    let project = "examples/vault-runtime-devnet";
    let report = value(run(&[
        "snapshot",
        "replay",
        "fixtures/devnet-snapshots/vault-devnet",
        "--project",
        project,
        "--candidate",
        "fixtures/devnet-snapshots/vault-devnet/artifacts/v2-broken.bin",
        "--output",
        report_path.to_str().unwrap(),
    ]));
    assert_eq!(report["snapshot"]["provenance"]["acquisition"], "rpc");
    let store = temp.0.join("regressions");
    let created = value(run(&[
        "regression",
        "create",
        "--from-report",
        report_path.to_str().unwrap(),
        "--manifest",
        project,
        "--finding",
        "PRESERVE-balance-field",
        "--store",
        store.to_str().unwrap(),
    ]));
    let directory = created["directory"].as_str().unwrap();
    fs::remove_file(report_path).unwrap();
    let broken = value(run(&["regression", "reproduce", directory]));
    assert_eq!(broken["result"], "REPRODUCED");
    assert_eq!(broken["initial_state_verified"], true);
    let good = value(run(&[
        "regression",
        "reproduce",
        directory,
        "--candidate",
        "fixtures/devnet-snapshots/vault-devnet/artifacts/v2-good.bin",
    ]));
    assert_eq!(good["result"], "NOT_REPRODUCED");
    assert_eq!(good["initial_state_verified"], true);
}

#[test]
fn tampered_snapshot_and_cli_errors_stop_before_execution() {
    let temp = Temp::new();
    let mut s = value(run(&["snapshot", "inspect", SNAPSHOT]));
    s["provenance"]["slot"] = json!(43);
    let path = temp.0.join("snapshot.json");
    fs::write(&path, serde_json::to_vec(&s).unwrap()).unwrap();
    let output = run(&[
        "snapshot",
        "replay",
        path.to_str().unwrap(),
        "--project",
        PROJECT,
    ]);
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["diagnostics"][0]["code"], "SNAPSHOT_HASH_MISMATCH");
    assert!(error.get("execution").is_none());
    let output = run(&["snapshot", "capture", PROJECT, "--output"]);
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["diagnostics"][0]["code"], "CONFIG");
    let original = value(run(&["snapshot", "inspect", SNAPSHOT]));
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let output = run(&[
        "snapshot",
        "replay",
        path.to_str().unwrap(),
        "--project",
        PROJECT,
        "--output",
        path.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["diagnostics"][0]["code"], "SNAPSHOT_EXISTS");
    assert_eq!(
        value(run(&["snapshot", "inspect", path.to_str().unwrap()])),
        original
    );
}
