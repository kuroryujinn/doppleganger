//! One-host worker. Production execution always goes through the checked Docker runtime.
use crate::{
    model::*, protocol, publish::ValidatedReport, sandbox, store::Store, Result, ServiceError,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::mpsc,
    time::{Instant, MissedTickBehavior},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub storage_root: PathBuf,
    pub docker: PathBuf,
    pub socket: PathBuf,
    pub image: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    300
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_millis()
        .try_into()
        .expect("timestamp fits i64")
}
fn error(code: &str, message: &str) -> ServiceError {
    ServiceError::new(code, message)
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn opaque(s: &str, prefix: &str) -> bool {
    s.strip_prefix(prefix)
        .is_some_and(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let info = fs::symlink_metadata(path)?;
    if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
        return Err(error(
            "INVALID_STORAGE",
            "Worker directories must be private regular directories",
        ));
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

pub(crate) fn storage_usage(path: &Path) -> Result<u64> {
    fn size(path: &Path, count: &mut usize) -> Result<u64> {
        let mut total = 0u64;
        for item in fs::read_dir(path)? {
            *count += 1;
            if *count > 100_000 {
                return Err(error("STORAGE_FULL", "Storage inventory exceeds limit"));
            }
            let path = item?.path();
            let m = fs::symlink_metadata(&path)?;
            let n = if m.is_dir() {
                size(&path, count)?
            } else if m.is_file() {
                m.len()
            } else {
                return Err(error(
                    "INVALID_STORAGE",
                    "Storage contains links or special files",
                ));
            };
            total = total
                .checked_add(n)
                .ok_or_else(|| error("STORAGE_FULL", "Storage budget exceeded"))?;
        }
        Ok(total)
    }

    size(path, &mut 0)
}

impl WorkerConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = dg_analysis::read_bounded(path, 32 * 1024)?;
        let cfg: Self = serde_json::from_slice(&bytes)
            .map_err(|_| error("INVALID_CONFIG", "Invalid worker configuration"))?;
        cfg.validate()?;
        Ok(cfg)
    }
    pub fn validate(&self) -> Result<()> {
        for path in [&self.storage_root, &self.docker, &self.socket] {
            if !path.is_absolute()
                || path
                    .to_str()
                    .is_none_or(|p| p.contains([',', '\n', '\r', '\0']))
            {
                return Err(error(
                    "INVALID_CONFIG",
                    "Worker paths must be absolute local paths",
                ));
            }
        }
        if !self.image.strip_prefix("sha256:").is_some_and(hex)
            || self.timeout_seconds == 0
            || self.timeout_seconds > 300
        {
            return Err(error(
                "INVALID_CONFIG",
                "Require a local image digest and a timeout of 1–300 seconds",
            ));
        }
        Ok(())
    }
}

pub struct Worker {
    cfg: WorkerConfig,
    pub store: Store,
    _lock: File,
    namespace: String,
    shutdown: AtomicBool,
}
pub struct Capture {
    pub status: i32,
    pub stdout: Vec<u8>,
}

async fn drain(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = vec![0u8; 8192];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        let count = n.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..count]);
        truncated |= count < n;
    }
    Ok((bytes, truncated))
}

async fn drain_logs(mut reader: impl AsyncRead + Unpin, mut file: File) -> Result<(u64, bool)> {
    let mut retained = 0u64;
    let mut truncated = false;
    let mut buffer = vec![0u8; 8192];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        let count = n.min((8 * 1024 * 1024u64).saturating_sub(retained) as usize);
        file.write_all(&buffer[..count])?;
        retained += count as u64;
        truncated |= count < n;
    }
    file.sync_all()?;
    Ok((retained, truncated))
}

impl Worker {
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
    pub fn shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }
    fn shutdown_check(&self) -> Result<()> {
        if self.shutting_down() {
            Err(error("WORKER_SHUTDOWN", "Worker interrupted"))
        } else {
            Ok(())
        }
    }
    pub async fn open(cfg: WorkerConfig) -> Result<Self> {
        cfg.validate()?;
        private_dir(&cfg.storage_root)?;
        for name in ["submissions", "jobs", "docker-config"] {
            private_dir(&cfg.storage_root.join(name))?;
        }
        let lockpath = cfg.storage_root.join("worker.lock");
        if fs::symlink_metadata(&lockpath).is_ok_and(|m| !m.is_file()) {
            return Err(error("INVALID_STORAGE", "Worker lock cannot be a link"));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lockpath)?;
        lock.try_lock()
            .map_err(|_| error("WORKER_BUSY", "Another worker owns this storage root"))?;
        let canonical = cfg.storage_root.canonicalize()?;
        let namespace = format!(
            "{:x}",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        );
        let store = Store::open(&cfg.storage_root.join("jobs.sqlite")).await?;
        Ok(Self {
            cfg,
            store,
            _lock: lock,
            namespace,
            shutdown: AtomicBool::new(false),
        })
    }
    fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.cfg.docker);
        command.args([
            "--host",
            &format!("unix://{}", self.cfg.socket.display()),
            "--config",
            self.cfg
                .storage_root
                .join("docker-config")
                .to_str()
                .expect("validated path"),
        ]);
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
    async fn control(&self, args: &[&str]) -> Result<Capture> {
        let args = args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let mut child = self
            .command(&args)
            .spawn()
            .map_err(|_| error("RUNTIME_UNAVAILABLE", "Cannot start Docker CLI"))?;
        let out = child
            .stdout
            .take()
            .ok_or_else(|| error("RUNTIME_UNAVAILABLE", "Missing runtime stdout"))?;
        let err = child
            .stderr
            .take()
            .ok_or_else(|| error("RUNTIME_UNAVAILABLE", "Missing runtime stderr"))?;
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            let (status, out, err) = tokio::join!(
                child.wait(),
                drain(out, 1024 * 1024),
                drain(err, 1024 * 1024)
            );
            let (stdout, too_much) = out?;
            let (_, err_overflow) = err?;
            if too_much || err_overflow {
                return Err(error(
                    "RUNTIME_UNAVAILABLE",
                    "Runtime control output exceeds limit",
                ));
            }
            Ok(Capture {
                status: status?.code().unwrap_or(-1),
                stdout,
            })
        })
        .await
        .map_err(|_| error("RUNTIME_UNAVAILABLE", "Runtime control timed out"))?;
        result
    }

    pub fn container_args(&self, name: &str, input: Option<&Path>, probe: bool) -> Vec<String> {
        let mut args = vec![
            "create".into(),
            "--pull=never".into(),
            "--name".into(),
            name.into(),
            "--label".into(),
            format!("org.doppelganger.worker={}", self.namespace),
            "--read-only".into(),
            "--network=none".into(),
            "--ipc=none".into(),
            "--cap-drop=ALL".into(),
            "--security-opt=no-new-privileges:true".into(),
            "--user=10001:10001".into(),
            "--memory=2147483648".into(),
            "--memory-swap=2147483648".into(),
            "--cpus=2".into(),
            "--pids-limit=128".into(),
            "--ulimit=fsize=134217728:134217728".into(),
            "--tmpfs".into(),
            "/work:rw,noexec,nosuid,nodev,size=2147483648,uid=10001,gid=10001,mode=0700".into(),
            "--log-driver=none".into(),
            "--entrypoint=/opt/dg-sandbox".into(),
        ];
        if let Some(path) = input {
            args.extend([
                "--mount".into(),
                format!("type=bind,src={},dst=/submission,readonly", path.display()),
            ]);
        }
        args.push(self.cfg.image.clone());
        if probe {
            args.push("--probe".into());
        }
        args
    }
    async fn create(&self, name: &str, input: Option<&Path>, probe: bool) -> Result<()> {
        let args = self.container_args(name, input, probe);
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        if self.control(&refs).await?.status != 0 {
            return Err(error("SANDBOX_ERROR", "Container creation failed"));
        }
        Ok(())
    }
    pub async fn check(&self) -> Result<()> {
        let result = Box::pin(self.check_runtime()).await;
        self.store
            .record_worker_ready(result.is_ok(), now())
            .await?;
        result
    }
    async fn check_runtime(&self) -> Result<()> {
        self.shutdown_check()?;
        if !fs::symlink_metadata(&self.cfg.socket).is_ok_and(|m| m.file_type().is_socket()) {
            return Err(error(
                "RUNTIME_UNAVAILABLE",
                "Require a local rootless Docker Unix socket",
            ));
        }
        let info = self.control(&["info", "--format", "{{json .}}"]).await?;
        if info.status != 0 {
            return Err(error(
                "RUNTIME_UNAVAILABLE",
                "Runtime identity check failed",
            ));
        }
        let info: Value = serde_json::from_slice(&info.stdout)
            .map_err(|_| error("RUNTIME_UNAVAILABLE", "Invalid runtime identity"))?;
        if info["OSType"] != "linux"
            || info["Architecture"] != "x86_64"
            || info["CgroupVersion"] != "2"
            || !info["SecurityOptions"].as_array().is_some_and(|a| {
                a.iter()
                    .any(|v| v.as_str().is_some_and(|s| s == "name=rootless"))
            })
            || !info["SecurityOptions"].as_array().is_some_and(|a| {
                a.iter()
                    .any(|v| v.as_str().is_some_and(|s| s.starts_with("name=seccomp")))
            })
        {
            return Err(error(
                "ISOLATION_UNAVAILABLE",
                "Require Linux x86_64 rootless Docker, cgroups v2 and seccomp",
            ));
        }
        let image = self.control(&["image", "inspect", &self.cfg.image]).await?;
        if image.status != 0 {
            return Err(error("IMAGE_IDENTITY_ERROR", "Pinned image is unavailable"));
        }
        let image: Value = serde_json::from_slice(&image.stdout)
            .map_err(|_| error("IMAGE_IDENTITY_ERROR", "Cannot inspect pinned image"))?;
        if image[0]["Id"] != self.cfg.image
            || image[0]["Config"]["Labels"]["org.doppelganger.engine-sha256"]
                != sandbox::engine_digest()
            || image[0]["Config"]["Labels"]["org.doppelganger.protocol"] != "1"
        {
            return Err(error(
                "IMAGE_IDENTITY_ERROR",
                "Runtime image identity does not match the qualified engine/protocol",
            ));
        }
        let name = format!("dg-probe-{}", uuid::Uuid::new_v4().simple());
        let result = async {
            self.create(&name, None, true).await?;
            let output = self.control(&["start", "--attach", &name]).await?;
            let probe: Value = serde_json::from_slice(&output.stdout)
                .map_err(|_| error("ISOLATION_UNAVAILABLE", "Kernel isolation probe failed"))?;
            if output.status != 0
                || probe["ready"] != true
                || probe["protocol"] != 1
                || probe["engine_sha256"] != sandbox::engine_digest()
            {
                return Err(error(
                    "ISOLATION_UNAVAILABLE",
                    "Kernel isolation probe failed",
                ));
            }
            Ok(())
        }
        .await;
        let cleanup = self.stop_owned(&name).await;
        result?;
        cleanup?;
        self.capacity()?;
        self.shutdown_check()?;
        Ok(())
    }
    async fn stop_owned(&self, name: &str) -> Result<()> {
        let found = self
            .control(&[
                "ps",
                "-aq",
                "--filter",
                &format!("label=org.doppelganger.worker={}", self.namespace),
                "--filter",
                &format!("name=^/{name}$"),
            ])
            .await?;
        if found.status != 0 {
            return Err(error(
                "RUNTIME_UNAVAILABLE",
                "Cannot reconcile container ownership",
            ));
        }
        let ids = std::str::from_utf8(&found.stdout)
            .map_err(|_| error("RUNTIME_UNAVAILABLE", "Invalid container inventory"))?;
        for id in ids.lines().filter(|s| !s.is_empty()) {
            if id.len() > 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(error("RUNTIME_UNAVAILABLE", "Invalid container identity"));
            }
            if self.control(&["rm", "--force", id]).await?.status != 0 {
                return Err(error(
                    "RUNTIME_UNAVAILABLE",
                    "Cannot confirm container stopped",
                ));
            }
        }
        Ok(())
    }
    pub async fn recover(&self) -> Result<()> {
        let found = self
            .control(&[
                "ps",
                "-aq",
                "--filter",
                &format!("label=org.doppelganger.worker={}", self.namespace),
            ])
            .await?;
        if found.status != 0 {
            return Err(error(
                "RUNTIME_UNAVAILABLE",
                "Cannot inventory owned containers",
            ));
        }
        let ids = std::str::from_utf8(&found.stdout)
            .map_err(|_| error("RUNTIME_UNAVAILABLE", "Invalid inventory"))?;
        for id in ids.lines().filter(|s| !s.is_empty()) {
            if id.len() > 64
                || !id.bytes().all(|b| b.is_ascii_hexdigit())
                || self.control(&["rm", "--force", id]).await?.status != 0
            {
                return Err(error("RUNTIME_UNAVAILABLE", "Cannot stop orphan container"));
            }
        }
        for claim in self.store.expired_claims(now()).await? {
            if self.store.claim_state(&claim).await? == Some(JobState::Cancelling) {
                self.store.acknowledge_cancel(&claim, now()).await?;
            } else {
                self.store.reconcile_lost(&claim, now()).await?;
            }
        }
        Ok(())
    }
    fn capacity(&self) -> Result<()> {
        let stat = rustix::fs::statvfs(&self.cfg.storage_root)
            .map_err(|_| error("STORAGE_ERROR", "Cannot inspect storage capacity"))?;
        if stat.f_bavail.saturating_mul(stat.f_frsize) < 3 * 1024 * 1024 * 1024 {
            return Err(error(
                "STORAGE_FULL",
                "Keep 2 GiB free plus bounded job staging reserve",
            ));
        }
        if storage_usage(&self.cfg.storage_root)? > 19 * 1024 * 1024 * 1024 {
            return Err(error(
                "STORAGE_FULL",
                "Persistent storage needs cleanup before another job",
            ));
        }
        Ok(())
    }
    async fn stage_input(&self, claim: &Claim, directory: &Path) -> Result<()> {
        let submission = self
            .store
            .worker_submission(claim, now())
            .await?
            .ok_or_else(|| error("LEASE_LOST", "Claim expired before staging"))?;
        if submission.storage_key != format!("{}/archive.zip", claim.submission_id)
            || !opaque(&claim.submission_id, "sub_")
        {
            return Err(error("INPUT_CHANGED", "Invalid stored archive identity"));
        }
        let source = self
            .cfg
            .storage_root
            .join("submissions")
            .join(&submission.storage_key);
        let parent = source
            .parent()
            .ok_or_else(|| error("INPUT_CHANGED", "Missing archive directory"))?;
        if !fs::symlink_metadata(parent)?.is_dir() {
            return Err(error(
                "INPUT_CHANGED",
                "Submission directory cannot be a link",
            ));
        }
        let info = fs::symlink_metadata(&source)?;
        if !info.is_file()
            || info.len() != submission.bytes as u64
            || info.len() > 384 * 1024 * 1024
        {
            return Err(error(
                "INPUT_CHANGED",
                "Archive no longer matches its receipt",
            ));
        }
        let mut input = File::open(source)?;
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("archive.zip"))?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        let mut heartbeat = Instant::now();
        loop {
            self.shutdown_check()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > 384 * 1024 * 1024 {
                return Err(error("INPUT_CHANGED", "Archive grew during staging"));
            }
            hash.update(&buffer[..n]);
            target.write_all(&buffer[..n])?;
            // Cancel only between synchronous bounded writes: no detached filesystem task survives cleanup.
            tokio::task::yield_now().await;
            if heartbeat.elapsed() >= Duration::from_secs(1) {
                if self.store.claim_state(claim).await? == Some(JobState::Cancelling) {
                    return Err(error("JOB_CANCELLED", "Staging cancelled"));
                }
                if !self.store.heartbeat(claim, now()).await? {
                    return Err(error("LEASE_LOST", "Staging lease expired"));
                }
                self.store.record_worker_ready(true, now()).await?;
                heartbeat = Instant::now();
            }
        }
        if total != submission.bytes as u64 || format!("{:x}", hash.finalize()) != submission.sha256
        {
            return Err(error(
                "INPUT_CHANGED",
                "Archive differs from its immutable receipt",
            ));
        }
        target.sync_all()?;
        drop(target);
        write_new(
            &directory.join("metadata.json"),
            &serde_json::to_vec(&submission.metadata)
                .map_err(|_| error("STORAGE_ERROR", "Cannot serialize metadata"))?,
        )?;
        for file in ["archive.zip", "metadata.json"] {
            fs::set_permissions(directory.join(file), fs::Permissions::from_mode(0o444))?;
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(0o555))?;
        File::open(directory)?.sync_all()?;
        Ok(())
    }

    pub async fn run_once(&self) -> Result<bool> {
        let result = Box::pin(self.run_attempt()).await;
        if result.is_err() {
            self.store.record_worker_ready(false, now()).await?;
        }
        result
    }
    async fn run_attempt(&self) -> Result<bool> {
        Box::pin(self.check()).await?;
        let Some(claim) = self.store.claim(now()).await? else {
            return Ok(false);
        };
        if !opaque(&claim.job_id, "job_") || !opaque(&claim.lease_token, "lease_") {
            return Err(error("STORAGE_ERROR", "Invalid persisted claim identity"));
        }
        let jobdir = self.cfg.storage_root.join("jobs").join(&claim.job_id);
        private_dir(&jobdir)?;
        let attempt = jobdir.join(&claim.lease_token);
        private_dir(&attempt)?;
        let input = attempt.join("input");
        private_dir(&input)?;
        let output = attempt.join("staging");
        private_dir(&output)?;
        let name = format!("dg-{}", &claim.lease_token[6..]);
        let result = Box::pin(self.execute(&claim, &name, &input, &output)).await;
        // Never release a claim or publish a terminal failure before confirming container removal.
        self.stop_owned(&name).await?;
        if input.exists() {
            fs::set_permissions(&input, fs::Permissions::from_mode(0o700))?;
            fs::remove_dir_all(&input)?;
        }
        match result {
            Ok(()) => {}
            Err(e) => {
                if self.store.claim_state(&claim).await? == Some(JobState::Cancelling) {
                    self.store.acknowledge_cancel(&claim, now()).await?;
                } else {
                    let reason = match e.code.as_str() {
                        "JOB_TIMEOUT" => FailureReason::Timeout,
                        "INPUT_CHANGED" => FailureReason::InputChanged,
                        "ENGINE_PROTOCOL_ERROR" => FailureReason::EngineProtocol,
                        "ENGINE_INTERNAL_ERROR" => FailureReason::EngineInternal,
                        "STORAGE_ERROR" | "OUTPUT_TOO_LARGE" => FailureReason::Storage,
                        _ => FailureReason::Sandbox,
                    };
                    if !self.store.fail_attempt(&claim, reason, now()).await? {
                        self.store.reconcile_lost(&claim, now()).await?;
                    }
                }
                if e.code == "WORKER_SHUTDOWN" {
                    return Err(e);
                }
            }
        }
        Ok(true)
    }

    async fn execute(&self, claim: &Claim, name: &str, input: &Path, output: &Path) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(self.cfg.timeout_seconds);
        tokio::time::timeout_at(deadline, self.stage_input(claim, input))
            .await
            .map_err(|_| error("JOB_TIMEOUT", "Input staging timed out"))??;
        if self.store.claim_state(claim).await? == Some(JobState::Cancelling) {
            return Err(error("JOB_CANCELLED", "Job cancelled"));
        }
        if !self.store.heartbeat(claim, now()).await? {
            return Err(error(
                "LEASE_LOST",
                "Lease expired before container creation",
            ));
        }
        self.create(name, Some(input), false).await?;
        if Instant::now() >= deadline {
            return Err(error(
                "JOB_TIMEOUT",
                "Container creation exceeded job deadline",
            ));
        }
        self.shutdown_check()?;
        if self.store.claim_state(claim).await? == Some(JobState::Cancelling) {
            return Err(error("JOB_CANCELLED", "Creation cancelled"));
        }
        let args = vec!["start".into(), "--attach".into(), name.into()];
        let mut child = self.command(&args).spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("ENGINE_PROTOCOL_ERROR", "Missing sandbox stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| error("ENGINE_PROTOCOL_ERROR", "Missing sandbox stderr"))?;
        let (sender, mut stages) = mpsc::channel(4);
        let collect = protocol::collect(stdout, output, sender);
        tokio::pin!(collect);
        let private_log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(output.join("private-stderr.log"))?;
        let logs = drain_logs(stderr, private_log);
        tokio::pin!(logs);
        let mut captured = None;
        let mut log_result = None;
        let mut current = JobState::Validating;
        let mut channel_open = true;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _=tokio::time::sleep_until(deadline)=>return Err(error("JOB_TIMEOUT","Sandbox exceeded job timeout")),
                _=tokio::signal::ctrl_c()=>return Err(error("WORKER_SHUTDOWN","Worker interrupted")),
                _=tick.tick()=>{
                    self.shutdown_check()?;
                    self.store.record_worker_ready(true,now()).await?;
                    if self.store.claim_state(claim).await?==Some(JobState::Cancelling){return Err(error("JOB_CANCELLED","Job cancelled"));}
                    if !self.store.heartbeat(claim,now()).await?{return Err(error("LEASE_LOST","Worker lease lost"));}
                }
                stage=stages.recv(),if channel_open=>{
                    if let Some(stage)=stage{if !self.store.advance(claim,current,stage,now()).await?{return Err(error("LEASE_LOST","Stage update was fenced"));}current=stage;}else{channel_open=false;}
                }
                result=&mut collect,if captured.is_none()=>captured=Some(result?),
                result=&mut logs,if log_result.is_none()=>log_result=Some(result?),
            }
            if captured.is_some() && log_result.is_some() && !channel_open {
                break;
            }
        }
        let status = tokio::time::timeout_at(deadline, child.wait())
            .await
            .map_err(|_| error("JOB_TIMEOUT", "Runtime attach did not exit"))??;
        if !status.success() {
            return Err(error(
                "SANDBOX_ERROR",
                "Sandbox exited without a successful wrapper status",
            ));
        }
        let collected =
            captured.ok_or_else(|| error("ENGINE_PROTOCOL_ERROR", "Missing final output"))?;
        if collected.logs_truncated || log_result.as_ref().is_some_and(|(_, truncated)| *truncated)
        {
            self.store.note(claim, "LOG_TRUNCATED", now()).await?;
        }
        if collected.portable_unavailable {
            self.store
                .note(claim, "PORTABLE_CASE_UNAVAILABLE", now())
                .await?;
        }
        let (log_size, _) = log_result.unwrap_or_default();
        self.stop_owned(name).await?;
        if !self.store.heartbeat(claim, now()).await? {
            return Err(error("LEASE_LOST", "Publication lease expired"));
        }
        if self.store.claim_state(claim).await? == Some(JobState::Cancelling) {
            return Err(error("JOB_CANCELLED", "Publication cancelled"));
        }
        self.shutdown_check()?;
        let raw = dg_analysis::read_bounded(
            &output.join("original.json"),
            protocol::OUTPUT_LIMIT as usize,
        )?;
        let report = ValidatedReport::parse(Some(collected.reported_exit), &raw)?;
        if report.engine_exit_code() != collected.engine_exit {
            return Err(error(
                "ENGINE_PROTOCOL_ERROR",
                "Engine invocation evidence disagrees with wrapper",
            ));
        }
        if report.state() == JobState::Failed {
            return Err(error("ENGINE_INTERNAL_ERROR", "Engine internal failure"));
        }
        let client = report.client_view("/work/input")?;
        write_new(
            &output.join("report.json"),
            &serde_json::to_vec_pretty(&client)
                .map_err(|_| error("ENGINE_PROTOCOL_ERROR", "Cannot encode client report"))?,
        )?;
        let mut total = 0u64;
        let mut artifacts = vec![];
        for (kind, file) in [
            ("report_json", "report.json"),
            ("report_html", "report.html"),
            ("summary", "summary.md"),
            ("portable_case", "portable_case.zip"),
        ] {
            let path = output.join(file);
            if !path.exists() {
                continue;
            }
            let bytes = dg_analysis::read_bounded(&path, protocol::OUTPUT_LIMIT as usize)?;
            total += bytes.len() as u64;
            artifacts.push(Artifact {
                kind: kind.into(),
                storage_key: format!(
                    "jobs/{}/{}/published/{file}",
                    claim.job_id, claim.lease_token
                ),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                bytes: bytes.len() as i64,
            });
        }
        total += fs::metadata(output.join("original.json"))?.len() + log_size;
        if total > protocol::OUTPUT_LIMIT {
            return Err(error(
                "OUTPUT_TOO_LARGE",
                "Published evidence exceeds job budget",
            ));
        }
        File::open(output)?.sync_all()?;
        let published = output
            .parent()
            .ok_or_else(|| error("STORAGE_ERROR", "Missing attempt directory"))?
            .join("published");
        if published.exists() {
            return Err(error(
                "STORAGE_ERROR",
                "Cannot overwrite published evidence",
            ));
        }
        fs::rename(output, &published)?;
        File::open(published.parent().expect("attempt parent"))?.sync_all()?;
        if Instant::now() >= deadline {
            return Err(error("JOB_TIMEOUT", "Publication exceeded job deadline"));
        }
        self.shutdown_check()?;
        if !self
            .store
            .complete(claim, &report, &artifacts, now())
            .await?
        {
            return Err(error(
                "LEASE_LOST",
                "Publication fenced by cancellation or expired lease",
            ));
        }
        Ok(())
    }
}
