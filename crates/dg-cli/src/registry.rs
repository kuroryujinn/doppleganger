//! Versioned local registry. Every read-modify-write operation holds the same OS lock.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
const ROOT: &str = ".doppelganger";
const FILE: &str = ".doppelganger/projects.json";
const BACKUP: &str = ".doppelganger/projects.json.bak";
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    // Absence identifies the original, frozen map-only registry format.
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub projects: BTreeMap<String, PathBuf>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            schema_version: Some(1),
            projects: BTreeMap::new(),
        }
    }
}
fn reject_symlink(path: &Path) -> Result<(), Box<dyn Error>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(format!("registry path must not be a symlink: {}", path.display()).into())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn lock() -> Result<File, Box<dyn Error>> {
    reject_symlink(Path::new(ROOT))?;
    fs::create_dir_all(ROOT)?;
    let path = Path::new(".doppelganger/projects.lock");
    reject_symlink(path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    lock.lock()?;
    for path in [FILE, BACKUP] {
        reject_symlink(Path::new(path))?;
    }
    Ok(lock)
}
fn decode(path: &str) -> Result<Registry, Box<dyn Error>> {
    let bytes = dg_analysis::read_bounded(Path::new(path), 4 * 1024 * 1024)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("CONFIG: corrupt registry {path}: {e}. Use doppelganger project recover to restore its backup."))?;
    if value
        .get("schema_version")
        .is_some_and(|v| v != &serde_json::json!(1))
    {
        return Err("CONFIG: unsupported registry schema version; supported: 1".into());
    }
    let registry: Registry = serde_json::from_value(value)?;
    if registry.projects.len() > 10_000 {
        return Err("CONFIG: registry exceeds 10,000 projects".into());
    }
    for (name, path) in &registry.projects {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !path.is_absolute()
        {
            return Err(
                "CONFIG: registry contains an invalid project name or nonabsolute path".into(),
            );
        }
    }
    Ok(registry)
}
fn read() -> Result<Registry, Box<dyn Error>> {
    if Path::new(FILE).exists() {
        decode(FILE)
    } else {
        Ok(Registry::default())
    }
}
fn replace(path: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let temp = format!("{path}.{}.tmp", std::process::id());
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(ROOT)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn load() -> Result<Registry, Box<dyn Error>> {
    let _lock = lock()?;
    read()
}
pub fn add(name: String, path: PathBuf) -> Result<(), Box<dyn Error>> {
    let _lock = lock()?;
    let mut registry = read()?;
    if registry.projects.get(&name).is_some_and(|old| old != &path) {
        return Err(format!("project '{name}' is already registered from another manifest").into());
    }
    if registry
        .projects
        .iter()
        .any(|(id, old)| id != &name && old == &path)
    {
        return Err("manifest is already registered under another name".into());
    }
    if registry.projects.len() >= 10_000 && !registry.projects.contains_key(&name) {
        return Err("registry exceeds 10,000 projects".into());
    }
    // Preserve the last validated state; corruption is never silently overwritten.
    if Path::new(FILE).exists() {
        replace(
            BACKUP,
            &dg_analysis::read_bounded(Path::new(FILE), 4 * 1024 * 1024)?,
        )?;
    }
    registry.schema_version = Some(1);
    registry.projects.insert(name, path);
    replace(FILE, &serde_json::to_vec_pretty(&registry)?)
}
pub fn recover() -> Result<(), Box<dyn Error>> {
    let _lock = lock()?;
    let mut registry = decode(BACKUP)?;
    registry.schema_version = Some(1);
    // Recovery is explicit. Keep the damaged bytes for diagnosis.
    if Path::new(FILE).exists() {
        let damaged = format!("{FILE}.corrupt.{}", std::process::id());
        fs::copy(FILE, damaged)?;
    }
    replace(FILE, &serde_json::to_vec_pretty(&registry)?)
}
