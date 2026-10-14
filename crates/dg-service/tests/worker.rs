use dg_service::{
    intake::{receive, Limits},
    model::*,
    protocol::*,
    publish::ValidatedReport,
    runner::{now, Worker, WorkerConfig},
    sandbox,
    store::Store,
    ServiceError,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Cursor,
    os::unix::{
        fs::{symlink, PermissionsExt},
        net::UnixListener,
    },
    path::PathBuf,
};
use tempfile::TempDir;
use tokio::{
    sync::mpsc,
    time::{timeout, Duration},
};

fn report(exit: i32) -> Vec<u8> {
    serde_json::to_vec(&json!({"schema_version":1,"engine_version":"0.1.0","workflow_version":1,
        "manifest_schema_version":1,"legacy_unversioned_manifest":false,"run_id":format!("RUN-{}","a".repeat(64)),
        "result":if exit==0 {"PASS"} else {"FAIL"},"exit_code":exit,
        "remote_transactions_sent":false,"rpc_write_calls":0,"migration_executed_locally":true,
        "preservation_evaluated":true,"lifecycle_evaluated":true,"policy_evaluated":true,
        "analysis":{},"execution":{},"preservation":{},"relationships":{},"lifecycle":{},
        "artifacts":{"manifest":{"path":"/work/input/project/doppelganger.toml"}},
        "policy":{"schema_version":1,"policy_evaluated":true,"passed":exit==0,"blocking":if exit==0 {0}else{1},"evaluation_errors":0}
    })).unwrap()
}
fn stream(exit: i32, case: bool) -> Vec<u8> {
    let mut output = MAGIC.to_vec();
    frame(&mut output, RUNNING, &[]).unwrap();
    frame(&mut output, PUBLISHING, &[]).unwrap();
    frame(&mut output, ENGINE_EXIT, &[exit as u8]).unwrap();
    frame(&mut output, ORIGINAL, &report(exit)).unwrap();
    frame(&mut output, HTML, b"<html>fixture</html>").unwrap();
    frame(&mut output, SUMMARY, b"fixture").unwrap();
    if case {
        frame(&mut output, CASE, b"opaque fixture ZIP").unwrap();
    }
    frame(&mut output, DONE, &[]).unwrap();
    output
}
fn input_error() -> Vec<u8> {
    let mut output = MAGIC.to_vec();
    frame(&mut output, PUBLISHING, &[]).unwrap();
    frame(&mut output, INPUT_ERROR, &[]).unwrap();
    let report = ValidatedReport::backend_error(&ServiceError::new(
        "MISSING_ARTIFACT",
        "Supply V1/V2 binaries",
    ));
    frame(
        &mut output,
        ORIGINAL,
        &serde_json::to_vec(report.original()).unwrap(),
    )
    .unwrap();
    frame(&mut output, DONE, &[]).unwrap();
    output
}
struct Runtime {
    _temp: TempDir,
    _socket: UnixListener,
    root: PathBuf,
    cfg: WorkerConfig,
}
impl Runtime {
    fn new(seconds: u64) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_owned();
        let socket = UnixListener::bind(root.join("docker.sock")).unwrap();
        fs::write(root.join("docker.py"), include_str!("fixtures/docker.py")).unwrap();
        fs::set_permissions(root.join("docker.py"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("identity.json"), sandbox::ENGINE_LOCK).unwrap();
        fs::write(root.join("mode"), "normal").unwrap();
        fs::write(root.join("stream.bin"), stream(0, false)).unwrap();
        let cfg = WorkerConfig {
            storage_root: root.join("storage"),
            docker: root.join("docker.py"),
            socket: root.join("docker.sock"),
            image: format!("sha256:{}", "b".repeat(64)),
            timeout_seconds: seconds,
        };
        Self {
            _temp: temp,
            _socket: socket,
            root,
            cfg,
        }
    }
    fn set(&self, mode: &str, data: &[u8]) {
        fs::write(self.root.join("mode"), mode).unwrap();
        fs::write(self.root.join("stream.bin"), data).unwrap();
    }
    fn inventory(&self) -> Value {
        serde_json::from_slice(&fs::read(self.root.join("containers.json")).unwrap()).unwrap()
    }
    async fn job(&self, store: &Store) -> Job {
        // Receipt bytes remain opaque on the host. Runtime double supplies trusted output.
        let archive = receive(
            &self.cfg.storage_root.join("submissions"),
            Cursor::new(b"opaque upload"),
            Limits::default(),
        )
        .unwrap();
        let metadata = PreparedUpload {
            schema_version: 1,
            mode: "prepared".into(),
            manifest_path: "project/manifest.toml".into(),
            policy_path: "project/policy.toml".into(),
            snapshot_path: None,
        };
        let sub = store
            .add_submission("owner", &metadata, &archive, now())
            .await
            .unwrap();
        store.enqueue("owner", &sub, None, now()).await.unwrap()
    }
}

#[tokio::test]
async fn frames_reject_duplicates_trailing_data_missing_outputs_and_forged_invocation() {
    let valid = stream(0, false);
    let mut trailing = valid.clone();
    trailing.push(1);
    let mut duplicate = valid[..valid.len() - 9].to_vec();
    frame(&mut duplicate, ORIGINAL, b"{}").unwrap();
    frame(&mut duplicate, DONE, &[]).unwrap();
    let mut missing = MAGIC.to_vec();
    frame(&mut missing, RUNNING, &[]).unwrap();
    frame(&mut missing, PUBLISHING, &[]).unwrap();
    frame(&mut missing, ENGINE_EXIT, &[0]).unwrap();
    frame(&mut missing, ORIGINAL, &report(0)).unwrap();
    frame(&mut missing, DONE, &[]).unwrap();
    let mut forged = MAGIC.to_vec();
    frame(&mut forged, PUBLISHING, &[]).unwrap();
    frame(&mut forged, ENGINE_EXIT, &[0]).unwrap();
    let mut oversized = valid[..36].to_vec();
    oversized.push(ORIGINAL);
    oversized.extend_from_slice(&(OUTPUT_LIMIT + 1).to_be_bytes());
    for bytes in [
        b"invalid!".to_vec(),
        trailing,
        duplicate,
        missing,
        forged,
        oversized,
        valid[..valid.len() - 1].to_vec(),
    ] {
        let directory = TempDir::new().unwrap();
        let (sender, _receiver) = mpsc::channel(4);
        assert!(collect(bytes.as_slice(), directory.path(), sender)
            .await
            .is_err());
    }
    let directory = TempDir::new().unwrap();
    let (sender, mut receiver) = mpsc::channel(4);
    let result = collect(valid.as_slice(), directory.path(), sender)
        .await
        .unwrap();
    assert_eq!(result.engine_exit, Some(0));
    assert_eq!(receiver.recv().await, Some(JobState::Running));
    assert_eq!(receiver.recv().await, Some(JobState::Publishing));
    assert_eq!(receiver.recv().await, None);
}

#[tokio::test]
async fn passes_failures_and_backend_errors_publish_durable_allowlisted_evidence() {
    for (data, verdict, exit, artifacts) in [
        (stream(0, false), Verdict::Pass, Some(0), 3),
        (stream(1, true), Verdict::Fail, Some(1), 4),
        (input_error(), Verdict::Error, None, 1),
    ] {
        let runtime = Runtime::new(5);
        runtime.set("normal", &data);
        let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
        let job = runtime.job(&worker.store).await;
        assert!(timeout(Duration::from_secs(10), worker.run_once())
            .await
            .unwrap()
            .unwrap());
        let result = worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.state, JobState::Completed);
        assert_eq!(result.verdict, Some(verdict));
        assert_eq!(result.engine_exit_code, exit);
        if exit.is_none() {
            assert_eq!(result.engine_version, None);
        }
        let inventory = worker.store.artifacts("owner", &job.job_id).await.unwrap();
        assert_eq!(inventory.len(), artifacts);
        for artifact in &inventory {
            let bytes = fs::read(runtime.cfg.storage_root.join(&artifact.storage_key)).unwrap();
            assert_eq!(artifact.bytes, bytes.len() as i64);
            assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(&bytes)));
            if artifact.kind == "report_json" {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["derived_view"], true);
                assert!(!String::from_utf8(bytes).unwrap().contains("/work/input"));
            }
        }
        let events = worker.store.events("owner", &job.job_id, 0).await.unwrap();
        assert_eq!(events.last().unwrap().state, "completed");
        assert_eq!(runtime.inventory(), json!({}));
        drop(worker);
        let reopened = Store::open(&runtime.cfg.storage_root.join("jobs.sqlite"))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .artifacts("owner", &job.job_id)
                .await
                .unwrap()
                .len(),
            artifacts
        );
    }
}

#[tokio::test]
async fn malformed_exit_mismatch_wrapper_failure_and_timeout_never_publish_pass() {
    let mut mismatch = stream(0, false);
    let needle = report(0);
    let index = mismatch
        .windows(needle.len())
        .position(|v| v == needle)
        .unwrap();
    let mut value: Value = serde_json::from_slice(&needle).unwrap();
    value["exit_code"] = json!(1);
    let raw = serde_json::to_vec(&value).unwrap();
    assert_eq!(needle.len(), raw.len());
    mismatch[index..index + raw.len()].copy_from_slice(&raw);
    for (mode, data, code) in [
        ("normal", b"forged".to_vec(), "ENGINE_PROTOCOL_ERROR"),
        ("normal", mismatch, "ENGINE_PROTOCOL_ERROR"),
        ("wrapper_error", stream(0, false), "SANDBOX_ERROR"),
        ("hang", stream(0, false), "JOB_TIMEOUT"),
    ] {
        let runtime = Runtime::new(if mode == "hang" { 1 } else { 5 });
        runtime.set(mode, &data);
        let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
        let job = runtime.job(&worker.store).await;
        assert!(timeout(Duration::from_secs(10), worker.run_once())
            .await
            .unwrap()
            .unwrap());
        let job = worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.verdict, Some(Verdict::Error));
        assert_eq!(job.error_code.as_deref(), Some(code));
        assert!(worker
            .store
            .artifacts("owner", &job.job_id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(runtime.inventory(), json!({}));
    }
}

#[tokio::test]
async fn cancellation_stops_before_acknowledging_and_logs_are_drained_after_the_cap() {
    let runtime = Runtime::new(5);
    runtime.set("hang", &stream(0, false));
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    let job = runtime.job(&worker.store).await;
    let canceller = async {
        loop {
            let state = worker
                .store
                .job("owner", &job.job_id)
                .await
                .unwrap()
                .unwrap()
                .state;
            if state == JobState::Running {
                worker
                    .store
                    .cancel("owner", &job.job_id, now())
                    .await
                    .unwrap();
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(Duration::from_secs(10), async {
        let (ran, ()) = tokio::join!(worker.run_once(), canceller);
        assert!(ran.unwrap());
    })
    .await
    .unwrap();
    let job = worker
        .store
        .job("owner", &job.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Cancelled);
    assert_eq!(job.verdict, None);
    assert_eq!(runtime.inventory(), json!({}));
    runtime.set("logs", &stream(0, false));
    let job = runtime.job(&worker.store).await;
    worker.run_once().await.unwrap();
    let events = worker.store.events("owner", &job.job_id, 0).await.unwrap();
    assert!(events
        .iter()
        .any(|event| event.message.as_deref() == Some("LOG_TRUNCATED")));
    let artifacts = worker.store.artifacts("owner", &job.job_id).await.unwrap();
    let parent = runtime
        .cfg
        .storage_root
        .join(&artifacts[0].storage_key)
        .parent()
        .unwrap()
        .to_owned();
    assert_eq!(
        fs::metadata(parent.join("private-stderr.log"))
            .unwrap()
            .len(),
        8 * 1024 * 1024
    );
    assert_eq!(
        worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .verdict,
        Some(Verdict::Pass)
    );
}

#[tokio::test]
async fn exclusive_worker_ownership_recovery_and_stored_input_integrity() {
    let runtime = Runtime::new(5);
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    assert_eq!(
        Worker::open(runtime.cfg.clone()).await.err().unwrap().code,
        "WORKER_BUSY"
    );
    let job = runtime.job(&worker.store).await;
    let claim = worker.store.claim(now() - 31_000).await.unwrap().unwrap();
    let args = worker.container_args("orphan", None, false);
    let label = &args[args.iter().position(|s| s == "--label").unwrap() + 1];
    let owned = "a".repeat(64);
    let foreign = "c".repeat(64);
    fs::write(runtime.root.join("containers.json"),serde_json::to_vec(&json!({owned.clone():{"name":"orphan","label":label,"probe":false},foreign.clone():{"name":"foreign","label":"org.doppelganger.worker=foreign","probe":false}})).unwrap()).unwrap();
    worker.recover().await.unwrap();
    assert!(runtime.inventory().get(&owned).is_none());
    assert!(runtime.inventory().get(&foreign).is_some());
    assert_eq!(
        worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .error_code
            .as_deref(),
        Some("WORKER_LOST")
    );
    assert!(!worker.store.heartbeat(&claim, now()).await.unwrap());
    let job = runtime.job(&worker.store).await;
    fs::write(
        runtime
            .cfg
            .storage_root
            .join("submissions")
            .join(&job.submission_id)
            .join("archive.zip"),
        b"changed upload",
    )
    .unwrap();
    worker.run_once().await.unwrap();
    assert_eq!(
        worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .error_code
            .as_deref(),
        Some("INPUT_CHANGED")
    );
    let calls = fs::read_to_string(runtime.root.join("calls.jsonl")).unwrap();
    assert!(!calls.lines().any(|line| {
        let args: Vec<String> = serde_json::from_str(line).unwrap();
        args[0] == "start" && !args.last().unwrap().starts_with("dg-probe-")
    }));
}

#[tokio::test]
async fn readiness_and_unconfirmed_removal_preserve_queue_or_live_claim() {
    let runtime = Runtime::new(1);
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    let job = runtime.job(&worker.store).await;
    fs::write(runtime.root.join("unready"), b"").unwrap();
    assert_eq!(
        worker.run_once().await.unwrap_err().code,
        "ISOLATION_UNAVAILABLE"
    );
    assert_eq!(
        worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Queued
    );
    fs::remove_file(runtime.root.join("unready")).unwrap();
    runtime.set("hang", &stream(0, false));
    let refusal = async {
        loop {
            if worker
                .store
                .job("owner", &job.job_id)
                .await
                .unwrap()
                .unwrap()
                .state
                == JobState::Running
            {
                fs::write(runtime.root.join("refuse-removal"), b"").unwrap();
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let (ran, ()) = tokio::join!(worker.run_once(), refusal);
    assert_eq!(ran.unwrap_err().code, "RUNTIME_UNAVAILABLE");
    let job = worker
        .store
        .job("owner", &job.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Running);
    assert_eq!(job.verdict, None);
    assert!(!runtime.inventory().as_object().unwrap().is_empty());
}

#[tokio::test]
async fn storage_links_and_budget_exhaustion_fail_before_claim() {
    let runtime = Runtime::new(5);
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    let job = runtime.job(&worker.store).await;
    let file = fs::File::create(runtime.cfg.storage_root.join("oversize")).unwrap();
    file.set_len(19 * 1024 * 1024 * 1024 + 1).unwrap();
    assert_eq!(worker.run_once().await.unwrap_err().code, "STORAGE_FULL");
    assert_eq!(
        worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Queued
    );
    fs::remove_file(runtime.cfg.storage_root.join("oversize")).unwrap();
    symlink("/tmp", runtime.cfg.storage_root.join("link")).unwrap();
    assert_eq!(worker.run_once().await.unwrap_err().code, "INVALID_STORAGE");
}

#[tokio::test]
async fn version_one_migration_preserves_queued_jobs_and_events() {
    use sqlx::{sqlite::SqliteConnectOptions, Connection, SqliteConnection};
    let runtime = Runtime::new(5);
    fs::create_dir(&runtime.cfg.storage_root).unwrap();
    fs::set_permissions(&runtime.cfg.storage_root, fs::Permissions::from_mode(0o700)).unwrap();
    let db = runtime.cfg.storage_root.join("jobs.sqlite");
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0001_initial.sql"))
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO submissions VALUES('sub_old','owner','{}',?,'sub_old/archive.zip',1,0)",
    )
    .bind("a".repeat(64))
    .execute(&mut connection)
    .await
    .unwrap();
    sqlx::query("INSERT INTO jobs(id,owner,submission_id,state,created_at_ms,updated_at_ms,idempotency_key,request_digest) VALUES('job_old','owner','sub_old','queued',0,0,'retained-key','retained-digest')").execute(&mut connection).await.unwrap();
    sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms) VALUES('job_old','queued',0)")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    assert_eq!(
        worker
            .store
            .job("owner", "job_old")
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Queued
    );
    let events = worker.store.events("owner", "job_old", 0).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message, None);
    let mut connection = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&db))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA user_version")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT idempotency_key FROM jobs")
            .fetch_one(&mut connection)
            .await
            .unwrap(),
        "retained-key"
    );
}

#[tokio::test]
async fn case_capture_failure_preserves_fail_and_internal_exit_four_is_failed() {
    let runtime = Runtime::new(5);
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    let mut unavailable = stream(1, false);
    unavailable.truncate(unavailable.len() - 9);
    frame(&mut unavailable, CASE_UNAVAILABLE, &[]).unwrap();
    frame(&mut unavailable, DONE, &[]).unwrap();
    runtime.set("normal", &unavailable);
    let job = runtime.job(&worker.store).await;
    worker.run_once().await.unwrap();
    assert_eq!(
        worker
            .store
            .job("owner", &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .verdict,
        Some(Verdict::Fail)
    );
    assert!(worker
        .store
        .events("owner", &job.job_id, 0)
        .await
        .unwrap()
        .iter()
        .any(|e| e.message.as_deref() == Some("PORTABLE_CASE_UNAVAILABLE")));
    let mut internal = MAGIC.to_vec();
    frame(&mut internal, RUNNING, &[]).unwrap();
    frame(&mut internal, PUBLISHING, &[]).unwrap();
    frame(&mut internal, ENGINE_EXIT, &[4]).unwrap();
    frame(&mut internal,ORIGINAL,br#"{"schema_version":1,"result":"ERROR","exit_code":4,"diagnostics":[{"message":"internal","exit_code":4}]}"#).unwrap();
    frame(&mut internal, DONE, &[]).unwrap();
    runtime.set("normal", &internal);
    let job = runtime.job(&worker.store).await;
    worker.run_once().await.unwrap();
    let job = worker
        .store
        .job("owner", &job.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.engine_exit_code, Some(4));
    assert_eq!(job.error_code.as_deref(), Some("ENGINE_INTERNAL_ERROR"));
    assert!(worker
        .store
        .artifacts("owner", &job.job_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn requested_shutdown_stops_active_attempt_and_refuses_new_work() {
    let runtime = Runtime::new(5);
    runtime.set("hang", &stream(0, false));
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    let job = runtime.job(&worker.store).await;
    let shutdown = async {
        loop {
            if worker
                .store
                .job("owner", &job.job_id)
                .await
                .unwrap()
                .unwrap()
                .state
                == JobState::Running
            {
                worker.request_shutdown();
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let (ran, ()) = timeout(Duration::from_secs(10), async {
        tokio::join!(worker.run_once(), shutdown)
    })
    .await
    .unwrap();
    assert_eq!(ran.unwrap_err().code, "WORKER_SHUTDOWN");
    assert_eq!(runtime.inventory(), json!({}));
    let result = worker
        .store
        .job("owner", &job.job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.state, JobState::Failed);
    assert_eq!(result.verdict, Some(Verdict::Error));
    assert_eq!(worker.run_once().await.unwrap_err().code, "WORKER_SHUTDOWN");
}

#[tokio::test]
async fn worker_arguments_have_only_readonly_input_and_bounded_scratch() {
    let runtime = Runtime::new(5);
    assert!(runtime.cfg.validate().is_ok());
    let mut invalid = runtime.cfg.clone();
    invalid.image = "latest".into();
    assert!(invalid.validate().is_err());
    invalid = runtime.cfg.clone();
    invalid.socket = "relative".into();
    assert!(invalid.validate().is_err());
    let worker = Worker::open(runtime.cfg.clone()).await.unwrap();
    let args = worker.container_args("job", Some(std::path::Path::new("/private/input")), false);
    for flag in [
        "--read-only",
        "--network=none",
        "--ipc=none",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges:true",
        "--user=10001:10001",
        "--memory=2147483648",
        "--memory-swap=2147483648",
        "--cpus=2",
        "--pids-limit=128",
        "--pull=never",
        "--log-driver=none",
    ] {
        assert!(args.iter().any(|arg| arg == flag));
    }
    assert_eq!(
        args.iter().filter(|arg| arg.as_str() == "--mount").count(),
        1
    );
    assert!(args
        .iter()
        .any(|arg| arg == "type=bind,src=/private/input,dst=/submission,readonly"));
    assert!(args
        .iter()
        .any(|arg| arg
            == "/work:rw,noexec,nosuid,nodev,size=2147483648,uid=10001,gid=10001,mode=0700"));
    assert!(sandbox::verify_engine(std::path::Path::new("/bin/true")).is_err());
}
