//! Trusted fixture qualification only. This host process path is not a production runner.
use dg_service::{
    intake::{extract, preflight, Limits},
    model::{JobState, PreparedUpload, Verdict},
    publish::ValidatedReport,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, ZipWriter};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn qualified_engine() -> PathBuf {
    let engine = PathBuf::from(
        std::env::var("DG_QUALIFIED_ENGINE")
            .expect("Set DG_QUALIFIED_ENGINE to the qualified 0.1.0 binary"),
    );
    let lock: Value =
        serde_json::from_slice(&fs::read(repo().join("docs/backend-engine-lock.json")).unwrap())
            .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(&engine).unwrap())),
        lock["sha256"].as_str().unwrap(),
        "Engine binary differs from qualification lock"
    );
    engine
}
fn archive(example: &Path, broken: bool) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for name in [
        "doppelganger.toml",
        "doppelganger-policy.toml",
        "v1.bin",
        "v2.bin",
        "v1.json",
        "v2.json",
    ] {
        let path = if broken && name == "v2.bin" {
            repo().join("target/deploy/escrow-broken/dg_escrow_v2.so")
        } else {
            example.join(name)
        };
        writer
            .start_file(format!("project/{name}"), SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&fs::read(path).unwrap()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
#[ignore = "Explicit trusted-fixture qualification; requires pinned engine and packaged examples"]
fn prepared_uploads_match_real_cli_results_and_broken_case_replays() {
    let engine = qualified_engine();
    let examples = PathBuf::from(
        std::env::var("DG_PREPARED_EXAMPLES")
            .expect("Set DG_PREPARED_EXAMPLES to packaged standalone examples"),
    );
    for (name, broken, verdict) in [
        ("vault", false, Verdict::Pass),
        ("escrow", false, Verdict::Pass),
        ("record", false, Verdict::Pass),
        ("escrow", true, Verdict::Fail),
    ] {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path().join("input");
        extract(
            Cursor::new(archive(&examples.join(name), broken)),
            &root,
            Limits::default(),
        )
        .unwrap();
        let metadata = PreparedUpload {
            schema_version: 1,
            mode: "prepared".into(),
            manifest_path: "project/doppelganger.toml".into(),
            policy_path: "project/doppelganger-policy.toml".into(),
            snapshot_path: None,
        };
        let paths = preflight(&root, &metadata).unwrap();
        let output = temporary.path().join("output");
        // Only committed developer-controlled fixtures are allowed in this test.
        let process = Command::new(&engine)
            .args(["test"])
            .arg(&paths.manifest)
            .arg("--policy")
            .arg(&paths.policy)
            .arg("--output")
            .arg(&output)
            .arg("--ci")
            .current_dir(temporary.path())
            .output()
            .unwrap();
        let bytes = fs::read(output.join("report.json")).unwrap();
        let report = ValidatedReport::parse(process.status.code(), &bytes).unwrap();
        assert_eq!(report.state(), JobState::Completed);
        assert_eq!(report.verdict(), verdict);
        let client = report.client_view(root.to_str().unwrap()).unwrap();
        assert!(!serde_json::to_string(&client)
            .unwrap()
            .contains(root.to_str().unwrap()));
        if broken {
            let case = temporary.path().join("case");
            let captured = Command::new(&engine)
                .args(["regression", "capture", "--from-report"])
                .arg(output.join("report.json"))
                .arg("--store")
                .arg(&case)
                .output()
                .unwrap();
            assert!(
                captured.status.success(),
                "{}",
                String::from_utf8_lossy(&captured.stdout)
            );
            let replay = Command::new(&engine)
                .args(["regression", "replay"])
                .arg(case)
                .arg("--output")
                .arg(temporary.path().join("replayed"))
                .output()
                .unwrap();
            assert!(
                replay.status.success(),
                "{}",
                String::from_utf8_lossy(&replay.stdout)
            );
        }
    }
}

#[test]
#[ignore = "Explicit trusted-fixture qualification; requires pinned engine"]
fn invalid_configuration_is_a_completed_error_with_real_cli_diagnostics() {
    let engine = qualified_engine();
    let temporary = TempDir::new().unwrap();
    let output = temporary.path().join("error");
    let process = Command::new(engine)
        .arg("test")
        .arg(temporary.path().join("missing.toml"))
        .arg("--output")
        .arg(&output)
        .arg("--ci")
        .output()
        .unwrap();
    let report = ValidatedReport::parse(
        process.status.code(),
        &fs::read(output.join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report.exit_code(), 2);
    assert_eq!(report.state(), JobState::Completed);
    assert_eq!(report.verdict(), Verdict::Error);
}
