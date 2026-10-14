//! Fixed-name, bounded output frames. No host archive extraction or container paths.
use crate::{model::JobState, Result, ServiceError};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

pub const MAGIC: &[u8; 8] = b"DGJOB001";
pub const RUNNING: u8 = 1;
pub const PUBLISHING: u8 = 2;
pub const ENGINE_EXIT: u8 = 3;
pub const INPUT_ERROR: u8 = 4;
pub const CASE_UNAVAILABLE: u8 = 5;
pub const LOG_TRUNCATED: u8 = 6;
pub const ORIGINAL: u8 = 16;
pub const HTML: u8 = 17;
pub const SUMMARY: u8 = 18;
pub const CASE: u8 = 19;
pub const DONE: u8 = 255;
pub const OUTPUT_LIMIT: u64 = 128 * 1024 * 1024;

pub fn frame(writer: &mut impl Write, tag: u8, body: &[u8]) -> Result<()> {
    writer.write_all(&[tag])?;
    writer.write_all(&(body.len() as u64).to_be_bytes())?;
    writer.write_all(body)?;
    writer.flush()?;
    Ok(())
}
pub fn file_frame(writer: &mut impl Write, tag: u8, path: &Path) -> Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > OUTPUT_LIMIT {
        return Err(bad());
    }
    writer.write_all(&[tag])?;
    writer.write_all(&metadata.len().to_be_bytes())?;
    let copied = std::io::copy(&mut File::open(path)?.take(metadata.len() + 1), writer)?;
    if copied != metadata.len() {
        return Err(bad());
    }
    writer.flush()?;
    Ok(copied)
}
pub fn filename(tag: u8) -> Option<&'static str> {
    match tag {
        ORIGINAL => Some("original.json"),
        HTML => Some("report.html"),
        SUMMARY => Some("summary.md"),
        CASE => Some("portable_case.zip"),
        _ => None,
    }
}
fn bad() -> ServiceError {
    ServiceError::new("ENGINE_PROTOCOL_ERROR", "Malformed sandbox output stream")
}

#[derive(Debug)]
pub struct Collected {
    pub engine_exit: Option<i32>,
    pub reported_exit: i32,
    pub portable_unavailable: bool,
    pub logs_truncated: bool,
}

pub async fn collect(
    mut reader: impl AsyncRead + Unpin,
    directory: &Path,
    stages: mpsc::Sender<JobState>,
) -> Result<Collected> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).await.map_err(|_| bad())?;
    if &magic != MAGIC {
        return Err(bad());
    }
    let mut stage = JobState::Validating;
    let mut invoked = false;
    let mut outcome: Option<(Option<i32>, i32)> = None;
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut unavailable = false;
    let mut logs_truncated = false;
    loop {
        let tag = reader.read_u8().await.map_err(|_| bad())?;
        let bytes = reader.read_u64().await.map_err(|_| bad())?;
        match tag {
            RUNNING | PUBLISHING if bytes == 0 => {
                let next = if tag == RUNNING {
                    JobState::Running
                } else {
                    JobState::Publishing
                };
                if !matches!(
                    (stage, next),
                    (JobState::Validating, JobState::Running)
                        | (JobState::Validating, JobState::Publishing)
                        | (JobState::Running, JobState::Publishing)
                ) {
                    return Err(bad());
                }
                stage = next;
                invoked |= next == JobState::Running;
                stages.send(next).await.map_err(|_| bad())?;
            }
            ENGINE_EXIT
                if bytes == 1 && stage == JobState::Publishing && outcome.is_none() && invoked =>
            {
                let exit = reader.read_u8().await.map_err(|_| bad())?;
                if exit > 4 {
                    return Err(bad());
                }
                outcome = Some((Some(exit as i32), exit as i32));
            }
            INPUT_ERROR
                if bytes == 0 && stage == JobState::Publishing && outcome.is_none() && !invoked =>
            {
                outcome = Some((None, 2))
            }
            CASE_UNAVAILABLE
                if bytes == 0
                    && stage == JobState::Publishing
                    && !unavailable
                    && matches!(outcome, Some((Some(1 | 3), _))) =>
            {
                unavailable = true
            }
            LOG_TRUNCATED if bytes == 0 && stage == JobState::Publishing && !logs_truncated => {
                logs_truncated = true
            }
            DONE if bytes == 0 => {
                let (engine_exit, reported_exit) = outcome.ok_or_else(bad)?;
                if !seen.contains(&ORIGINAL)
                    || (unavailable && seen.contains(&CASE))
                    || (matches!(engine_exit, Some(0 | 1))
                        && !(seen.contains(&HTML) && seen.contains(&SUMMARY)))
                    || (seen.contains(&CASE) && !matches!(engine_exit, Some(1 | 3)))
                {
                    return Err(bad());
                }
                let mut trailing = [0u8; 1];
                if reader.read(&mut trailing).await? != 0 {
                    return Err(bad());
                }
                drop(stages);
                return Ok(Collected {
                    engine_exit,
                    reported_exit,
                    portable_unavailable: unavailable,
                    logs_truncated,
                });
            }
            tag if filename(tag).is_some() && outcome.is_some() => {
                if !seen.insert(tag) {
                    return Err(bad());
                }
                total = total
                    .checked_add(bytes)
                    .filter(|n| *n <= OUTPUT_LIMIT)
                    .ok_or_else(|| {
                        ServiceError::new(
                            "OUTPUT_TOO_LARGE",
                            "Sandbox output exceeds publication budget",
                        )
                    })?;
                let path = directory.join(filename(tag).ok_or_else(bad)?);
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)?;
                let mut file = tokio::fs::File::from_std(file);
                let mut remaining = bytes;
                let mut buffer = vec![0u8; 64 * 1024];
                while remaining > 0 {
                    let limit = remaining.min(buffer.len() as u64) as usize;
                    let n = reader.read(&mut buffer[..limit]).await.map_err(|_| bad())?;
                    if n == 0 {
                        return Err(bad());
                    }
                    file.write_all(&buffer[..n]).await?;
                    remaining -= n as u64;
                }
                file.sync_all().await?;
            }
            _ => return Err(bad()),
        }
    }
}
