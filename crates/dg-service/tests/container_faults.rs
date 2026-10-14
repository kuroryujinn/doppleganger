//! Trusted fault injection against actual rootless Docker, separate from the production image.
use dg_service::{
    intake::{receive, Limits},
    model::{Artifact, Claim, Job, JobState, PreparedUpload, Verdict},
    publish::ValidatedReport,
    runner::{now, Worker, WorkerConfig},
    store::Store,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tempfile::TempDir;
use tokio::{
    process::Command,
    time::{sleep, timeout, Duration, Instant},
};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, ZipWriter};

const OWNER: &str = "fault-qualification";

struct Runtime {
    directory: TempDir,
    cfg: WorkerConfig,
    fault_image: String,
    base_alias: String,
    containers: Vec<String>,
}
impl Runtime {
    async fn new() -> Self {
        let cfg = WorkerConfig::load(Path::new(
            &std::env::var("DG_WORKER_CONFIG").expect("DG_WORKER_CONFIG"),
        ))
        .unwrap();
        let directory = TempDir::new_in(cfg.storage_root.parent().unwrap()).unwrap();
        let mut runtime = Self {
            directory,
            cfg,
            fault_image: String::new(),
            base_alias: String::new(),
            containers: vec![],
        };
        let context = runtime.directory.path().join("image");
        fs::create_dir(&context).unwrap();
        fs::write(
            context.join("fault.c"),
            include_str!("fixtures/container_fault.c"),
        )
        .unwrap();
        let compiled = Command::new("/usr/bin/cc")
            .args(["-static", "-O2", "-Wall", "-Wextra", "-Werror"])
            .arg(context.join("fault.c"))
            .arg("-o")
            .arg(context.join("fault"))
            .output()
            .await
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        // BuildKit interprets a bare image ID as a registry name. Retain a local base alias.
        let base = format!(
            "doppelganger-qualification-base:{}",
            Uuid::new_v4().simple()
        );
        runtime.base_alias = base.clone();
        assert!(runtime
            .docker(&["image", "tag", &runtime.cfg.image, &base])
            .await
            .status
            .success());
        let inspect = runtime.docker(&["image", "inspect", &base]).await;
        let identity: Value = serde_json::from_slice(&inspect.stdout).unwrap();
        assert_eq!(identity[0]["Id"], runtime.cfg.image);
        fs::write(context.join("Dockerfile"),format!("FROM {base} AS original\nFROM {base}\nCOPY --from=original /opt/dg-sandbox /opt/dg-sandbox-real\nCOPY fault /opt/dg-sandbox\nLABEL org.doppelganger.fault-test=\"true\"\n")).unwrap();
        let iid = context.join("image.id");
        let output = runtime
            .docker(&[
                "build",
                "--network=none",
                "--pull=false",
                "--iidfile",
                iid.to_str().unwrap(),
                context.to_str().unwrap(),
            ])
            .await;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        runtime.fault_image = fs::read_to_string(iid).unwrap().trim().into();
        runtime
    }
    async fn docker(&self, args: &[&str]) -> std::process::Output {
        timeout(
            Duration::from_secs(30),
            Command::new(&self.cfg.docker)
                .args([
                    "--host",
                    &format!("unix://{}", self.cfg.socket.display()),
                    "--config",
                    self.directory.path().to_str().unwrap(),
                ])
                .args(args)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap()
    }
    fn config(&self, name: &str, seconds: u64, faults: bool) -> WorkerConfig {
        WorkerConfig {
            storage_root: self.directory.path().join(name),
            image: if faults {
                self.fault_image.clone()
            } else {
                self.cfg.image.clone()
            },
            timeout_seconds: seconds,
            ..self.cfg.clone()
        }
    }
    async fn worker(&self, cfg: &WorkerConfig) -> Arc<Worker> {
        let worker = Arc::new(Worker::open(cfg.clone()).await.unwrap());
        worker.check().await.unwrap();
        worker
    }
    async fn submit(&self, cfg: &WorkerConfig, store: &Store, data: &[u8]) -> Job {
        let receipt = receive(
            &cfg.storage_root.join("submissions"),
            Cursor::new(data),
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
        let id = store
            .add_submission(OWNER, &metadata, &receipt, now())
            .await
            .unwrap();
        store.enqueue(OWNER, &id, None, now()).await.unwrap()
    }
    async fn running(&mut self, cfg: &WorkerConfig, store: &Store, job: &Job) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let current = store.job(OWNER, &job.job_id).await.unwrap().unwrap();
            if current.state == JobState::Running {
                let attempt = fs::read_dir(cfg.storage_root.join("jobs").join(&job.job_id))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap();
                let lease = attempt.file_name().into_string().unwrap();
                let name = format!("dg-{}", lease.strip_prefix("lease_").unwrap());
                self.containers.push(name.clone());
                return name;
            }
            assert!(
                !matches!(current.state, JobState::Failed | JobState::Completed),
                "Job ended before injection: {current:?}"
            );
            assert!(Instant::now() < deadline, "Job never entered running");
            sleep(Duration::from_millis(10)).await;
        }
    }
    async fn absent(&self, name: &str) {
        assert!(
            !self.docker(&["inspect", name]).await.status.success(),
            "Owned container survived: {name}"
        );
    }
    async fn failed(&self, store: &Store, job: &Job, code: Option<&str>) {
        let current = store.job(OWNER, &job.job_id).await.unwrap().unwrap();
        assert_eq!(current.state, JobState::Failed, "{current:?}");
        assert_eq!(current.verdict, Some(Verdict::Error));
        if let Some(code) = code {
            assert_eq!(current.error_code.as_deref(), Some(code));
        }
        assert!(store
            .artifacts(OWNER, &job.job_id)
            .await
            .unwrap()
            .is_empty());
        eprintln!("LIVE_FAULT {}: {}", job.job_id, current.error_code.unwrap());
    }
    fn logs(&self, cfg: &WorkerConfig, job: &Job) -> String {
        let attempt = fs::read_dir(cfg.storage_root.join("jobs").join(&job.job_id))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        fs::read_to_string(attempt.join("staging/private-stderr.log")).unwrap()
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // Only explicitly recorded test containers and the uniquely built test image.
        for name in &self.containers {
            let _ = std::process::Command::new(&self.cfg.docker)
                .args([
                    "--host",
                    &format!("unix://{}", self.cfg.socket.display()),
                    "rm",
                    "--force",
                    name,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if !self.fault_image.is_empty() {
            let _ = std::process::Command::new(&self.cfg.docker)
                .args([
                    "--host",
                    &format!("unix://{}", self.cfg.socket.display()),
                    "image",
                    "rm",
                    &self.fault_image,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if !self.base_alias.is_empty() {
            let _ = std::process::Command::new(&self.cfg.docker)
                .args([
                    "--host",
                    &format!("unix://{}", self.cfg.socket.display()),
                    "image",
                    "rm",
                    &self.base_alias,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
fn prepared_vault() -> Vec<u8> {
    let root = PathBuf::from(std::env::var("DG_PREPARED_EXAMPLES").expect("DG_PREPARED_EXAMPLES"))
        .join("vault");
    let mut zip = ZipWriter::new(Cursor::new(vec![]));
    for name in [
        "doppelganger.toml",
        "doppelganger-policy.toml",
        "v1.bin",
        "v2.bin",
        "v1.json",
        "v2.json",
    ] {
        zip.start_file(format!("project/{name}"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&fs::read(root.join(name)).unwrap()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
async fn signal(pid: u32, name: &str) {
    assert!(Command::new("/usr/bin/kill")
        .args([name, &pid.to_string()])
        .status()
        .await
        .unwrap()
        .success());
}
fn spawn_worker(cfg: &WorkerConfig) -> tokio::process::Child {
    let path = cfg.storage_root.join("operator.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "storage_root": cfg.storage_root, "docker": cfg.docker, "socket": cfg.socket,
            "image": cfg.image, "timeout_seconds": cfg.timeout_seconds
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    Command::new(env!("CARGO_BIN_EXE_dg-worker"))
        .arg(path)
        .arg("once")
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            fs::File::create(cfg.storage_root.join("operator.log")).unwrap(),
        ))
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test]
#[ignore = "Required live rootless Docker faults; set DG_WORKER_CONFIG and DG_PREPARED_EXAMPLES"]
async fn live_faults_never_publish_pass_and_recovery_preserves_queued_work() {
    let mut runtime = Runtime::new().await;
    // Cancellation and deadline must remove the complete container/PID tree.
    for (name, seconds, cancel) in [
        ("cancel", 30, true),
        ("timeout", 3, false),
        ("pids", 3, false),
    ] {
        let cfg = runtime.config(name, seconds, true);
        let worker = runtime.worker(&cfg).await;
        let job = runtime
            .submit(
                &cfg,
                &worker.store,
                if name == "pids" { b"P" } else { b"H" },
            )
            .await;
        let runner = worker.clone();
        let attempt = tokio::spawn(async move { runner.run_once().await });
        let container = runtime.running(&cfg, &worker.store, &job).await;
        if cancel {
            assert_eq!(
                worker
                    .store
                    .cancel(OWNER, &job.job_id, now())
                    .await
                    .unwrap()
                    .state,
                JobState::Cancelling
            );
        }
        assert!(timeout(Duration::from_secs(20), attempt)
            .await
            .unwrap()
            .unwrap()
            .unwrap());
        if cancel {
            let result = worker.store.job(OWNER, &job.job_id).await.unwrap().unwrap();
            assert_eq!(result.state, JobState::Cancelled);
            assert_ne!(result.verdict, Some(Verdict::Pass));
            assert!(worker
                .store
                .artifacts(OWNER, &job.job_id)
                .await
                .unwrap()
                .is_empty());
        } else {
            runtime
                .failed(&worker.store, &job, Some("JOB_TIMEOUT"))
                .await;
        }
        if name == "pids" {
            assert!(runtime.logs(&cfg, &job).contains("PIDS_LIMIT_ENFORCED"));
        }
        runtime.absent(&container).await;
        eprintln!("LIVE_FAULT {name}: stopped and fenced");
    }
    // A contradictory purported PASS and file exhaustion have no downloadable results.
    for (name, mode, marker) in [
        ("forged", b"X".as_slice(), None),
        ("file", b"F".as_slice(), Some("FILE_LIMIT_ENFORCED")),
    ] {
        let cfg = runtime.config(name, 10, true);
        let worker = runtime.worker(&cfg).await;
        let job = runtime.submit(&cfg, &worker.store, mode).await;
        assert!(worker.run_once().await.unwrap());
        runtime
            .failed(&worker.store, &job, Some("ENGINE_PROTOCOL_ERROR"))
            .await;
        if let Some(marker) = marker {
            assert!(runtime.logs(&cfg, &job).contains(marker));
        }
    }
    // The same persistent-storage guard still runs after an actual successful probe.
    let cfg = runtime.config("storage", 10, false);
    let worker = runtime.worker(&cfg).await;
    let job = runtime.submit(&cfg, &worker.store, &prepared_vault()).await;
    let sparse = cfg.storage_root.join("budget");
    fs::File::create(&sparse)
        .unwrap()
        .set_len(19 * 1024 * 1024 * 1024)
        .unwrap();
    assert_eq!(worker.run_once().await.unwrap_err().code, "STORAGE_FULL");
    assert_eq!(
        worker
            .store
            .job(OWNER, &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Queued
    );
    fs::remove_file(sparse).unwrap();
    assert!(worker.run_once().await.unwrap());
    assert_eq!(
        worker
            .store
            .job(OWNER, &job.job_id)
            .await
            .unwrap()
            .unwrap()
            .verdict,
        Some(Verdict::Pass)
    );
    drop(worker);

    // Resource OOM: stop only the worker process so Docker's exit evidence can be inspected.
    let cfg = runtime.config("oom", 30, true);
    let worker = runtime.worker(&cfg).await;
    let job = runtime.submit(&cfg, &worker.store, b"M").await;
    let store = worker.store.clone();
    drop(worker);
    let mut process = spawn_worker(&cfg);
    let container = runtime.running(&cfg, &store, &job).await;
    signal(process.id().unwrap(), "-STOP").await;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let inspect = runtime.docker(&["inspect", &container]).await;
        assert!(inspect.status.success());
        let info: Value = serde_json::from_slice(&inspect.stdout).unwrap();
        if info[0]["State"]["Running"] == false {
            assert_eq!(info[0]["State"]["OOMKilled"], true);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Memory limit did not terminate container"
        );
        sleep(Duration::from_millis(50)).await;
    }
    signal(process.id().unwrap(), "-CONT").await;
    assert!(timeout(Duration::from_secs(20), process.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    runtime.failed(&store, &job, None).await;
    runtime.absent(&container).await;
    eprintln!("LIVE_FAULT memory: kernel OOM kill observed");

    // Host process death: cleanup happens immediately, the live lease remains fenced until expiry.
    let cfg = runtime.config("crash", 60, true);
    let worker = runtime.worker(&cfg).await;
    let first = runtime.submit(&cfg, &worker.store, b"H").await;
    let queued = runtime.submit(&cfg, &worker.store, &prepared_vault()).await;
    let store = worker.store.clone();
    drop(worker);
    let mut process = spawn_worker(&cfg);
    let container = runtime.running(&cfg, &store, &first).await;
    let stale: Claim = store
        .expired_claims(now() + 60_000)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.job_id == first.job_id)
        .unwrap();
    process.kill().await.unwrap();
    process.wait().await.unwrap();
    let healthy = WorkerConfig {
        image: runtime.cfg.image.clone(),
        ..cfg.clone()
    };
    let recovered = runtime.worker(&healthy).await;
    recovered.recover().await.unwrap();
    runtime.absent(&container).await;
    assert_eq!(
        store
            .job(OWNER, &first.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Running
    );
    assert!(store.claim(now()).await.unwrap().is_none());
    let remaining = stale.lease_expires_at_ms.saturating_sub(now()).max(0) as u64;
    sleep(Duration::from_millis(remaining + 50)).await;
    recovered.recover().await.unwrap();
    runtime.failed(&store, &first, Some("WORKER_LOST")).await;
    assert!(recovered.run_once().await.unwrap());
    let result = store.job(OWNER, &queued.job_id).await.unwrap().unwrap();
    assert_eq!(result.verdict, Some(Verdict::Pass));
    let artifacts = store.artifacts(OWNER, &queued.job_id).await.unwrap();
    let original = healthy
        .storage_root
        .join(&artifacts[0].storage_key)
        .parent()
        .unwrap()
        .join("original.json");
    let report = ValidatedReport::parse(Some(0), &fs::read(original).unwrap()).unwrap();
    let stale_artifact = Artifact {
        kind: "report_json".into(),
        storage_key: format!("jobs/{}/stale/report.json", stale.job_id),
        sha256: format!("{:x}", Sha256::digest(b"stale")),
        bytes: 5,
    };
    assert!(!store
        .complete(&stale, &report, &[stale_artifact], now())
        .await
        .unwrap());
    eprintln!("LIVE_FAULT crash: orphan removed, real lease expired, stale publication fenced, queue resumed");
    drop(recovered);

    // A different storage namespace cannot see inputs or remove the other root's live job.
    let a = runtime.config("root-a", 60, true);
    let first_worker = runtime.worker(&a).await;
    let job = runtime.submit(&a, &first_worker.store, b"H").await;
    let runner = first_worker.clone();
    let attempt = tokio::spawn(async move { runner.run_once().await });
    let container = runtime.running(&a, &first_worker.store, &job).await;
    let sentinel = a
        .storage_root
        .join("jobs")
        .join(&job.job_id)
        .join("private-host-marker");
    fs::write(&sentinel, b"private").unwrap();
    let b = runtime.config("root-b", 10, true);
    let second_worker = runtime.worker(&b).await;
    second_worker.recover().await.unwrap();
    assert!(runtime
        .docker(&["inspect", &container])
        .await
        .status
        .success());
    let other = runtime
        .submit(
            &b,
            &second_worker.store,
            format!("I{}", sentinel.display()).as_bytes(),
        )
        .await;
    assert!(second_worker.run_once().await.unwrap());
    runtime
        .failed(&second_worker.store, &other, Some("ENGINE_PROTOCOL_ERROR"))
        .await;
    assert!(runtime
        .logs(&b, &other)
        .contains("CROSS_JOB_READS_AND_INPUT_WRITES_DENIED"));
    assert_eq!(fs::read(&sentinel).unwrap(), b"private");
    assert!(runtime
        .docker(&["inspect", &container])
        .await
        .status
        .success());
    first_worker
        .store
        .cancel(OWNER, &job.job_id, now())
        .await
        .unwrap();
    assert!(attempt.await.unwrap().unwrap());
    runtime.absent(&container).await;
    eprintln!("LIVE_FAULT cross-job: private tmpfs, contained inputs, namespace cleanup verified");

    // Graceful host SIGTERM must also stop an active container before releasing its claim.
    let cfg = runtime.config("shutdown", 60, true);
    let worker = runtime.worker(&cfg).await;
    let job = runtime.submit(&cfg, &worker.store, b"H").await;
    let store = worker.store.clone();
    drop(worker);
    let mut process = spawn_worker(&cfg);
    let container = runtime.running(&cfg, &store, &job).await;
    signal(process.id().unwrap(), "-TERM").await;
    assert!(!timeout(Duration::from_secs(20), process.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
    runtime.failed(&store, &job, None).await;
    runtime.absent(&container).await;
    eprintln!("LIVE_FAULT shutdown: active container removed before terminal failure");
}
