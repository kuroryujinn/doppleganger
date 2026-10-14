//! Raw reception is host-safe; archive extraction/configuration parsing belongs in a sandbox.
use crate::{
    model::{relative_path, PreparedUpload},
    Result, ServiceError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive};

#[derive(Clone, Copy)]
pub struct Limits {
    pub upload_bytes: u64,
    pub extracted_bytes: u64,
    pub entry_bytes: u64,
    pub entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            upload_bytes: 384 * 1024 * 1024,
            extracted_bytes: 512 * 1024 * 1024,
            entry_bytes: 128 * 1024 * 1024,
            entries: 10_000,
        }
    }
}

#[derive(Debug)]
pub struct ReceivedArchive {
    pub(crate) id: String,
    pub(crate) sha256: String,
    pub(crate) bytes: i64,
    pub(crate) storage_key: String,
    path: PathBuf,
}
impl ReceivedArchive {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn bytes(&self) -> i64 {
        self.bytes
    }
}

fn private_directory(path: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

/// Incremental raw reception shared by local tooling and HTTP. No ZIP parsing on the host.
pub struct UploadWriter {
    directory: PathBuf,
    file: File,
    hash: Sha256,
    bytes: u64,
    limit: u64,
    committed: bool,
}
impl UploadWriter {
    pub fn new(root: &Path, limits: Limits) -> Result<Self> {
        if !root.is_absolute() || !fs::symlink_metadata(root).is_ok_and(|m| m.is_dir()) {
            return Err(ServiceError::new(
                "INVALID_STORAGE",
                "Expected a private absolute storage directory",
            ));
        }
        let directory = root.join(format!("sub_{}", Uuid::new_v4().simple()));
        private_directory(&directory)?;
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("archive.part"))
        {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(error.into());
            }
        };
        Ok(Self {
            directory,
            file,
            hash: Sha256::new(),
            bytes: 0,
            limit: limits.upload_bytes,
            committed: false,
        })
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .filter(|&n| n <= self.limit)
            .ok_or_else(|| ServiceError::new("INPUT_TOO_LARGE", "Upload exceeds budget"))?;
        self.file.write_all(bytes)?;
        self.hash.update(bytes);
        Ok(())
    }
    pub fn finish(mut self) -> Result<ReceivedArchive> {
        if self.bytes == 0 {
            return Err(ServiceError::new("INVALID_ARCHIVE", "Empty upload"));
        }
        self.file.sync_all()?;
        let path = self.directory.join("archive.zip");
        fs::rename(self.directory.join("archive.part"), &path)?;
        File::open(&self.directory)?.sync_all()?;
        let root = self
            .directory
            .parent()
            .ok_or_else(|| ServiceError::new("INVALID_STORAGE", "Missing upload root"))?;
        File::open(root)?.sync_all()?;
        let id = self
            .directory
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ServiceError::new("INVALID_STORAGE", "Missing submission identity"))?
            .to_owned();
        self.committed = true;
        Ok(ReceivedArchive {
            id: id.clone(),
            storage_key: format!("{id}/archive.zip"),
            sha256: format!("{:x}", self.hash.clone().finalize()),
            bytes: self.bytes as i64,
            path,
        })
    }
}
impl Drop for UploadWriter {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
// shortcut: hard crashes can leave unregistered receipts; add orphan cleanup with B4 retention.
/// Register only after finish; orphan published receipts remain recoverable cleanup candidates.
pub fn receive(root: &Path, mut source: impl Read, limits: Limits) -> Result<ReceivedArchive> {
    let mut writer = UploadWriter::new(root, limits)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write(&buffer[..n])?;
    }
    writer.finish()
}

/// Use only in the isolated job. The destination must be fresh and privately owned.
pub fn extract(mut source: impl Read + Seek, destination: &Path, limits: Limits) -> Result<()> {
    let expected_entries = zip32_entries(&mut source, limits)?;
    let mut archive = ZipArchive::new(source)
        .map_err(|_| ServiceError::new("INVALID_ARCHIVE", "ZIP directory is invalid"))?;
    if archive.len() != expected_entries {
        return Err(ServiceError::new(
            "INVALID_ARCHIVE",
            "ZIP directory count mismatch or duplicate file names",
        ));
    }
    private_directory(destination)?;
    let result = (|| {
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|_| {
                ServiceError::new("INVALID_ARCHIVE", "Unsupported or malformed ZIP entry")
            })?;
            let raw = std::str::from_utf8(entry.name_raw())
                .map_err(|_| ServiceError::new("INVALID_PATH", "Archive paths must be UTF-8"))?;
            let name = if entry.is_dir() {
                raw.strip_suffix('/').unwrap_or(raw)
            } else {
                raw
            };
            let relative = relative_path(name)?.to_owned();
            if !names.insert(relative.clone()) {
                return Err(ServiceError::new(
                    "INVALID_ARCHIVE",
                    "Duplicate archive path",
                ));
            }
            let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
            if entry.encrypted()
                || !matches!(
                    entry.compression(),
                    CompressionMethod::Stored | CompressionMethod::Deflated
                )
                || !(kind == 0
                    || (entry.is_dir() && kind == 0o040000)
                    || (!entry.is_dir() && kind == 0o100000))
            {
                return Err(ServiceError::new(
                    "INVALID_ARCHIVE",
                    "Links, special files, encryption or compression are unsupported",
                ));
            }
            if entry.size() > limits.entry_bytes {
                return Err(ServiceError::new(
                    "INPUT_TOO_LARGE",
                    "Archive entry exceeds budget",
                ));
            }
            let path = destination.join(relative);
            if entry.is_dir() {
                if entry.size() != 0 {
                    return Err(ServiceError::new(
                        "INVALID_ARCHIVE",
                        "Directory entry contains data",
                    ));
                }
                fs::create_dir_all(&path)?;
                continue;
            }
            fs::create_dir_all(
                path.parent()
                    .ok_or_else(|| ServiceError::new("INVALID_PATH", "No file parent"))?,
            )?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            let mut bytes = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = entry.read(&mut buffer).map_err(|_| {
                    ServiceError::new(
                        "INVALID_ARCHIVE",
                        "ZIP entry is truncated or checksum failed",
                    )
                })?;
                if count == 0 {
                    break;
                }
                bytes = bytes.checked_add(count as u64).ok_or_else(|| {
                    ServiceError::new("INPUT_TOO_LARGE", "Archive entry exceeds budget")
                })?;
                total = total.checked_add(count as u64).ok_or_else(|| {
                    ServiceError::new("INPUT_TOO_LARGE", "Archive exceeds extraction budget")
                })?;
                if bytes > limits.entry_bytes || total > limits.extracted_bytes {
                    return Err(ServiceError::new(
                        "INPUT_TOO_LARGE",
                        "Archive exceeds extraction budget",
                    ));
                }
                output.write_all(&buffer[..count])?;
            }
            if bytes != entry.size() {
                return Err(ServiceError::new("INVALID_ARCHIVE", "ZIP size mismatch"));
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

/// ZIP's reader deduplicates identical names. Validate the declared count before
/// allocation and compare afterwards so duplicate central-directory entries cannot hide.
/// ZIP64/multi-volume archives are deliberately outside the bounded upload format.
fn zip32_entries(source: &mut (impl Read + Seek), limits: Limits) -> Result<usize> {
    let length = source.seek(SeekFrom::End(0))?;
    if length > limits.upload_bytes {
        return Err(ServiceError::new(
            "INPUT_TOO_LARGE",
            "Archive exceeds upload budget",
        ));
    }
    let tail_bytes = length.min(65_557);
    source.seek(SeekFrom::Start(length - tail_bytes))?;
    let mut tail = vec![0; tail_bytes as usize];
    source.read_exact(&mut tail)?;
    let footer = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&index| {
            tail[index..].starts_with(b"PK\x05\x06")
                && index + 22 + u16::from_le_bytes([tail[index + 20], tail[index + 21]]) as usize
                    == tail.len()
        })
        .ok_or_else(|| ServiceError::new("INVALID_ARCHIVE", "Missing bounded ZIP32 footer"))?;
    let u16_at =
        |offset: usize| u16::from_le_bytes([tail[footer + offset], tail[footer + offset + 1]]);
    let u32_at = |offset: usize| {
        u32::from_le_bytes(
            tail[footer + offset..footer + offset + 4]
                .try_into()
                .expect("fixed footer slice"),
        )
    };
    let count = u16_at(10) as usize;
    let size = u32_at(12);
    let start = u32_at(16);
    let footer_offset = length - tail_bytes + footer as u64;
    if u16_at(4) != 0
        || u16_at(6) != 0
        || u16_at(8) as usize != count
        || count == u16::MAX as usize
        || size == u32::MAX
        || start == u32::MAX
        || u64::from(start) + u64::from(size) != footer_offset
    {
        return Err(ServiceError::new(
            "INVALID_ARCHIVE",
            "ZIP64, multi-volume or inconsistent ZIP directory is unsupported",
        ));
    }
    if count > limits.entries {
        return Err(ServiceError::new(
            "INPUT_TOO_LARGE",
            "Too many archive entries",
        ));
    }
    source.rewind()?;
    Ok(count)
}

pub struct PreparedPaths {
    pub manifest: PathBuf,
    pub policy: PathBuf,
    pub snapshot: Option<PathBuf>,
}

fn contained_file(root: &Path, base: &Path, relative: &Path) -> Result<PathBuf> {
    relative_path(
        relative
            .to_str()
            .ok_or_else(|| ServiceError::new("INVALID_PATH", "Non-UTF-8 artifact path"))?,
    )?;
    let path = base.join(relative);
    // Check every component before following it. Archive extraction cannot create links;
    // this also protects callers that use an existing prepared directory.
    let mut walked = root.to_owned();
    let suffix = path
        .strip_prefix(root)
        .map_err(|_| ServiceError::new("INVALID_PATH", "Artifact escapes project"))?;
    for component in suffix.components() {
        walked.push(component);
        if fs::symlink_metadata(&walked)?.file_type().is_symlink() {
            return Err(ServiceError::new(
                "INVALID_PATH",
                "Artifact links are unsupported",
            ));
        }
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(ServiceError::new(
            "INVALID_PATH",
            "Artifact must be a contained regular file",
        ));
    }
    Ok(canonical)
}

/// Validate all paths before the existing local loader reads any artifact.
/// Run in the sandbox; this performs bounded parsing of untrusted configuration.
pub fn preflight(root: &Path, upload: &PreparedUpload) -> Result<PreparedPaths> {
    upload.validate()?;
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(ServiceError::new(
            "INVALID_PATH",
            "Project root must be a directory",
        ));
    }
    let root = root.canonicalize()?;
    let manifest = contained_file(&root, &root, Path::new(&upload.manifest_path))?;
    let policy = contained_file(&root, &root, Path::new(&upload.policy_path))?;
    let source = dg_analysis::read_bounded(&manifest, 4 * 1024 * 1024)
        .map_err(|_| ServiceError::new("INVALID_CONFIG", "Cannot read bounded manifest"))?;
    let text = std::str::from_utf8(&source)
        .map_err(|_| ServiceError::new("INVALID_CONFIG", "Manifest is not UTF-8"))?;
    let parsed = dg_config::ProjectManifest::parse(text).map_err(|_| {
        ServiceError::new(
            "INVALID_CONFIG",
            "Invalid migration manifest; validate it with the CLI",
        )
    })?;
    if parsed.schema_version != Some(1) {
        return Err(ServiceError::new(
            "UNSUPPORTED_INPUT",
            "Service requires manifest schema 1",
        ));
    }
    let base = manifest
        .parent()
        .ok_or_else(|| ServiceError::new("INVALID_PATH", "Manifest parent unavailable"))?;
    for version in [&parsed.versions.v1, &parsed.versions.v2] {
        contained_file(&root, base, &version.program_path)?;
        contained_file(&root, base, &version.idl)?;
    }
    if let Some(snapshot) = &parsed.snapshot {
        for program in &snapshot.programs {
            contained_file(&root, base, &program.program_path)?;
        }
    }
    let policy_bytes = dg_analysis::read_bounded(&policy, 4 * 1024 * 1024)
        .map_err(|_| ServiceError::new("INVALID_CONFIG", "Cannot read bounded policy"))?;
    dg_policy::Profile::parse(
        std::str::from_utf8(&policy_bytes)
            .map_err(|_| ServiceError::new("INVALID_CONFIG", "Policy is not UTF-8"))?,
    )
    .map_err(|_| {
        ServiceError::new(
            "INVALID_CONFIG",
            "Invalid migration policy; validate it with the CLI",
        )
    })?;
    let snapshot = upload
        .snapshot_path
        .as_ref()
        .map(|p| contained_file(&root, &root, Path::new(p)))
        .transpose()?;
    Ok(PreparedPaths {
        manifest,
        policy,
        snapshot,
    })
}
