//! IDL-driven state decoding. No VM, transactions, or preservation policy.
pub mod raw;
use dg_analysis::model::{FieldModel, FieldType, ProgramModel, TypeModel};
pub use dg_config::{FixtureAccount, FixtureDefinition};
use num_bigint::{BigInt, Sign};
use serde::{Deserialize, Serialize};
use solana_address::Address;
use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Value {
    Bool(bool),
    Integer(#[serde(with = "decimal")] BigInt),
    Pubkey(String),
    String(String),
    Sequence(Vec<Value>),
    Option(Option<Box<Value>>),
    Struct(BTreeMap<String, Value>),
    Enum {
        variant: String,
        fields: BTreeMap<String, Value>,
    },
}

mod decimal {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        n: &BigInt,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&n.to_string())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<BigInt, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateError {
    pub code: String,
    pub path: String,
    pub offset: usize,
    pub message: String,
}
impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {} (byte {}): {}",
            self.code, self.path, self.offset, self.message
        )
    }
}
impl std::error::Error for StateError {}
type Result<T> = std::result::Result<T, StateError>;
fn error(code: &str, path: &str, offset: usize, message: impl Into<String>) -> StateError {
    StateError {
        code: code.into(),
        path: path.into(),
        offset,
        message: message.into(),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DecodeLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_sequence: usize,
    pub max_values: usize,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_bytes: 10 * 1024 * 1024,
            max_depth: 64,
            max_sequence: 100_000,
            max_values: 100_000,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum TrailingBytes {
    Reject,
    Preserve,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedAccount {
    pub account_type: String,
    pub fields: BTreeMap<String, Value>,
    pub consumed_bytes: usize,
    pub trailing_bytes: Vec<u8>,
}

pub fn decode_account(
    model: &ProgramModel,
    account_type: &str,
    data: &[u8],
    limits: DecodeLimits,
    trailing: TrailingBytes,
) -> Result<DecodedAccount> {
    if data.len() > limits.max_bytes {
        return Err(error(
            "LIMIT",
            account_type,
            0,
            "account byte limit exceeded",
        ));
    }
    let account = model
        .accounts
        .get(account_type)
        .ok_or_else(|| error("UNKNOWN_TYPE", account_type, 0, "unknown account type"))?;
    let discriminator = account.discriminator.as_ref().ok_or_else(|| {
        error(
            "UNSUPPORTED",
            account_type,
            0,
            "explicit discriminator required",
        )
    })?;
    let mut reader = Reader {
        data,
        offset: 0,
        remaining: limits.max_values,
        limits,
        model,
    };
    if reader.take(discriminator.len(), account_type)? != discriminator {
        return Err(error(
            "DISCRIMINATOR",
            account_type,
            0,
            "account discriminator mismatch",
        ));
    }
    let fields = reader.fields(&account.fields, account_type, 0)?;
    let trailing_bytes = data[reader.offset..].to_vec();
    if !trailing_bytes.is_empty() && matches!(trailing, TrailingBytes::Reject) {
        return Err(error(
            "TRAILING_BYTES",
            account_type,
            reader.offset,
            "unconsumed account bytes",
        ));
    }
    Ok(DecodedAccount {
        account_type: account_type.into(),
        fields,
        consumed_bytes: reader.offset,
        trailing_bytes,
    })
}

/// Decode one Borsh value using the same bounded decoder as account state.
pub fn decode_value(
    model: &ProgramModel,
    ty: &FieldType,
    data: &[u8],
    limits: DecodeLimits,
) -> Result<Value> {
    if data.len() > limits.max_bytes {
        return Err(error("LIMIT", "value", 0, "value byte limit exceeded"));
    }
    let mut reader = Reader {
        data,
        offset: 0,
        remaining: limits.max_values,
        limits,
        model,
    };
    let value = reader.value(ty, "value", 0)?;
    if reader.offset != data.len() {
        return Err(error(
            "TRAILING_BYTES",
            "value",
            reader.offset,
            "unconsumed value bytes",
        ));
    }
    Ok(value)
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
    remaining: usize,
    limits: DecodeLimits,
    model: &'a ProgramModel,
}
impl Reader<'_> {
    fn take(&mut self, len: usize, path: &str) -> Result<&[u8]> {
        let end = self
            .offset
            .checked_add(len)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| error("TRUNCATED", path, self.offset, format!("need {len} bytes")))?;
        let bytes = &self.data[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }
    fn byte(&mut self, path: &str) -> Result<u8> {
        Ok(self.take(1, path)?[0])
    }
    fn length(&mut self, path: &str) -> Result<usize> {
        let b = self.take(4, path)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }
    fn fields(
        &mut self,
        fields: &[FieldModel],
        path: &str,
        depth: usize,
    ) -> Result<BTreeMap<String, Value>> {
        let mut result = BTreeMap::new();
        for field in fields {
            let path = format!("{path}.{}", field.name);
            let value = self.value(&field.ty, &path, depth + 1)?;
            if result.insert(field.name.clone(), value).is_some() {
                return Err(error("SCHEMA", &path, self.offset, "duplicate field"));
            }
        }
        Ok(result)
    }
    fn sequence(&mut self, ty: &FieldType, len: usize, path: &str, depth: usize) -> Result<Value> {
        if len > self.limits.max_sequence || len > self.remaining {
            return Err(error("LIMIT", path, self.offset, "sequence limit exceeded"));
        }
        let mut values = Vec::new();
        for i in 0..len {
            values.push(self.value(ty, &format!("{path}[{i}]"), depth + 1)?);
        }
        Ok(Value::Sequence(values))
    }
    fn value(&mut self, ty: &FieldType, path: &str, depth: usize) -> Result<Value> {
        if depth > self.limits.max_depth || self.remaining == 0 {
            return Err(error(
                "LIMIT",
                path,
                self.offset,
                "depth or decoded value limit exceeded",
            ));
        }
        self.remaining -= 1;
        match ty {
            FieldType::Primitive(name) => match name.as_str() {
                "bool" => match self.byte(path)? {
                    0 => Ok(Value::Bool(false)),
                    1 => Ok(Value::Bool(true)),
                    _ => Err(error(
                        "INVALID_DATA",
                        path,
                        self.offset - 1,
                        "invalid boolean tag",
                    )),
                },
                "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" => {
                    let width = match &name[1..] {
                        "8" => 1,
                        "16" => 2,
                        "32" => 4,
                        "64" => 8,
                        _ => 16,
                    };
                    let bytes = self.take(width, path)?;
                    Ok(Value::Integer(if name.starts_with('i') {
                        BigInt::from_signed_bytes_le(bytes)
                    } else {
                        BigInt::from_bytes_le(Sign::Plus, bytes)
                    }))
                }
                "pubkey" | "publicKey" => {
                    let mut bytes = [0; 32];
                    bytes.copy_from_slice(self.take(32, path)?);
                    Ok(Value::Pubkey(Address::new_from_array(bytes).to_string()))
                }
                "string" => {
                    let len = self.length(path)?;
                    let offset = self.offset;
                    let text = std::str::from_utf8(self.take(len, path)?)
                        .map_err(|_| error("INVALID_DATA", path, offset, "invalid UTF-8"))?;
                    Ok(Value::String(text.into()))
                }
                "bytes" => {
                    let len = self.length(path)?;
                    self.sequence(&FieldType::Primitive("u8".into()), len, path, depth)
                }
                _ => Err(error(
                    "UNSUPPORTED",
                    path,
                    self.offset,
                    format!("unsupported primitive {name}"),
                )),
            },
            FieldType::Vec(inner) => {
                let len = self.length(path)?;
                self.sequence(inner, len, path, depth)
            }
            FieldType::Array(inner, len) => self.sequence(inner, *len, path, depth),
            FieldType::Option(inner) => match self.byte(path)? {
                0 => Ok(Value::Option(None)),
                1 => Ok(Value::Option(Some(Box::new(self.value(
                    inner,
                    path,
                    depth + 1,
                )?)))),
                _ => Err(error(
                    "INVALID_DATA",
                    path,
                    self.offset - 1,
                    "invalid option tag",
                )),
            },
            FieldType::Tuple(parts) => {
                let mut values = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    values.push(self.value(part, &format!("{path}[{i}]"), depth + 1)?);
                }
                Ok(Value::Sequence(values))
            }
            FieldType::Defined(name) => {
                let definition = self.model.types.get(name).ok_or_else(|| {
                    error(
                        "UNKNOWN_TYPE",
                        path,
                        self.offset,
                        format!("unknown custom type {name}"),
                    )
                })?;
                match definition {
                    TypeModel::Alias { value } => self.value(value, path, depth + 1),
                    TypeModel::Struct { fields } => {
                        Ok(Value::Struct(self.fields(fields, path, depth)?))
                    }
                    TypeModel::Enum { variants } => {
                        if variants.is_empty() || variants.len() > 256 {
                            return Err(error(
                                "SCHEMA",
                                path,
                                self.offset,
                                "enum requires 1..256 variants",
                            ));
                        }
                        let tag = self.byte(path)? as usize;
                        let variant = variants.get(tag).ok_or_else(|| {
                            error("INVALID_DATA", path, self.offset - 1, "invalid enum tag")
                        })?;
                        Ok(Value::Enum {
                            variant: variant.name.clone(),
                            fields: self.fields(&variant.fields, path, depth)?,
                        })
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBinding {
    pub name: String,
    pub address: String,
    pub owner: String,
    pub account_type: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramArtifact {
    pub program_id: String,
    pub program_path: PathBuf,
    pub idl_path: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub schema_version: u32,
    pub binding: AccountBinding,
    #[serde(with = "decimal")]
    pub lamports: BigInt,
    pub executable: bool,
    pub raw_data: Vec<u8>,
    pub decoded: DecodedAccount,
    pub data_length: usize,
    pub metadata: BTreeMap<String, String>,
}
impl AccountSnapshot {
    pub fn capture(
        binding: AccountBinding,
        lamports: u64,
        executable: bool,
        raw_data: Vec<u8>,
        model: &ProgramModel,
        limits: DecodeLimits,
        trailing: TrailingBytes,
    ) -> Result<Self> {
        for address in [&binding.address, &binding.owner] {
            Address::from_str(address)
                .map_err(|e| error("SNAPSHOT", &binding.name, 0, e.to_string()))?;
        }
        let decoded = decode_account(model, &binding.account_type, &raw_data, limits, trailing)?;
        Ok(Self {
            schema_version: 1,
            binding,
            lamports: lamports.into(),
            executable,
            data_length: raw_data.len(),
            raw_data,
            decoded,
            metadata: BTreeMap::new(),
        })
    }
    /// Call after deserializing externally supplied snapshots, before comparing state.
    pub fn validate(&self, model: &ProgramModel, limits: DecodeLimits) -> Result<()> {
        if self.schema_version != 1
            || self.data_length != self.raw_data.len()
            || self.lamports < BigInt::from(0u8)
            || self.lamports > BigInt::from(u64::MAX)
        {
            return Err(error(
                "SNAPSHOT",
                &self.binding.name,
                0,
                "invalid version, length or lamports",
            ));
        }
        for address in [&self.binding.address, &self.binding.owner] {
            Address::from_str(address)
                .map_err(|e| error("SNAPSHOT", &self.binding.name, 0, e.to_string()))?;
        }
        if decode_account(
            model,
            &self.binding.account_type,
            &self.raw_data,
            limits,
            TrailingBytes::Preserve,
        )? != self.decoded
        {
            return Err(error(
                "SNAPSHOT",
                &self.binding.name,
                0,
                "decoded state does not match raw bytes",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeWorld {
    pub accounts: BTreeMap<String, AccountSnapshot>,
    pub programs: BTreeMap<String, ProgramArtifact>,
}

/// Materialize declarative accounts locally; does not execute initialization or migration.
pub fn load_fixture(
    fixture: &FixtureDefinition,
    model: &ProgramModel,
    bindings: &BTreeMap<String, String>,
    limits: DecodeLimits,
) -> Result<RuntimeWorld> {
    if !fixture.initialize.is_empty() || !fixture.setup.is_empty() {
        return Err(error(
            "FIXTURE",
            &fixture.name,
            0,
            "setup instructions require the runtime world builder",
        ));
    }
    if fixture.account.len() > 10_000 {
        return Err(error(
            "LIMIT",
            &fixture.name,
            0,
            "maximum 10,000 fixture accounts",
        ));
    }
    let mut total_bytes = 0usize;
    let mut world = RuntimeWorld::default();
    let mut addresses = std::collections::BTreeSet::new();
    for account in &fixture.account {
        if account.name.is_empty() || world.accounts.contains_key(&account.name) {
            return Err(error(
                "FIXTURE",
                &account.name,
                0,
                "empty or duplicate account name",
            ));
        }
        let resolve = |s: &str| -> Result<String> {
            if let Some(name) = s.strip_prefix('$') {
                bindings.get(name).cloned().ok_or_else(|| {
                    error(
                        "FIXTURE",
                        &account.name,
                        0,
                        format!("unknown binding {name}"),
                    )
                })
            } else {
                Ok(s.into())
            }
        };
        let binding = AccountBinding {
            name: account.name.clone(),
            address: resolve(&account.address)?,
            owner: resolve(&account.owner)?,
            account_type: account.account_type.clone(),
        };
        if !addresses.insert(binding.address.clone()) {
            return Err(error(
                "FIXTURE",
                &account.name,
                0,
                "duplicate account address",
            ));
        }
        let schema = model
            .accounts
            .get(&account.account_type)
            .ok_or_else(|| error("UNKNOWN_TYPE", &account.name, 0, "unknown account type"))?;
        let mut raw = schema.discriminator.clone().ok_or_else(|| {
            error(
                "UNSUPPORTED",
                &account.name,
                0,
                "explicit discriminator required",
            )
        })?;
        let fields = account.fields.as_ref().ok_or_else(|| {
            error(
                "FIXTURE",
                &account.name,
                0,
                "instruction-created account requires the runtime world builder",
            )
        })?;
        if fields.len() != schema.fields.len() {
            return Err(error(
                "FIXTURE",
                &account.name,
                0,
                "account field count mismatch",
            ));
        }
        for field in &schema.fields {
            let path = format!("{}.{}", account.name, field.name);
            let input = fields
                .get(&field.name)
                .ok_or_else(|| error("FIXTURE", &path, 0, "missing field"))?;
            let input = dg_config::fixture::resolve_value(input, bindings, 0)
                .map_err(|e| error("FIXTURE", &e.field, 0, e.message))?;
            raw.extend(
                dg_config::encode(&input, &field.ty, &model.types, &path)
                    .map_err(|e| error("FIXTURE", &e.field, 0, e.message))?,
            );
            if raw.len() > limits.max_bytes {
                return Err(error("LIMIT", &path, 0, "account byte limit exceeded"));
            }
        }
        let lamports = dg_config::encode(
            &account.lamports,
            &FieldType::Primitive("u64".into()),
            &model.types,
            "lamports",
        )
        .map_err(|e| error("FIXTURE", &e.field, 0, e.message))?;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&lamports);
        let snapshot = AccountSnapshot::capture(
            binding,
            u64::from_le_bytes(bytes),
            account.executable,
            raw,
            model,
            limits,
            TrailingBytes::Reject,
        )?;
        total_bytes = total_bytes
            .checked_add(snapshot.raw_data.len())
            .ok_or_else(|| error("LIMIT", &account.name, 0, "fixture byte count overflow"))?;
        if total_bytes > 64 * 1024 * 1024 {
            return Err(error(
                "LIMIT",
                &account.name,
                0,
                "fixture exceeds 64 MiB total account data",
            ));
        }
        world.accounts.insert(account.name.clone(), snapshot);
    }
    Ok(world)
}
