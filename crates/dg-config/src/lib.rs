use dg_analysis::{
    model::{FieldType, ProgramModel, TypeModel},
    normalize,
};
use num_bigint::BigInt;
pub mod addresses;
pub mod fixture;
pub mod lifecycle;
pub mod preservation;
pub mod snapshot;
pub use addresses::{AddressDefinition, ResolvedAddresses};
use serde::{Deserialize, Serialize};
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug)]
pub struct ConfigError {
    pub field: String,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}
impl Error for ConfigError {}

type Result<T> = std::result::Result<T, ConfigError>;
fn fail(field: impl Into<String>, message: impl Into<String>) -> ConfigError {
    ConfigError {
        field: field.into(),
        message: message.into(),
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    /// None identifies the frozen pre-versioned format; it is never reinterpreted.
    pub schema_version: Option<u32>,
    pub snapshot: Option<snapshot::SnapshotConfig>,
    pub project: Project,
    pub versions: Versions,
    pub migration: MigrationDefinition,
    pub fixture: Option<FixtureDefinition>,
    #[serde(default)]
    pub addresses: BTreeMap<String, AddressDefinition>,
    #[serde(default)]
    pub migration_targets: Vec<MigrationTarget>,
    #[serde(default)]
    pub preservation: Vec<preservation::PreservationRule>,
    #[serde(default)]
    pub scenario: Vec<lifecycle::LifecycleScenario>,
    #[serde(default)]
    pub fixtures: BTreeMap<String, FixtureDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationTarget {
    pub name: String,
    pub address: String,
    pub v1_type: Option<String>,
    pub v2_type: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureDefinition {
    pub name: String,
    #[serde(default)]
    pub payer: Option<String>,
    #[serde(default)]
    pub account: Vec<FixtureAccount>,
    #[serde(default)]
    pub initialize: Vec<MigrationDefinition>,
    #[serde(default)]
    pub setup: Vec<MigrationDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureAccount {
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    /// Base58 address or a `$binding` resolved by the caller.
    pub address: String,
    pub owner: String,
    pub lamports: toml::Value,
    #[serde(default)]
    pub executable: bool,
    /// Omit fields for an account created by initialization instructions.
    pub fields: Option<toml::Table>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub framework: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
    pub v1: ProgramVersion,
    pub v2: ProgramVersion,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramVersion {
    pub program_path: PathBuf,
    pub idl: PathBuf,
    pub program_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationDefinition {
    pub instruction: String,
    pub account_mapping: BTreeMap<String, String>,
    #[serde(default, alias = "args")]
    pub arguments: BTreeMap<String, toml::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationAccount {
    pub idl_name: String,
    pub source: String,
    pub signer: bool,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct MigrationArgument {
    pub name: String,
    pub ty: FieldType,
    pub encoded: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct InstructionBinding {
    pub program_id: Address,
    pub instruction: String,
    pub discriminator: Vec<u8>,
    pub accounts: Vec<MigrationAccount>,
    pub arguments: Vec<MigrationArgument>,
}

impl InstructionBinding {
    pub fn build(&self, sources: &BTreeMap<String, Address>) -> Result<Instruction> {
        let accounts = self
            .accounts
            .iter()
            .map(|binding| {
                let address = *sources.get(&binding.source).ok_or_else(|| {
                    fail(
                        format!("migration.account_mapping.{}", binding.idl_name),
                        format!("source '{}' has no address", binding.source),
                    )
                })?;
                Ok(if binding.writable {
                    AccountMeta::new(address, binding.signer)
                } else {
                    AccountMeta::new_readonly(address, binding.signer)
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut data = self.discriminator.clone();
        for argument in &self.arguments {
            data.extend(&argument.encoded);
        }
        Ok(Instruction {
            program_id: self.program_id,
            accounts,
            data,
        })
    }
}

#[derive(Clone, Debug)]
pub struct LoadedProject {
    pub manifest: ProjectManifest,
    pub manifest_path: PathBuf,
    pub v1_program_path: PathBuf,
    pub v2_program_path: PathBuf,
    pub v1_idl_path: PathBuf,
    pub v2_idl_path: PathBuf,
    pub v1: ProgramModel,
    pub v2: ProgramModel,
    pub migration: InstructionBinding,
}

impl ProjectManifest {
    pub fn parse(source: &str) -> Result<Self> {
        if source.len() > 4 * 1024 * 1024 {
            return Err(fail("manifest", "manifest exceeds 4 MiB limit"));
        }
        let manifest: Self =
            toml::from_str(source).map_err(|e| fail("doppelganger.toml", e.to_string()))?;
        if manifest.schema_version.is_some_and(|version| version != 1) {
            return Err(fail(
                "schema_version",
                "supported version: 1; migrate using docs/migration-guide.md",
            ));
        }
        let fixture_count: usize = manifest.fixtures.values().map(|f| f.account.len()).sum();
        if manifest.addresses.len() > 10_000
            || manifest.migration_targets.len() > 10_000
            || fixture_count > 10_000
            || manifest
                .fixture
                .as_ref()
                .is_some_and(|f| f.account.len() > 10_000)
        {
            return Err(fail(
                "manifest",
                "maximum 10,000 bindings, targets or fixture accounts",
            ));
        }
        let mut names = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        for target in &manifest.migration_targets {
            if target.name.is_empty()
                || !names.insert(&target.name)
                || !addresses.insert(&target.address)
            {
                return Err(fail(
                    "migration_targets",
                    "empty name or duplicate name/address",
                ));
            }
        }
        if manifest.project.name.is_empty()
            || !manifest
                .project
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(fail("project.name", "must use letters, digits, '-' or '_'"));
        }
        if manifest.project.framework != "anchor" {
            return Err(fail("project.framework", "only anchor is supported"));
        }
        if manifest.migration.instruction.is_empty() {
            return Err(fail("migration.instruction", "must not be empty"));
        }
        for (label, version) in [
            ("versions.v1", &manifest.versions.v1),
            ("versions.v2", &manifest.versions.v2),
        ] {
            if version.program_path.as_os_str().is_empty() {
                return Err(fail(format!("{label}.program_path"), "missing binary path"));
            }
            if version.idl.as_os_str().is_empty() {
                return Err(fail(format!("{label}.idl"), "missing IDL path"));
            }
            Address::from_str(&version.program_id)
                .map_err(|e| fail(format!("{label}.program_id"), e.to_string()))?;
        }
        if manifest.versions.v1.program_id != manifest.versions.v2.program_id {
            return Err(fail(
                "versions",
                "V1 and V2 must use the same program ID for an upgrade",
            ));
        }
        preservation::validate_rules(&manifest.preservation)?;
        lifecycle::validate_scenarios(&manifest.scenario)?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> Result<LoadedProject> {
        let path = if path.is_dir() {
            path.join("doppelganger.toml")
        } else {
            path.to_owned()
        };
        let manifest_path = path
            .canonicalize()
            .map_err(|e| fail("manifest", e.to_string()))?;
        let source = String::from_utf8(
            dg_analysis::read_bounded(&manifest_path, 4 * 1024 * 1024)
                .map_err(|e| fail("manifest", e.to_string()))?,
        )
        .map_err(|e| fail("manifest", e.to_string()))?;
        let manifest = Self::parse(&source)?;
        let base = manifest_path
            .parent()
            .ok_or_else(|| fail("manifest", "no parent directory"))?;
        let artifact = |relative: &Path, field: &str| -> Result<PathBuf> {
            let path = base
                .join(relative)
                .canonicalize()
                .map_err(|e| fail(field, e.to_string()))?;
            if fs::metadata(&path)
                .map_err(|e| fail(field, e.to_string()))?
                .len()
                > 64 * 1024 * 1024
            {
                return Err(fail(field, "artifact exceeds 64 MiB limit"));
            }
            if !path.is_file() {
                return Err(fail(field, "expected file"));
            }
            Ok(path)
        };
        let v1_program_path = artifact(
            &manifest.versions.v1.program_path,
            "versions.v1.program_path",
        )?;
        let v2_program_path = artifact(
            &manifest.versions.v2.program_path,
            "versions.v2.program_path",
        )?;
        let v1_idl_path = artifact(&manifest.versions.v1.idl, "versions.v1.idl")?;
        let v2_idl_path = artifact(&manifest.versions.v2.idl, "versions.v2.idl")?;
        let read_idl = |path: &Path, field: &str| -> Result<ProgramModel> {
            let bytes = dg_analysis::read_bounded(path, 8 * 1024 * 1024)
                .map_err(|e| fail(field, e.to_string()))?;
            normalize(&bytes).map_err(|e| fail(field, e.to_string()))
        };
        let v1 = read_idl(&v1_idl_path, "versions.v1.idl")?;
        let v2 = read_idl(&v2_idl_path, "versions.v2.idl")?;
        for (label, version, model) in [
            ("versions.v1", &manifest.versions.v1, &v1),
            ("versions.v2", &manifest.versions.v2, &v2),
        ] {
            if model.address.as_deref() != Some(&version.program_id) {
                return Err(fail(
                    format!("{label}.program_id"),
                    "IDL address does not match manifest",
                ));
            }
        }
        // Generic address arguments need the same substitution as runtime execution.
        let programs = BTreeMap::from([
            (
                "v1".into(),
                manifest
                    .versions
                    .v1
                    .program_id
                    .parse::<Address>()
                    .map_err(|e| fail("versions.v1.program_id", e.to_string()))?,
            ),
            (
                "v2".into(),
                manifest
                    .versions
                    .v2
                    .program_id
                    .parse::<Address>()
                    .map_err(|e| fail("versions.v2.program_id", e.to_string()))?,
            ),
        ]);
        let resolved = ResolvedAddresses::resolve(&manifest.addresses, &programs)?;
        let mut migration_definition = manifest.migration.clone();
        for value in migration_definition.arguments.values_mut() {
            *value = fixture::resolve_value(value, &resolved.strings(), 0)?;
        }
        let migration = migration_definition.resolve(&v2, &manifest.versions.v2.program_id)?;
        Ok(LoadedProject {
            manifest,
            manifest_path,
            v1_program_path,
            v2_program_path,
            v1_idl_path,
            v2_idl_path,
            v1,
            v2,
            migration,
        })
    }
}

impl MigrationDefinition {
    pub fn resolve(&self, idl: &ProgramModel, program_id: &str) -> Result<InstructionBinding> {
        let instruction = idl.instructions.get(&self.instruction).ok_or_else(|| {
            fail(
                "migration.instruction",
                format!("instruction '{}' not found in V2 IDL", self.instruction),
            )
        })?;
        let discriminator = instruction.discriminator.clone().ok_or_else(|| {
            fail(
                "migration.instruction",
                "IDL instruction has no discriminator",
            )
        })?;
        let required: BTreeSet<_> = instruction
            .accounts
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        for key in self.account_mapping.keys() {
            if !required.contains(key.as_str()) {
                return Err(fail(
                    format!("migration.account_mapping.{key}"),
                    "unknown IDL account",
                ));
            }
        }
        let accounts = instruction
            .accounts
            .iter()
            .map(|account| {
                let source = self.account_mapping.get(&account.name).ok_or_else(|| {
                    fail(
                        format!("migration.account_mapping.{}", account.name),
                        "missing account binding",
                    )
                })?;
                if source.is_empty() {
                    return Err(fail(
                        format!("migration.account_mapping.{}", account.name),
                        "source name must not be empty",
                    ));
                }
                Ok(MigrationAccount {
                    idl_name: account.name.clone(),
                    source: source.clone(),
                    signer: account.signer,
                    writable: account.writable,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let required_args: BTreeSet<_> = instruction.args.iter().map(|a| a.name.as_str()).collect();
        for key in self.arguments.keys() {
            if !required_args.contains(key.as_str()) {
                return Err(fail(
                    format!("migration.arguments.{key}"),
                    "unknown IDL argument",
                ));
            }
        }
        let arguments = instruction
            .args
            .iter()
            .map(|arg| {
                let value = self.arguments.get(&arg.name).ok_or_else(|| {
                    fail(
                        format!("migration.arguments.{}", arg.name),
                        "missing argument",
                    )
                })?;
                let field = format!("migration.arguments.{}", arg.name);
                let encoded = encode(value, &arg.ty, &idl.types, &field)?;
                Ok(MigrationArgument {
                    name: arg.name.clone(),
                    ty: arg.ty.clone(),
                    encoded,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let program_id = Address::from_str(program_id)
            .map_err(|e| fail("versions.v2.program_id", e.to_string()))?;
        Ok(InstructionBinding {
            program_id,
            instruction: self.instruction.clone(),
            discriminator,
            accounts,
            arguments,
        })
    }
}

fn length_prefix(len: usize, field: &str) -> Result<Vec<u8>> {
    Ok(u32::try_from(len)
        .map_err(|_| fail(field, "value too long"))?
        .to_le_bytes()
        .to_vec())
}

pub fn encode(
    value: &toml::Value,
    ty: &FieldType,
    types: &BTreeMap<String, TypeModel>,
    field: &str,
) -> Result<Vec<u8>> {
    encode_inner(value, ty, types, field, 0)
}

fn encode_inner(
    value: &toml::Value,
    ty: &FieldType,
    types: &BTreeMap<String, TypeModel>,
    field: &str,
    depth: usize,
) -> Result<Vec<u8>> {
    if depth > 64 {
        return Err(fail(field, "maximum type depth exceeded"));
    }
    let encode = |v: &toml::Value, t: &FieldType, ts: &BTreeMap<String, TypeModel>, f: &str| {
        encode_inner(v, t, ts, f, depth + 1)
    };
    match ty {
        FieldType::Primitive(name) => match name.as_str() {
            "bool" => Ok(vec![u8::from(
                value
                    .as_bool()
                    .ok_or_else(|| fail(field, "expected boolean"))?,
            )]),
            "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" => {
                let text = if let Some(n) = value.as_integer() {
                    n.to_string()
                } else if let Some(s) = value.as_str() {
                    match s.split_once(':') {
                        Some((prefix, digits)) if prefix == name => digits.to_string(),
                        Some(_) => {
                            return Err(fail(field, "integer type prefix does not match IDL"))
                        }
                        None => s.to_string(),
                    }
                } else {
                    return Err(fail(field, "expected integer or typed decimal string"));
                };
                if text.len() > 40 {
                    return Err(fail(field, format!("value outside {name} range")));
                }
                let n = text
                    .parse::<BigInt>()
                    .map_err(|_| fail(field, "expected integer"))?;
                let bits = name.trim_start_matches(['u', 'i']).parse::<u32>().unwrap();
                let signed = name.starts_with('i');
                let bound = BigInt::from(1u8) << (if signed { bits - 1 } else { bits }) as usize;
                let minimum = if signed { -&bound } else { BigInt::from(0u8) };
                if n < minimum || n >= bound {
                    return Err(fail(field, format!("value outside {name} range")));
                }
                let mut bytes = n.to_signed_bytes_le();
                bytes.resize(
                    (bits / 8) as usize,
                    if n < BigInt::from(0) { 255 } else { 0 },
                );
                Ok(bytes)
            }
            "pubkey" | "publicKey" => {
                let address = Address::from_str(
                    value
                        .as_str()
                        .ok_or_else(|| fail(field, "expected base58 public key"))?,
                )
                .map_err(|e| fail(field, e.to_string()))?;
                Ok(address.as_ref().to_vec())
            }
            "string" => {
                let bytes = value
                    .as_str()
                    .ok_or_else(|| fail(field, "expected string"))?
                    .as_bytes();
                let mut result = length_prefix(bytes.len(), field)?;
                result.extend(bytes);
                Ok(result)
            }
            "bytes" => {
                let raw = value
                    .as_array()
                    .ok_or_else(|| fail(field, "expected byte array"))?;
                let mut result = length_prefix(raw.len(), field)?;
                for byte in raw {
                    result.push(
                        u8::try_from(
                            byte.as_integer()
                                .ok_or_else(|| fail(field, "expected byte"))?,
                        )
                        .map_err(|_| fail(field, "byte outside 0..255"))?,
                    );
                }
                Ok(result)
            }
            _ => Err(fail(
                field,
                format!("argument type {name} is not supported yet"),
            )),
        },
        FieldType::Vec(inner) | FieldType::Array(inner, _) => {
            let items = value
                .as_array()
                .ok_or_else(|| fail(field, "expected array"))?;
            if let FieldType::Array(_, len) = ty {
                if items.len() != *len {
                    return Err(fail(field, format!("expected {len} elements")));
                }
            }
            let mut result = if matches!(ty, FieldType::Vec(_)) {
                length_prefix(items.len(), field)?
            } else {
                Vec::new()
            };
            for (index, item) in items.iter().enumerate() {
                result.extend(encode(item, inner, types, &format!("{field}[{index}]"))?);
            }
            Ok(result)
        }
        FieldType::Option(inner) => {
            let table = value
                .as_table()
                .ok_or_else(|| fail(field, "expected { none = true } or { some = value }"))?;
            if table.len() != 1 {
                return Err(fail(field, "option needs exactly one of none or some"));
            }
            if table.get("none").is_some_and(|v| v.as_bool() == Some(true)) {
                return Ok(vec![0]);
            }
            let value = table
                .get("some")
                .ok_or_else(|| fail(field, "option needs none or some"))?;
            let mut result = vec![1];
            result.extend(encode(value, inner, types, field)?);
            Ok(result)
        }
        FieldType::Tuple(parts) => {
            let items = value
                .as_array()
                .ok_or_else(|| fail(field, "expected tuple array"))?;
            if items.len() != parts.len() {
                return Err(fail(field, "tuple length mismatch"));
            }
            let mut result = Vec::new();
            for (index, (value, ty)) in items.iter().zip(parts).enumerate() {
                result.extend(encode(value, ty, types, &format!("{field}[{index}]"))?);
            }
            Ok(result)
        }
        FieldType::Defined(name) => match types
            .get(name)
            .ok_or_else(|| fail(field, format!("unknown custom type {name}")))?
        {
            TypeModel::Alias { value: inner } => encode(value, inner, types, field),
            TypeModel::Struct { fields } => {
                let table = value
                    .as_table()
                    .ok_or_else(|| fail(field, "expected struct table"))?;
                if table.len() != fields.len() {
                    return Err(fail(field, "struct field count mismatch"));
                }
                let mut result = Vec::new();
                for member in fields {
                    let value = table.get(&member.name).ok_or_else(|| {
                        fail(format!("{field}.{}", member.name), "missing struct field")
                    })?;
                    result.extend(encode(
                        value,
                        &member.ty,
                        types,
                        &format!("{field}.{}", member.name),
                    )?);
                }
                Ok(result)
            }
            TypeModel::Enum { variants } => {
                if variants.is_empty() || variants.len() > 256 {
                    return Err(fail(field, "enum requires 1..256 variants"));
                }
                if let Some(table) = value.as_table() {
                    if table.keys().any(|key| key != "variant" && key != "fields") {
                        return Err(fail(field, "unknown enum value field"));
                    }
                }
                let variant_name = value
                    .as_str()
                    .or_else(|| {
                        value
                            .as_table()
                            .and_then(|t| t.get("variant"))
                            .and_then(toml::Value::as_str)
                    })
                    .ok_or_else(|| fail(field, "expected enum variant name"))?;
                let (index, variant) = variants
                    .iter()
                    .enumerate()
                    .find(|(_, v)| v.name == variant_name)
                    .ok_or_else(|| fail(field, format!("unknown enum variant {variant_name}")))?;
                let mut result =
                    vec![u8::try_from(index).map_err(|_| fail(field, "too many enum variants"))?];
                if variant.fields.is_empty()
                    && value.as_table().is_some_and(|t| t.contains_key("fields"))
                {
                    return Err(fail(field, "unit enum variant cannot have payload fields"));
                }
                if !variant.fields.is_empty() {
                    let payload = value
                        .as_table()
                        .and_then(|t| t.get("fields"))
                        .ok_or_else(|| fail(field, "enum variant needs fields"))?;
                    let named = variant
                        .fields
                        .iter()
                        .any(|f| f.name.parse::<usize>().is_err());
                    if named {
                        let table = payload
                            .as_table()
                            .ok_or_else(|| fail(field, "expected named enum fields"))?;
                        if table.len() != variant.fields.len() {
                            return Err(fail(field, "enum field count mismatch"));
                        }
                        for member in &variant.fields {
                            result.extend(encode(
                                table
                                    .get(&member.name)
                                    .ok_or_else(|| fail(field, "missing enum field"))?,
                                &member.ty,
                                types,
                                field,
                            )?);
                        }
                    } else {
                        let items = payload
                            .as_array()
                            .ok_or_else(|| fail(field, "expected enum field array"))?;
                        if items.len() != variant.fields.len() {
                            return Err(fail(field, "enum field count mismatch"));
                        }
                        for (item, member) in items.iter().zip(&variant.fields) {
                            result.extend(encode(item, &member.ty, types, field)?);
                        }
                    }
                }
                Ok(result)
            }
        },
    }
}
