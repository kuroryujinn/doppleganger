//! Bounded read-only acquisition and offline import into the existing runtime.
pub mod replay;
pub mod rpc;

use dg_config::{snapshot::SnapshotConfig, AddressDefinition, ProjectManifest, ResolvedAddresses};
use dg_state::{raw::RawAccountSnapshot, DecodeLimits, DecodedAccount, StateError, TrailingBytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_address::Address;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const DEVNET_GENESIS: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
pub const MAX_ACCOUNTS: usize = 100;
pub const MAX_ACCOUNT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, SnapshotError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotError {
    pub code: String,
    pub context: String,
    pub message: String,
}
impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.context, self.message)
    }
}
impl std::error::Error for SnapshotError {}
pub(crate) fn err(code: &str, context: impl ToString, message: impl ToString) -> SnapshotError {
    SnapshotError {
        code: code.into(),
        context: context.to_string(),
        message: message.to_string(),
    }
}
pub fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub(crate) fn json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value)
        .map_err(|_| err("SNAPSHOT_SCHEMA_INVALID", "json", "serialization failed"))
}
pub(crate) fn address(value: &str) -> Result<Address> {
    value
        .parse()
        .map_err(|_| err("RPC_RESPONSE_INVALID", "address", "invalid public key"))
}
pub fn load_project(path: &Path) -> Result<(ProjectManifest, PathBuf)> {
    let path = if path.is_dir() {
        path.join("doppelganger.toml")
    } else {
        path.to_owned()
    };
    let text = dg_analysis::read_bounded(&path, 4 * 1024 * 1024)
        .and_then(|b| String::from_utf8(b).map_err(std::io::Error::other))
        .map_err(|_| err("CONFIG", "manifest", "cannot read project manifest"))?;
    let m = ProjectManifest::parse(&text)
        .map_err(|_| err("CONFIG", "manifest", "invalid project manifest"))?;
    Ok((m, path.parent().unwrap_or(Path::new(".")).to_owned()))
}
pub(crate) fn config(m: &ProjectManifest) -> Result<&SnapshotConfig> {
    let c = m
        .snapshot
        .as_ref()
        .ok_or_else(|| err("CONFIG", "snapshot", "snapshot configuration is required"))?;
    if c.accounts.is_empty()
        || c.accounts.len() > MAX_ACCOUNTS
        || c.max_account_bytes == 0
        || c.max_account_bytes > MAX_ACCOUNT_BYTES
        || c.programs.len() > MAX_ACCOUNTS
    {
        return Err(err(
            "ACCOUNT_DATA_TOO_LARGE",
            "snapshot",
            "account count or byte limit out of range",
        ));
    }
    let mut names = BTreeSet::new();
    for a in &c.accounts {
        if a.name.is_empty()
            || !a
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || !names.insert(&a.name)
        {
            return Err(err(
                "CONFIG",
                "snapshot.accounts",
                "empty, invalid or duplicate account name",
            ));
        }
    }
    Ok(c)
}
pub(crate) fn resolve(m: &ProjectManifest) -> Result<ResolvedAddresses> {
    for (name, a) in &m.addresses {
        if matches!(
            a,
            AddressDefinition::Signer {
                test_seed: None,
                ..
            }
        ) {
            return Err(err(
                "MISSING_LOCAL_SIGNER",
                name,
                "snapshot replay requires explicit public deterministic test seeds",
            ));
        }
    }
    ResolvedAddresses::resolve(
        &m.addresses,
        &BTreeMap::from([
            ("v1".into(), address(&m.versions.v1.program_id)?),
            ("v2".into(), address(&m.versions.v2.program_id)?),
        ]),
    )
    .map_err(|_| err("CONFIG", "addresses", "invalid address configuration"))
}
pub(crate) fn binding(source: &str, resolved: &ResolvedAddresses) -> Result<Address> {
    if let Some(name) = source.strip_prefix('$') {
        resolved
            .addresses
            .get(name)
            .copied()
            .ok_or_else(|| err("CONFIG", name, "unknown address binding"))
    } else {
        address(source)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Acquisition {
    Rpc,
    MockRpc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodeStatus {
    NotRequested,
    Decoded,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub path: PathBuf,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub cluster: String,
    pub genesis_hash: String,
    pub rpc_origin: String,
    pub acquisition: Acquisition,
    pub slot: u64,
    pub min_context_slot: Option<u64>,
    pub commitment: String,
    pub captured_at_unix: u64,
    pub non_atomic_capture: bool,
    pub program_id: String,
    pub tool_version: String,
    pub artifacts: BTreeMap<String, ArtifactReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedAccount {
    pub logical_name: String,
    pub address: String,
    pub exists: bool,
    pub raw: Option<RawAccountSnapshot>,
    pub rent_epoch: Option<String>,
    pub decoded: Option<DecodedAccount>,
    pub decode_error: Option<StateError>,
    pub decode_status: DecodeStatus,
    pub data_sha256: Option<String>,
    pub record_sha256: String,
}
impl CapturedAccount {
    fn record_hash(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.record_sha256.clear();
        Ok(digest(&json(&copy)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevnetSnapshot {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub snapshot_sha256: String,
    pub provenance: Provenance,
    pub accounts: Vec<CapturedAccount>,
}
impl DevnetSnapshot {
    fn content_hash(&self, identity: bool) -> Result<String> {
        let mut copy = self.clone();
        copy.snapshot_id.clear();
        copy.snapshot_sha256.clear();
        if identity {
            copy.provenance.captured_at_unix = 0;
        }
        Ok(digest(&json(&copy)?))
    }
    pub fn verify(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(err(
                "SNAPSHOT_SCHEMA_UNSUPPORTED",
                "schema_version",
                "only snapshot schema v1 is supported",
            ));
        }
        if self.accounts.is_empty() || self.accounts.len() > MAX_ACCOUNTS {
            return Err(err(
                "ACCOUNT_DATA_TOO_LARGE",
                "accounts",
                "account count exceeds limit",
            ));
        }
        let mut names = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        let mut total = 0usize;
        for a in &self.accounts {
            if let Some(raw) = &a.raw {
                total = total.saturating_add(raw.raw_data.len());
                if raw.raw_data.len() > MAX_ACCOUNT_BYTES || total > MAX_TOTAL_BYTES {
                    return Err(err(
                        "ACCOUNT_DATA_TOO_LARGE",
                        &a.logical_name,
                        "snapshot byte limit exceeded",
                    ));
                }
            }
            if a.record_sha256 != a.record_hash()?
                || a.data_sha256 != a.raw.as_ref().map(|r| digest(&r.raw_data))
            {
                return Err(err(
                    "ACCOUNT_HASH_MISMATCH",
                    &a.logical_name,
                    "account data or metadata changed",
                ));
            }
            if !names.insert(&a.logical_name)
                || !addresses.insert(&a.address)
                || a.exists != a.raw.is_some()
                || a.exists != a.rent_epoch.is_some()
            {
                return Err(err(
                    "RPC_RESPONSE_INVALID",
                    &a.logical_name,
                    "inconsistent or duplicate account record",
                ));
            }
            address(&a.address)?;
            let status = match (&a.decoded, &a.decode_error) {
                (None, None) => DecodeStatus::NotRequested,
                (Some(_), None) => DecodeStatus::Decoded,
                (None, Some(_)) => DecodeStatus::Error,
                _ => {
                    return Err(err(
                        "RPC_RESPONSE_INVALID",
                        &a.logical_name,
                        "conflicting decode evidence",
                    ))
                }
            };
            if status != a.decode_status {
                return Err(err(
                    "RPC_RESPONSE_INVALID",
                    &a.logical_name,
                    "invalid decode status",
                ));
            }
            if let Some(raw) = &a.raw {
                raw.validate(DecodeLimits {
                    max_bytes: MAX_ACCOUNT_BYTES,
                    ..DecodeLimits::default()
                })
                .map_err(|_| {
                    err(
                        "RPC_RESPONSE_INVALID",
                        &a.logical_name,
                        "invalid raw account record",
                    )
                })?;
                if raw.address != a.address
                    || a.rent_epoch
                        .as_ref()
                        .and_then(|s| s.parse::<u64>().ok())
                        .is_none()
                {
                    return Err(err(
                        "RPC_RESPONSE_INVALID",
                        &a.logical_name,
                        "invalid address or rent epoch",
                    ));
                }
            }
        }
        if self.snapshot_sha256 != self.content_hash(false)?
            || self.snapshot_id != format!("SNAP-{}", self.content_hash(true)?)
        {
            return Err(err(
                "SNAPSHOT_HASH_MISMATCH",
                "snapshot",
                "snapshot content or provenance changed",
            ));
        }
        if self.provenance.cluster != "devnet" || self.provenance.genesis_hash != DEVNET_GENESIS {
            return Err(err(
                "CLUSTER_MISMATCH",
                "provenance",
                "Devnet genesis hash is required",
            ));
        }
        address(&self.provenance.program_id)?;
        if rpc::redacted_origin(&self.provenance.rpc_origin)? != self.provenance.rpc_origin {
            return Err(err(
                "RPC_RESPONSE_INVALID",
                "rpc_origin",
                "provenance must contain only a redacted origin",
            ));
        }
        if self.provenance.non_atomic_capture
            || self
                .provenance
                .min_context_slot
                .is_some_and(|s| self.provenance.slot < s)
        {
            return Err(err(
                "SLOT_CONSISTENCY_FAILED",
                "provenance",
                "invalid capture context",
            ));
        }
        Ok(())
    }
}

pub fn capture(
    m: &ProjectManifest,
    base: &Path,
    rpc: &impl rpc::ReadOnlyRpc,
) -> Result<DevnetSnapshot> {
    let c = config(m)?;
    let resolved = resolve(m)?;
    let requested: Vec<_> = c
        .accounts
        .iter()
        .map(|a| binding(&a.address, &resolved).map(|p| p.to_string()))
        .collect::<Result<_>>()?;
    if requested.iter().collect::<BTreeSet<_>>().len() != requested.len() {
        return Err(err("CONFIG", "accounts", "duplicate addresses"));
    }
    let genesis = rpc.genesis_hash()?;
    if genesis != DEVNET_GENESIS {
        return Err(err(
            "CLUSTER_MISMATCH",
            "genesis",
            "endpoint is not Solana Devnet",
        ));
    }
    let batch = rpc.multiple_accounts(&requested, c.min_context_slot)?;
    if c.min_context_slot.is_some_and(|s| batch.slot < s) {
        return Err(err(
            "SLOT_CONSISTENCY_FAILED",
            "slot",
            "RPC returned a context older than requested",
        ));
    }
    if batch.accounts.len() != requested.len() {
        return Err(err(
            "RPC_RESPONSE_INVALID",
            "accounts",
            "RPC returned the wrong number of accounts",
        ));
    }
    let model = dg_analysis::read_bounded(&base.join(&m.versions.v1.idl), 8 * 1024 * 1024)
        .ok()
        .and_then(|b| dg_analysis::normalize(&b).ok());
    let targets = dg_runtime::MigrationExecutor::targets(m);
    let mut accounts = vec![];
    let mut total = 0usize;
    for ((def, pubkey), value) in c.accounts.iter().zip(requested).zip(batch.accounts) {
        let raw = rpc::parse_account(&pubkey, value, c.max_account_bytes)?;
        if raw.is_none() && !def.optional {
            return Err(err(
                "ACCOUNT_NOT_FOUND",
                &def.name,
                "required account is absent",
            ));
        }
        let target = targets.iter().find(|t| t.name == def.name);
        let expected_owner = match &def.owner {
            Some(o) => binding(o, &resolved)?,
            None if target.is_some() => address(&m.versions.v1.program_id)?,
            None => {
                return Err(err(
                    "CONFIG",
                    &def.name,
                    "external account requires explicit owner",
                ))
            }
        };
        let (raw, rent_epoch) = match raw {
            Some((raw, rent)) => (Some(raw), Some(rent.to_string())),
            None => (None, None),
        };
        if raw
            .as_ref()
            .is_some_and(|r| r.owner != expected_owner.to_string())
        {
            return Err(err(
                "OWNER_MISMATCH",
                &def.name,
                "observed account owner differs from configuration",
            ));
        }
        if let Some(r) = &raw {
            total = total.saturating_add(r.data_length);
            if total > MAX_TOTAL_BYTES {
                return Err(err(
                    "ACCOUNT_DATA_TOO_LARGE",
                    &def.name,
                    "total account byte limit exceeded",
                ));
            }
        }
        let ty = def
            .account_type
            .as_ref()
            .or_else(|| target.and_then(|t| t.v1_type.as_ref()));
        let (mut decoded, mut decode_error) = (None, None);
        if let (Some(raw), Some(ty)) = (
            &raw,
            ty.filter(|_| expected_owner.to_string() == m.versions.v1.program_id),
        ) {
            match model.as_ref().map(|model| {
                dg_state::decode_account(
                    model,
                    ty,
                    &raw.raw_data,
                    DecodeLimits {
                        max_bytes: c.max_account_bytes,
                        ..DecodeLimits::default()
                    },
                    TrailingBytes::Preserve,
                )
            }) {
                Some(Ok(value)) => decoded = Some(value),
                Some(Err(e)) => decode_error = Some(e),
                None => {
                    decode_error = Some(StateError {
                        code: "IDL_UNAVAILABLE".into(),
                        path: def.name.clone(),
                        offset: 0,
                        message: "V1 IDL could not be loaded".into(),
                    })
                }
            }
        }
        let decode_status = if decoded.is_some() {
            DecodeStatus::Decoded
        } else if decode_error.is_some() {
            DecodeStatus::Error
        } else {
            DecodeStatus::NotRequested
        };
        let mut account = CapturedAccount {
            logical_name: def.name.clone(),
            address: pubkey,
            exists: raw.is_some(),
            data_sha256: raw.as_ref().map(|r| digest(&r.raw_data)),
            raw,
            rent_epoch,
            decoded,
            decode_error,
            decode_status,
            record_sha256: String::new(),
        };
        account.record_sha256 = account.record_hash()?;
        accounts.push(account);
    }
    accounts.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));
    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("v1_binary", &m.versions.v1.program_path),
        ("v1_idl", &m.versions.v1.idl),
    ] {
        artifacts.insert(
            name.into(),
            ArtifactReference {
                sha256: dg_analysis::read_bounded(&base.join(path), 8 * 1024 * 1024)
                    .ok()
                    .map(|b| digest(&b)),
                path: path.clone(),
            },
        );
    }
    for p in &c.programs {
        artifacts.insert(
            format!("external:{}", p.program_id),
            ArtifactReference {
                sha256: dg_analysis::read_bounded(&base.join(&p.program_path), 64 * 1024 * 1024)
                    .ok()
                    .map(|b| digest(&b)),
                path: p.program_path.clone(),
            },
        );
    }
    let mut snapshot = DevnetSnapshot {
        schema_version: 1,
        snapshot_id: String::new(),
        snapshot_sha256: String::new(),
        provenance: Provenance {
            cluster: "devnet".into(),
            genesis_hash: genesis,
            rpc_origin: rpc::redacted_origin(&rpc.origin())?,
            acquisition: rpc.acquisition(),
            slot: batch.slot,
            min_context_slot: c.min_context_slot,
            commitment: "finalized".into(),
            captured_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| err("CONFIG", "clock", "clock predates epoch"))?
                .as_secs(),
            non_atomic_capture: false,
            program_id: m.versions.v1.program_id.clone(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            artifacts,
        },
        accounts,
    };
    snapshot.snapshot_id = format!("SNAP-{}", snapshot.content_hash(true)?);
    snapshot.snapshot_sha256 = snapshot.content_hash(false)?;
    snapshot.verify()?;
    Ok(snapshot)
}

pub fn load(path: &Path) -> Result<DevnetSnapshot> {
    let path = if path.is_dir() {
        path.join("snapshot.json")
    } else {
        path.to_owned()
    };
    let file = fs::File::open(&path)
        .map_err(|_| err("SNAPSHOT_NOT_FOUND", "snapshot", "cannot open snapshot"))?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            err(
                "SNAPSHOT_SCHEMA_INVALID",
                "snapshot",
                "cannot read snapshot",
            )
        })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(err(
            "ACCOUNT_DATA_TOO_LARGE",
            "snapshot",
            "snapshot file exceeds limit",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| err("SNAPSHOT_SCHEMA_INVALID", "snapshot", "invalid JSON"))?;
    if value.get("schema_version") != Some(&serde_json::json!(1)) {
        return Err(err(
            "SNAPSHOT_SCHEMA_UNSUPPORTED",
            "schema_version",
            "only snapshot schema v1 is supported",
        ));
    }
    let snapshot: DevnetSnapshot = serde_json::from_value(value).map_err(|_| {
        err(
            "SNAPSHOT_SCHEMA_INVALID",
            "snapshot",
            "invalid snapshot shape",
        )
    })?;
    snapshot.verify()?;
    Ok(snapshot)
}
pub fn save(snapshot: &DevnetSnapshot, directory: &Path) -> Result<PathBuf> {
    snapshot.verify()?;
    fs::create_dir_all(directory).map_err(|_| {
        err(
            "SNAPSHOT_WRITE",
            "directory",
            "cannot create snapshot directory",
        )
    })?;
    let path = directory.join("snapshot.json");
    if path.exists() {
        let old = load(&path)?;
        if old.snapshot_id != snapshot.snapshot_id {
            return Err(err(
                "SNAPSHOT_EXISTS",
                "snapshot",
                "refusing to overwrite historical state",
            ));
        }
        return Ok(path);
    }
    let data = json(snapshot)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut f| f.write_all(&data))
        .map_err(|_| {
            err(
                "SNAPSHOT_WRITE",
                "snapshot",
                "cannot create immutable snapshot",
            )
        })?;
    Ok(path)
}
