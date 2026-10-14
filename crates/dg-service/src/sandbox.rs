//! Container-only entry point. The worker never calls this on uploaded host data.
use crate::{
    intake::{extract, preflight, Limits},
    model::PreparedUpload,
    protocol::*,
    publish::ValidatedReport,
    Result, ServiceError,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    thread,
};
use zip::{write::SimpleFileOptions, ZipWriter};

pub const ENGINE: &str = "/opt/doppelganger";
pub const ENGINE_LOCK: &str = include_str!("../../../docs/backend-engine-lock.json");

pub fn engine_digest() -> String {
    serde_json::from_str::<Value>(ENGINE_LOCK).expect("compiled engine lock")["sha256"]
        .as_str()
        .expect("compiled engine digest")
        .into()
}

pub fn verify_engine(path: &Path) -> Result<()> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > 64 * 1024 * 1024 {
            return Err(ServiceError::new(
                "ENGINE_IDENTITY_ERROR",
                "Engine exceeds identity budget",
            ));
        }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()) != engine_digest() {
        return Err(ServiceError::new(
            "ENGINE_IDENTITY_ERROR",
            "Engine binary differs from qualified identity",
        ));
    }
    Ok(())
}

/// Probe real kernel state rather than trusting Docker flag strings alone.
pub fn probe() -> Result<Value> {
    verify_engine(Path::new(ENGINE))?;
    let status = fs::read_to_string("/proc/self/status")?;
    let field = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or("")
            .trim()
    };
    if field("Uid:").split_whitespace().count() != 4
        || field("Uid:").split_whitespace().any(|u| u != "10001")
        || field("CapEff:") != "0000000000000000"
        || field("CapPrm:") != "0000000000000000"
        || field("CapBnd:") != "0000000000000000"
        || field("NoNewPrivs:") != "1"
        || field("Seccomp:") != "2"
    {
        return Err(isolation());
    }
    let read_limit = |name: &str| -> Result<u64> {
        fs::read_to_string(format!("/sys/fs/cgroup/{name}"))?
            .trim()
            .parse()
            .map_err(|_| isolation())
    };
    if read_limit("memory.max")? > 2 * 1024 * 1024 * 1024
        || read_limit("memory.swap.max")? != 0
        || read_limit("pids.max")? > 128
    {
        return Err(isolation());
    }
    let cpu = fs::read_to_string("/sys/fs/cgroup/cpu.max")?;
    let parts = cpu
        .split_whitespace()
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| isolation())?;
    if parts.len() != 2 || parts[1] == 0 || parts[0] > parts[1].saturating_mul(2) {
        return Err(isolation());
    }
    let mounts = fs::read_to_string("/proc/self/mountinfo")?;
    if !mounts.lines().any(|line| {
        line.split_whitespace().nth(4) == Some("/work")
            && line.contains(" - tmpfs ")
            && line
                .split_whitespace()
                .nth(5)
                .is_some_and(|o| o.split(',').any(|p| p == "noexec"))
    }) {
        return Err(isolation());
    }
    if !mounts.lines().any(|line| {
        line.split_whitespace().nth(4) == Some("/")
            && line
                .split_whitespace()
                .nth(5)
                .is_some_and(|o| o.split(',').any(|p| p == "ro"))
    }) {
        return Err(isolation());
    }
    if fs::read_dir("/sys/class/net")?
        .any(|entry| entry.is_err() || entry.is_ok_and(|e| e.file_name() != "lo"))
    {
        return Err(isolation());
    }
    let limits = fs::read_to_string("/proc/self/limits")?;
    let file_limits = limits
        .lines()
        .find_map(|line| line.strip_prefix("Max file size"))
        .ok_or_else(isolation)?;
    let sizes = file_limits
        .split_whitespace()
        .take(2)
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| isolation())?;
    if sizes.len() != 2 || sizes.iter().any(|&v| v > OUTPUT_LIMIT) {
        return Err(isolation());
    }
    let stat = rustix::fs::statvfs("/work").map_err(|_| isolation())?;
    if stat.f_blocks.saturating_mul(stat.f_frsize) > 2 * 1024 * 1024 * 1024 {
        return Err(isolation());
    }
    if fs::write("/opt/.write-probe", b"x").is_ok() {
        return Err(isolation());
    }
    fs::write("/work/.write-probe", b"x")?;
    fs::remove_file("/work/.write-probe")?;
    Ok(json!({"schema_version":1,"ready":true,"protocol":1,"engine_sha256":engine_digest()}))
}
fn isolation() -> ServiceError {
    ServiceError::new(
        "ISOLATION_UNAVAILABLE",
        "Required kernel isolation or limits are not enforced",
    )
}

fn child(mut command: Command) -> Result<(i32, bool)> {
    // Engine diagnostics are bounded but always drained, independent of the host collector.
    let mut process = command
        .env_clear()
        .current_dir("/work")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = process.stderr.take().ok_or_else(|| {
        ServiceError::new("ENGINE_PROTOCOL_ERROR", "Missing engine diagnostic pipe")
    })?;
    let logs = thread::spawn(move || {
        let mut sent = 0usize;
        let mut truncated = false;
        let mut buffer = [0u8; 8192];
        let mut output = std::io::stderr().lock();
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let write = n.min((8 * 1024 * 1024usize).saturating_sub(sent));
            truncated |= write < n;
            if write > 0 {
                let _ = output.write_all(&buffer[..write]);
                sent += write;
            }
        }
        truncated
    });
    let status = process.wait()?;
    let truncated = logs
        .join()
        .map_err(|_| ServiceError::new("ENGINE_PROTOCOL_ERROR", "Diagnostic collector failed"))?;
    status
        .code()
        .map(|e| (e, truncated))
        .ok_or_else(|| ServiceError::new("ENGINE_PROTOCOL_ERROR", "Engine terminated by signal"))
}

fn case_zip(directory: &Path, destination: &Path, budget: u64) -> Result<()> {
    let mut writer = ZipWriter::new(File::create(destination)?);
    let mut bytes = 0u64;
    for name in [
        "manifest.toml",
        "policy.toml",
        "v1.bin",
        "v2.bin",
        "v1.json",
        "v2.json",
        "snapshot.json",
        "expected.json",
        "case.json",
    ] {
        let path = directory.join(name);
        if name == "snapshot.json" && !path.exists() {
            continue;
        }
        let info = fs::symlink_metadata(&path)?;
        bytes = bytes
            .checked_add(info.len())
            .filter(|n| *n + 8192 < budget)
            .ok_or_else(|| {
                ServiceError::new("OUTPUT_TOO_LARGE", "Portable case exceeds remaining budget")
            })?;
        if !info.is_file() {
            return Err(ServiceError::new(
                "ENGINE_PROTOCOL_ERROR",
                "Invalid portable case file",
            ));
        }
        writer
            .start_file(name, SimpleFileOptions::default())
            .map_err(|_| ServiceError::new("STORAGE_ERROR", "Cannot encode portable case"))?;
        std::io::copy(&mut File::open(path)?, &mut writer)?;
    }
    writer
        .finish()
        .map_err(|_| ServiceError::new("STORAGE_ERROR", "Cannot finish portable case"))?;
    Ok(())
}

fn input_hashes(root: &Path) -> Result<std::collections::BTreeMap<std::path::PathBuf, String>> {
    fn visit(
        path: &Path,
        files: &mut std::collections::BTreeMap<std::path::PathBuf, String>,
    ) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            let info = fs::symlink_metadata(&path)?;
            if info.is_dir() {
                visit(&path, files)?;
            } else if info.is_file() {
                let mut file = File::open(&path)?;
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buffer[..n]);
                }
                files.insert(path, format!("{:x}", hash.finalize()));
            } else {
                return Err(ServiceError::new(
                    "INPUT_CHANGED",
                    "Sealed project contains a link or special file",
                ));
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    visit(root, &mut files)?;
    Ok(files)
}

pub fn run() -> Result<()> {
    probe()?;
    let mut output = std::io::stdout().lock();
    output.write_all(MAGIC)?;
    let prepared = (|| -> Result<_> {
        let metadata =
            dg_analysis::read_bounded(Path::new("/submission/metadata.json"), 32 * 1024)?;
        let metadata: PreparedUpload = serde_json::from_slice(&metadata)
            .map_err(|_| ServiceError::new("INVALID_CONFIG", "Invalid upload metadata"))?;
        extract(
            File::open("/submission/archive.zip")?,
            Path::new("/work/input"),
            Limits::default(),
        )?;
        preflight(Path::new("/work/input"), &metadata)
    })();
    let paths = match prepared {
        Ok(paths) => paths,
        Err(error) => {
            let report = ValidatedReport::backend_error(&error);
            frame(&mut output, PUBLISHING, &[])?;
            frame(&mut output, INPUT_ERROR, &[])?;
            frame(
                &mut output,
                ORIGINAL,
                &serde_json::to_vec(report.original()).map_err(|_| {
                    ServiceError::new("ENGINE_PROTOCOL_ERROR", "Cannot serialize input error")
                })?,
            )?;
            frame(&mut output, DONE, &[])?;
            return Ok(());
        }
    };
    // Extracted project data is sealed before executing its SBF candidate.
    fn seal(path: &Path) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let p = entry?.path();
            if p.is_dir() {
                seal(&p)?;
            } else {
                fs::set_permissions(&p, fs::Permissions::from_mode(0o444))?;
            }
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o555))?;
        Ok(())
    }
    seal(Path::new("/work/input"))?;
    let input_before = input_hashes(Path::new("/work/input"))?;
    frame(&mut output, RUNNING, &[])?;
    let mut command = Command::new(ENGINE);
    command
        .arg("test")
        .arg(paths.manifest)
        .arg("--policy")
        .arg(paths.policy)
        .args(["--output", "/work/result", "--ci", "--quiet"]);
    if let Some(snapshot) = paths.snapshot {
        command.arg("--snapshot").arg(snapshot);
    }
    let (exit, mut truncated) = child(command)?;
    if input_before != input_hashes(Path::new("/work/input"))? {
        return Err(ServiceError::new(
            "INPUT_CHANGED",
            "Execution changed sealed project data",
        ));
    }
    if exit == 4 {
        frame(&mut output, PUBLISHING, &[])?;
        frame(&mut output, ENGINE_EXIT, &[4])?;
        let report = json!({"schema_version":1,"result":"ERROR","exit_code":4,"diagnostics":[{"code":"ENGINE_INTERNAL_ERROR","message":"The pinned engine reported an internal or output failure","exit_code":4}]});
        frame(
            &mut output,
            ORIGINAL,
            &serde_json::to_vec(&report)
                .map_err(|_| ServiceError::new("ENGINE_PROTOCOL_ERROR", "Cannot encode failure"))?,
        )?;
        if truncated {
            frame(&mut output, LOG_TRUNCATED, &[])?;
        }
        frame(&mut output, DONE, &[])?;
        return Ok(());
    }
    let raw =
        dg_analysis::read_bounded(Path::new("/work/result/report.json"), OUTPUT_LIMIT as usize)?;
    let report = ValidatedReport::parse(Some(exit), &raw)?;
    frame(&mut output, PUBLISHING, &[])?;
    frame(&mut output, ENGINE_EXIT, &[exit as u8])?;
    let mut emitted = file_frame(&mut output, ORIGINAL, Path::new("/work/result/report.json"))?;
    for (tag, path) in [
        (HTML, "/work/result/report.html"),
        (SUMMARY, "/work/result/summary.md"),
    ] {
        let bytes = fs::metadata(path)?.len();
        emitted = emitted
            .checked_add(bytes)
            .filter(|n| *n <= OUTPUT_LIMIT)
            .ok_or_else(|| {
                ServiceError::new("OUTPUT_TOO_LARGE", "Combined output exceeds budget")
            })?;
        file_frame(&mut output, tag, Path::new(path))?;
    }
    if matches!(report.exit_code(), 1 | 3) && report.original()["artifacts"].is_object() {
        let mut capture = Command::new(ENGINE);
        capture.args([
            "regression",
            "capture",
            "--from-report",
            "/work/result/report.json",
            "--store",
            "/work/case",
        ]);
        let capture_result = child(capture);
        if let Ok((_, overflow)) = capture_result {
            truncated |= overflow;
        }
        let captured = capture_result.is_ok_and(|(e, _)| e == 0)
            && case_zip(
                Path::new("/work/case"),
                Path::new("/work/case.zip"),
                OUTPUT_LIMIT - emitted,
            )
            .is_ok();
        if captured {
            file_frame(&mut output, CASE, Path::new("/work/case.zip"))?;
        } else {
            frame(&mut output, CASE_UNAVAILABLE, &[])?;
        }
    }
    if input_before != input_hashes(Path::new("/work/input"))? {
        return Err(ServiceError::new(
            "INPUT_CHANGED",
            "Capture changed sealed project data",
        ));
    }
    if truncated {
        frame(&mut output, LOG_TRUNCATED, &[])?;
    }
    frame(&mut output, DONE, &[])?;
    Ok(())
}
