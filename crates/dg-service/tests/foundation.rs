use dg_service::{
    intake::{extract, preflight, receive, Limits},
    model::*,
    publish::ValidatedReport,
    store::Store,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, ZipWriter};

fn metadata() -> PreparedUpload {
    PreparedUpload {
        schema_version: 1,
        mode: "prepared".into(),
        manifest_path: "project/doppelganger.toml".into(),
        policy_path: "project/doppelganger-policy.toml".into(),
        snapshot_path: None,
    }
}
fn zip_files(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn fixture_report(exit: i32) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version":1,"engine_version":"0.1.0","workflow_version":1,
        "manifest_schema_version":1,"legacy_unversioned_manifest":false,"run_id":format!("RUN-{}","a".repeat(64)),
        "result":if exit==0 {"PASS"} else {"FAIL"},"exit_code":exit,
        "remote_transactions_sent":false,"rpc_write_calls":0,"migration_executed_locally":true,
        "preservation_evaluated":true,"lifecycle_evaluated":true,"policy_evaluated":true,
        "analysis":{},"execution":{},"preservation":{},"relationships":{},"lifecycle":{},"artifacts":{"manifest":{"path":"/sandbox/project/doppelganger.toml"}},
        "policy":{"schema_version":1,"policy_evaluated":true,"passed":exit==0,"blocking":if exit==0 {0}else{1},"evaluation_errors":0}
    })).unwrap()
}
async fn store() -> (TempDir, Store) {
    let directory = TempDir::new().unwrap();
    let store = Store::open(&directory.path().join("jobs.sqlite"))
        .await
        .unwrap();
    (directory, store)
}
async fn submission(store: &Store, owner: &str, root: &Path) -> String {
    let archive = receive(
        root,
        Cursor::new(zip_files(&[("file", b"fixture")])),
        Limits::default(),
    )
    .unwrap();
    store
        .add_submission(owner, &metadata(), &archive, 0)
        .await
        .unwrap()
}
fn artifact(claim: &Claim, kind: &str) -> Artifact {
    Artifact {
        kind: kind.into(),
        storage_key: format!("jobs/{}/{kind}", claim.job_id),
        sha256: "a".repeat(64),
        bytes: 1,
    }
}

#[test]
fn upload_metadata_rejects_unsupported_modes_aliases_and_unknown_fields() {
    for path in [
        "",
        "/etc/passwd",
        "../data",
        "a/../data",
        "C:/data",
        "a\\data",
        "a//data",
        "a/./data",
        "a/",
        "a\0b",
    ] {
        let mut m = metadata();
        m.manifest_path = path.into();
        assert!(m.validate().is_err(), "{path}");
    }
    let mut m = metadata();
    m.mode = "source".into();
    assert_eq!(m.validate().unwrap_err().code, "UNSUPPORTED_INPUT");
    m = metadata();
    m.schema_version = 2;
    assert!(m.validate().is_err());
    let mut value = serde_json::to_value(metadata()).unwrap();
    value["command"] = json!("sh");
    assert!(serde_json::from_value::<PreparedUpload>(value).is_err());
}

#[test]
fn intake_hashes_publishes_and_removes_partial_uploads() {
    let root = TempDir::new().unwrap();
    let limits = Limits {
        upload_bytes: 3,
        ..Limits::default()
    };
    assert_eq!(
        receive(root.path(), Cursor::new(b"four"), limits)
            .unwrap_err()
            .code,
        "INPUT_TOO_LARGE"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert!(receive(root.path(), Cursor::new(b""), limits).is_err());
    let receipt = receive(root.path(), Cursor::new(b"abc"), limits).unwrap();
    assert_eq!(
        receipt.sha256(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(fs::read(receipt.path()).unwrap(), b"abc");
    assert_eq!(receipt.bytes(), 3);
}

#[test]
fn extraction_rejects_escape_duplicates_links_and_corruption() {
    let root = TempDir::new().unwrap();
    for name in [
        "../escaped",
        "/escaped",
        "a/../escaped",
        "C:/escaped",
        "a\\escaped",
    ] {
        let destination = root.path().join("output");
        assert!(extract(
            Cursor::new(zip_files(&[(name, b"x")])),
            &destination,
            Limits::default()
        )
        .is_err());
        assert!(!destination.exists());
    }
    let mut duplicate = zip_files(&[("file", b"one"), ("fill", b"two")]);
    // ZipWriter rejects duplicate names; forge both headers to exercise hostile input.
    for index in 0..duplicate.len() - 3 {
        if &duplicate[index..index + 4] == b"fill" {
            duplicate[index..index + 4].copy_from_slice(b"file");
        }
    }
    assert!(extract(
        Cursor::new(duplicate),
        &root.path().join("duplicate"),
        Limits::default()
    )
    .is_err());
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("linked", "/etc/passwd", SimpleFileOptions::default())
        .unwrap();
    assert!(extract(
        writer.finish().unwrap(),
        &root.path().join("linked"),
        Limits::default()
    )
    .is_err());
    let mut damaged = zip_files(&[("file", b"unique-data")]);
    let offset = damaged
        .windows(11)
        .position(|w| w == b"unique-data")
        .unwrap();
    damaged[offset] ^= 1;
    assert!(extract(
        Cursor::new(damaged),
        &root.path().join("corrupt"),
        Limits::default()
    )
    .is_err());
    assert!(!root.path().join("corrupt").exists());
}

#[test]
fn extraction_counts_actual_bytes_and_entries_and_refuses_overwrite() {
    let root = TempDir::new().unwrap();
    let archive = zip_files(&[("a", b"abc"), ("b", b"def")]);
    for limits in [
        Limits {
            entries: 1,
            ..Limits::default()
        },
        Limits {
            entry_bytes: 2,
            ..Limits::default()
        },
        Limits {
            extracted_bytes: 5,
            ..Limits::default()
        },
    ] {
        let p = root.path().join("output");
        assert!(extract(Cursor::new(&archive), &p, limits).is_err());
        assert!(!p.exists());
    }
    let p = root.path().join("valid");
    extract(Cursor::new(&archive), &p, Limits::default()).unwrap();
    assert_eq!(fs::read(p.join("b")).unwrap(), b"def");
    assert!(extract(Cursor::new(archive), &p, Limits::default()).is_err());
    assert_eq!(fs::read(p.join("a")).unwrap(), b"abc");
}

#[test]
fn preflight_blocks_external_binary_idl_snapshot_program_and_links() {
    let root = TempDir::new().unwrap();
    let base = root.path().join("project");
    fs::create_dir(&base).unwrap();
    // A self-contained minimal valid manifest avoids relying on generated artifacts.
    let source="schema_version=1\n[project]\nname='fixture'\nframework='anchor'\n[versions.v1]\nprogram_id='11111111111111111111111111111111'\nprogram_path='v1.bin'\nidl='v1.json'\n[versions.v2]\nprogram_id='11111111111111111111111111111111'\nprogram_path='v2.bin'\nidl='v2.json'\n[migration]\ninstruction='migrate'\naccount_mapping={}\n".to_string();
    for f in ["v1.bin", "v2.bin", "v1.json", "v2.json"] {
        fs::write(base.join(f), b"placeholder").unwrap();
    }
    fs::write(base.join("doppelganger-policy.toml"), "schema_version=1").unwrap();
    fs::write(base.join("doppelganger.toml"), &source).unwrap();
    assert!(preflight(root.path(), &metadata()).is_ok());
    let outside = TempDir::new().unwrap();
    let external = outside.path().join("external");
    fs::write(&external, b"private").unwrap();
    for (field, old) in [("program_path", "v1.bin"), ("idl", "v1.json")] {
        let changed = source
            .replace(
                &format!("{field} = \"{old}\""),
                &format!("{field} = \"{}\"", external.display()),
            )
            .replace(
                &format!("{field}='{old}'"),
                &format!("{field}='{}'", external.display()),
            );
        fs::write(base.join("doppelganger.toml"), changed).unwrap();
        assert!(preflight(root.path(), &metadata()).is_err());
    }
    fs::write(base.join("doppelganger.toml"),format!("{source}\n[snapshot]\nsource='devnet'\naccounts=[]\n[[snapshot.programs]]\nprogram_id='11111111111111111111111111111111'\nprogram_path='{}'\n",external.display())).unwrap();
    assert!(preflight(root.path(), &metadata()).is_err());
    fs::write(base.join("doppelganger.toml"), source).unwrap();
    fs::remove_file(base.join("v1.bin")).unwrap();
    std::os::unix::fs::symlink(external, base.join("v1.bin")).unwrap();
    assert!(preflight(root.path(), &metadata()).is_err());
}

#[test]
fn report_protocol_rejects_false_pass_exit_mismatch_and_future_schema() {
    let bytes = fixture_report(0);
    assert_eq!(
        ValidatedReport::parse(Some(0), &bytes).unwrap().verdict(),
        Verdict::Pass
    );
    assert!(ValidatedReport::parse(Some(1), &bytes).is_err());
    assert!(ValidatedReport::parse(None, &bytes).is_err());
    assert!(ValidatedReport::parse(
        Some(0),
        br#"{"schema_version":1,"result":"PASS","exit_code":0}"#
    )
    .is_err());
    for (field, value) in [
        ("schema_version", json!(2)),
        ("engine_version", json!("0.2.0")),
        ("policy_evaluated", json!(false)),
        ("remote_transactions_sent", json!(true)),
        ("execution", Value::Null),
    ] {
        let mut report: Value = serde_json::from_slice(&bytes).unwrap();
        report[field] = value;
        assert!(
            ValidatedReport::parse(Some(0), &serde_json::to_vec(&report).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut report: Value = serde_json::from_slice(&bytes).unwrap();
    report["policy"]["blocking"] = json!(1);
    assert!(ValidatedReport::parse(Some(0), &serde_json::to_vec(&report).unwrap()).is_err());
}

#[test]
fn report_fail_is_completed_and_error_only_diagnostics_are_supported() {
    let report = ValidatedReport::parse(Some(1), &fixture_report(1)).unwrap();
    assert_eq!(report.state(), JobState::Completed);
    assert_eq!(report.verdict(), Verdict::Fail);
    for exit in 2..=4 {
        let bytes=serde_json::to_vec(&json!({"schema_version":1,"result":"ERROR","exit_code":exit,"diagnostics":[{"exit_code":exit,"message":"Input error"}]})).unwrap();
        let parsed = ValidatedReport::parse(Some(exit), &bytes).unwrap();
        assert_eq!(
            parsed.state(),
            if exit == 4 {
                JobState::Failed
            } else {
                JobState::Completed
            }
        );
    }
}

#[test]
fn client_view_redacts_paths_without_mutating_original_evidence() {
    let report = ValidatedReport::parse(Some(0), &fixture_report(0)).unwrap();
    let view = report.client_view("/sandbox").unwrap();
    assert_eq!(
        view["report"]["artifacts"]["manifest"]["path"],
        "project/project/doppelganger.toml"
    );
    assert_eq!(
        report.original()["artifacts"]["manifest"]["path"],
        "/sandbox/project/doppelganger.toml"
    );
    assert_eq!(view["derived_view"], true);
    assert_eq!(view["sha256"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn owner_boundaries_idempotency_and_reopening_preserve_jobs() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    assert!(db.submission("bob", &sub).await.unwrap().is_none());
    assert_eq!(
        db.enqueue("bob", &sub, None, 0).await.unwrap_err().code,
        "NOT_FOUND"
    );
    let job = db.enqueue("alice", &sub, Some("retry"), 0).await.unwrap();
    assert_eq!(
        db.enqueue("alice", &sub, Some("retry"), 1)
            .await
            .unwrap()
            .job_id,
        job.job_id
    );
    let other = submission(&db, "alice", root.path()).await;
    assert_eq!(
        db.enqueue("alice", &other, Some("retry"), 2)
            .await
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert!(db.job("bob", &job.job_id).await.unwrap().is_none());
    assert_eq!(
        db.cancel("bob", &job.job_id, 3).await.unwrap_err().code,
        "NOT_FOUND"
    );
    assert!(db.events("bob", &job.job_id, 0).await.is_err());
    assert!(db.artifacts("bob", &job.job_id).await.is_err());
    drop(db);
    let db = Store::open(&root.path().join("jobs.sqlite")).await.unwrap();
    assert_eq!(
        db.job("alice", &job.job_id).await.unwrap().unwrap().state,
        JobState::Queued
    );
    assert_eq!(db.events("alice", &job.job_id, 0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_enqueues_and_claims_enforce_quotas_and_single_execution() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    let mut tasks = vec![];
    for _ in 0..24 {
        let db = db.clone();
        let sub = sub.clone();
        tasks.push(tokio::spawn(async move {
            db.enqueue("alice", &sub, None, 0).await
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => accepted += 1,
            Err(e) => assert_eq!(e.code, "QUEUE_FULL"),
        }
    }
    assert_eq!(accepted, 4);
    let mut tasks = vec![];
    for _ in 0..12 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move { db.claim(1).await.unwrap() }));
    }
    let mut claimed = 0;
    for task in tasks {
        if task.await.unwrap().is_some() {
            claimed += 1;
        }
    }
    assert_eq!(claimed, 1);
}

#[tokio::test]
async fn global_queue_quota_is_atomic_across_owners() {
    let (root, db) = store().await;
    let mut tasks = vec![];
    for index in 0..24 {
        let owner = format!("owner{index}");
        let sub = submission(&db, &owner, root.path()).await;
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db.enqueue(&owner, &sub, None, 0).await
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => accepted += 1,
            Err(e) => assert_eq!(e.code, "QUEUE_FULL"),
        }
    }
    assert_eq!(accepted, 16);
}

#[tokio::test]
async fn cancelling_fences_publication_and_only_stop_acknowledges_terminal_state() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    let job = db.enqueue("alice", &sub, None, 0).await.unwrap();
    let claim = db.claim(1).await.unwrap().unwrap();
    assert!(db
        .advance(&claim, JobState::Validating, JobState::Running, 2)
        .await
        .unwrap());
    assert!(db
        .advance(&claim, JobState::Running, JobState::Publishing, 3)
        .await
        .unwrap());
    assert_eq!(
        db.cancel("alice", &job.job_id, 4).await.unwrap().state,
        JobState::Cancelling
    );
    let report = ValidatedReport::parse(Some(0), &fixture_report(0)).unwrap();
    assert!(!db
        .complete(&claim, &report, &[artifact(&claim, "report_json")], 5)
        .await
        .unwrap());
    assert!(db.artifacts("alice", &job.job_id).await.unwrap().is_empty());
    assert!(db.acknowledge_cancel(&claim, 6).await.unwrap());
    assert_eq!(
        db.cancel("alice", &job.job_id, 7).await.unwrap().state,
        JobState::Cancelled
    );
    assert!(!db.heartbeat(&claim, 8).await.unwrap());
    assert!(!db.acknowledge_cancel(&claim, 9).await.unwrap());
}

#[tokio::test]
async fn leases_expire_without_reviving_or_starting_duplicate_workers() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    let job = db.enqueue("alice", &sub, None, 0).await.unwrap();
    let claim = db.claim(1).await.unwrap().unwrap();
    let mut forged = claim.clone();
    forged.lease_token = "wrong".into();
    assert!(!db.heartbeat(&forged, 2).await.unwrap());
    assert!(db.heartbeat(&claim, 2).await.unwrap());
    assert!(!db.heartbeat(&claim, 30_002).await.unwrap());
    assert!(db.claim(30_003).await.unwrap().is_none());
    let lost = db.expired_claims(30_003).await.unwrap();
    assert_eq!(lost.len(), 1);
    assert_eq!(lost[0].lease_token, claim.lease_token);
    assert!(!db.reconcile_lost(&claim, 20_000).await.unwrap());
    assert!(db.reconcile_lost(&claim, 30_003).await.unwrap());
    let result = db.job("alice", &job.job_id).await.unwrap().unwrap();
    assert_eq!(result.state, JobState::Failed);
    assert_eq!(result.error_code.as_deref(), Some("WORKER_LOST"));
    assert!(!db
        .advance(&claim, JobState::Validating, JobState::Running, 30_004)
        .await
        .unwrap());
}

#[tokio::test]
async fn worker_failures_are_terminal_and_clock_overflow_is_rejected() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    let job = db.enqueue("alice", &sub, None, 0).await.unwrap();
    assert_eq!(
        db.claim(i64::MAX).await.err().unwrap().code,
        "INVALID_CLOCK"
    );
    let claim = db.claim(1).await.unwrap().unwrap();
    assert!(db
        .fail_attempt(&claim, FailureReason::Timeout, 2)
        .await
        .unwrap());
    let result = db.job("alice", &job.job_id).await.unwrap().unwrap();
    assert_eq!(result.state, JobState::Failed);
    assert_eq!(result.verdict, Some(Verdict::Error));
    assert_eq!(result.error_code.as_deref(), Some("JOB_TIMEOUT"));
    assert!(db.expired_claims(40_000).await.unwrap().is_empty());
    assert!(!db
        .fail_attempt(&claim, FailureReason::EngineInternal, 3)
        .await
        .unwrap());
}

#[test]
fn declared_zip_count_and_zip64_inputs_fail_before_extraction() {
    let root = TempDir::new().unwrap();
    let archive = zip_files(&[("a", b"x")]);
    for count in [10_001u16, u16::MAX] {
        let mut forged = archive.clone();
        let footer = forged.len() - 22;
        forged[footer + 8..footer + 10].copy_from_slice(&count.to_le_bytes());
        forged[footer + 10..footer + 12].copy_from_slice(&count.to_le_bytes());
        let destination = root.path().join("output");
        assert!(extract(Cursor::new(forged), &destination, Limits::default()).is_err());
        assert!(!destination.exists());
    }
}

#[tokio::test]
async fn publication_is_atomic_fail_is_completed_and_terminal_results_are_immutable() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    let job = db.enqueue("alice", &sub, None, 0).await.unwrap();
    let claim = db.claim(1).await.unwrap().unwrap();
    assert!(db
        .advance(&claim, JobState::Validating, JobState::Publishing, 2)
        .await
        .unwrap());
    let report = ValidatedReport::parse(Some(1), &fixture_report(1)).unwrap();
    let json = artifact(&claim, "report_json");
    let bad = artifact(&claim, "unknown");
    assert!(db
        .complete(&claim, &report, &[json.clone(), bad], 3)
        .await
        .is_err());
    assert_eq!(
        db.job("alice", &job.job_id).await.unwrap().unwrap().state,
        JobState::Publishing
    );
    assert!(db.artifacts("alice", &job.job_id).await.unwrap().is_empty());
    let mut wrong = json.clone();
    wrong.storage_key = "jobs/other/report_json".into();
    assert!(db.complete(&claim, &report, &[wrong], 3).await.is_err());
    assert!(db
        .complete(&claim, &report, std::slice::from_ref(&json), 4)
        .await
        .unwrap());
    let done = db.job("alice", &job.job_id).await.unwrap().unwrap();
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.verdict, Some(Verdict::Fail));
    assert_eq!(db.artifacts("alice", &job.job_id).await.unwrap().len(), 1);
    assert!(!db.complete(&claim, &report, &[json], 5).await.unwrap());
    assert_eq!(
        db.cancel("alice", &job.job_id, 6).await.unwrap_err().code,
        "JOB_TERMINAL"
    );
}

#[tokio::test]
async fn queued_cancel_is_immediate_and_emits_one_terminal_event() {
    let (root, db) = store().await;
    let sub = submission(&db, "alice", root.path()).await;
    let job = db.enqueue("alice", &sub, None, 0).await.unwrap();
    assert_eq!(
        db.cancel("alice", &job.job_id, 1).await.unwrap().state,
        JobState::Cancelled
    );
    assert_eq!(
        db.cancel("alice", &job.job_id, 2).await.unwrap().state,
        JobState::Cancelled
    );
    assert!(db.claim(3).await.unwrap().is_none());
    let events = db.events("alice", &job.job_id, 0).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        db.events("alice", &job.job_id, events[1].cursor)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn unknown_database_versions_and_symlinks_are_rejected() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("future.sqlite");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version=99")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(
        Store::open(&path).await.err().unwrap().code,
        "UNSUPPORTED_STORAGE"
    );
    std::os::unix::fs::symlink(&path, root.path().join("linked.sqlite")).unwrap();
    assert_eq!(
        Store::open(&root.path().join("linked.sqlite"))
            .await
            .err()
            .unwrap()
            .code,
        "INVALID_STORAGE"
    );
}

#[test]
fn api_contract_references_resolve_and_routes_declare_security() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/backend-api.yaml");
    let spec: Value = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
    assert_eq!(spec["openapi"], "3.1.0");
    assert_eq!(spec["security"][0]["bearerAuth"], json!([]));
    fn walk(v: &Value, spec: &Value) {
        match v {
            Value::Object(o) => {
                if let Some(reference) = o.get("$ref") {
                    assert!(spec
                        .pointer(reference.as_str().unwrap().strip_prefix('#').unwrap())
                        .is_some());
                }
                for v in o.values() {
                    walk(v, spec)
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, spec)
                }
            }
            _ => {}
        }
    }
    walk(&spec, &spec);
    for route in [
        "/v1/submissions",
        "/v1/jobs",
        "/v1/jobs/{id}/cancel",
        "/v1/jobs/{id}/report",
        "/v1/jobs/{id}/artifacts/{kind}",
    ] {
        assert!(spec["paths"][route].is_object());
    }
}
