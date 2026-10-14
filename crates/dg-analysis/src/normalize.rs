use crate::model::*;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl Error for AnalysisError {}

type Result<T> = std::result::Result<T, AnalysisError>;

fn error(path: &str, message: impl Into<String>) -> AnalysisError {
    AnalysisError {
        path: path.into(),
        message: message.into(),
    }
}

fn required<'a>(value: &'a Value, key: &str, path: &str) -> Result<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| error(path, format!("missing {key}")))
}

fn string(value: &Value, path: &str) -> Result<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| error(path, "expected nonempty string"))
}

fn array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| error(path, "expected array"))
}

fn entries<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    match value.get(key) {
        Some(v) => array(v, key),
        None => Ok(&EMPTY),
    }
}

static EMPTY: Vec<Value> = Vec::new();

fn discriminator(value: &Value, path: &str) -> Result<Option<Vec<u8>>> {
    let Some(raw) = value.get("discriminator") else {
        return Ok(None);
    };
    array(raw, path)?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| error(path, "discriminator must contain bytes"))
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn flag(value: &Value, current: &str, legacy: &str, path: &str) -> Result<bool> {
    match value.get(current).or_else(|| value.get(legacy)) {
        Some(v) => v
            .as_bool()
            .ok_or_else(|| error(path, format!("{current} must be boolean"))),
        None => Ok(false),
    }
}

fn parse_type(value: &Value, path: &str) -> Result<FieldType> {
    if let Some(primitive) = value.as_str() {
        if !matches!(
            primitive,
            "bool"
                | "u8"
                | "i8"
                | "u16"
                | "i16"
                | "u32"
                | "i32"
                | "u64"
                | "i64"
                | "u128"
                | "i128"
                | "u256"
                | "i256"
                | "f32"
                | "f64"
                | "pubkey"
                | "publicKey"
                | "string"
                | "bytes"
        ) {
            return Err(error(
                path,
                format!("unsupported primitive type {primitive}"),
            ));
        }
        return Ok(FieldType::Primitive(primitive.into()));
    }
    let obj = value
        .as_object()
        .ok_or_else(|| error(path, "unsupported field type"))?;
    if obj.len() != 1 {
        return Err(error(path, "field type must have exactly one tag"));
    }
    let (tag, inner) = obj.iter().next().unwrap();
    Ok(match tag.as_str() {
        "defined" => FieldType::Defined(if inner.is_string() {
            string(inner, path)?
        } else {
            if inner.as_object().is_none_or(|object| object.len() != 1) {
                return Err(error(path, "generic defined types are not supported"));
            }
            string(required(inner, "name", path)?, path)?
        }),
        "vec" => FieldType::Vec(Box::new(parse_type(inner, path)?)),
        "option" => FieldType::Option(Box::new(parse_type(inner, path)?)),
        "array" => {
            let parts = array(inner, path)?;
            if parts.len() != 2 {
                return Err(error(path, "array type needs element and length"));
            }
            let n = parts[1]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| error(path, "invalid array length"))?;
            FieldType::Array(Box::new(parse_type(&parts[0], path)?), n)
        }
        "tuple" => FieldType::Tuple(
            array(inner, path)?
                .iter()
                .map(|v| parse_type(v, path))
                .collect::<Result<_>>()?,
        ),
        _ => return Err(error(path, format!("unsupported type tag {tag}"))),
    })
}

fn fields(values: &Value, path: &str) -> Result<Vec<FieldModel>> {
    let mut names = BTreeSet::new();
    array(values, path)?
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let here = format!("{path}[{i}]");
            let name = string(required(item, "name", &here)?, &here)?;
            if !names.insert(name.clone()) {
                return Err(error(&here, format!("duplicate field {name}")));
            }
            Ok(FieldModel {
                name,
                ty: parse_type(required(item, "type", &here)?, &here)?,
            })
        })
        .collect()
}

fn seed(value: &Value, path: &str) -> Result<SeedModel> {
    let kind = string(required(value, "kind", path)?, path)?;
    let (bytes, reference) = match kind.as_str() {
        "const" => {
            let raw = array(required(value, "value", path)?, path)?;
            let bytes = raw
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or_else(|| error(path, "constant seed must contain bytes"))
                })
                .collect::<Result<Vec<_>>>()?;
            (Some(bytes), None)
        }
        "account" | "arg" => (None, Some(string(required(value, "path", path)?, path)?)),
        _ => return Err(error(path, format!("unsupported PDA seed kind {kind}"))),
    };
    Ok(SeedModel {
        kind,
        value: bytes,
        path: reference,
        ty: value.get("type").map(|v| parse_type(v, path)).transpose()?,
    })
}

fn pda(value: &Value, path: &str) -> Result<PdaModel> {
    let seeds = array(required(value, "seeds", path)?, path)?
        .iter()
        .enumerate()
        .map(|(i, v)| seed(v, &format!("{path}.seeds[{i}]")))
        .collect::<Result<_>>()?;
    let program = value
        .get("program")
        .map(|v| seed(v, &format!("{path}.program")))
        .transpose()?;
    Ok(PdaModel { seeds, program })
}

fn instruction_accounts(
    values: &Value,
    path: &str,
    prefix: &str,
    output: &mut Vec<InstructionAccountModel>,
) -> Result<()> {
    for (i, value) in array(values, path)?.iter().enumerate() {
        let here = format!("{path}[{i}]");
        let name = string(required(value, "name", &here)?, &here)?;
        let full = format!("{prefix}{name}");
        if let Some(group) = value.get("accounts") {
            instruction_accounts(group, &here, &format!("{full}."), output)?;
        } else {
            output.push(InstructionAccountModel {
                name: full,
                signer: flag(value, "signer", "isSigner", &here)?,
                writable: flag(value, "writable", "isMut", &here)?,
                optional: flag(value, "optional", "isOptional", &here)?,
                pda: value
                    .get("pda")
                    .map(|v| pda(v, &format!("{here}.pda")))
                    .transpose()?,
            });
        }
    }
    Ok(())
}

fn unique<T>(map: &mut BTreeMap<String, T>, name: String, value: T, path: &str) -> Result<()> {
    if map.insert(name.clone(), value).is_some() {
        return Err(error(path, format!("duplicate definition {name}")));
    }
    Ok(())
}

fn type_definition(value: &Value, path: &str) -> Result<TypeModel> {
    let kind = string(required(value, "kind", path)?, path)?;
    Ok(match kind.as_str() {
        "struct" => TypeModel::Struct {
            fields: fields(required(value, "fields", path)?, path)?,
        },
        "enum" => {
            let mut names = BTreeSet::new();
            let mut variants = Vec::new();
            for (i, variant) in array(required(value, "variants", path)?, path)?
                .iter()
                .enumerate()
            {
                let here = format!("{path}.variants[{i}]");
                let name = string(required(variant, "name", &here)?, &here)?;
                if !names.insert(name.clone()) {
                    return Err(error(&here, "duplicate enum variant"));
                }
                let payload = match variant.get("fields") {
                    None => Vec::new(),
                    Some(raw)
                        if raw.as_array().is_some_and(|v| {
                            v.first().is_some_and(|f| f.get("name").is_some())
                        }) =>
                    {
                        fields(raw, &here)?
                    }
                    Some(raw) => array(raw, &here)?
                        .iter()
                        .enumerate()
                        .map(|(j, ty)| {
                            Ok(FieldModel {
                                name: j.to_string(),
                                ty: parse_type(ty, &here)?,
                            })
                        })
                        .collect::<Result<_>>()?,
                };
                variants.push(EnumVariantModel {
                    name,
                    fields: payload,
                });
            }
            if variants.is_empty() {
                return Err(error(path, "enum needs at least one variant"));
            }
            TypeModel::Enum { variants }
        }
        "alias" => TypeModel::Alias {
            value: parse_type(required(value, "value", path)?, path)?,
        },
        _ => return Err(error(path, format!("unsupported type definition {kind}"))),
    })
}

fn referenced(
    ty: &FieldType,
    types: &BTreeMap<String, TypeModel>,
    stack: &mut Vec<String>,
    path: &str,
    depth: usize,
    budget: &mut usize,
) -> Result<()> {
    if depth > 64 || *budget == 0 {
        return Err(error(
            path,
            "custom type validation exceeds depth/work limit",
        ));
    }
    *budget -= 1;
    match ty {
        FieldType::Defined(name) => {
            let def = types
                .get(name)
                .ok_or_else(|| error(path, format!("unresolved custom type {name}")))?;
            if stack.contains(name) {
                return Err(error(
                    path,
                    format!(
                        "cyclic custom type reference: {} -> {name}",
                        stack.join(" -> ")
                    ),
                ));
            }
            stack.push(name.clone());
            match def {
                TypeModel::Struct { fields } => {
                    for field in fields {
                        referenced(&field.ty, types, stack, path, depth + 1, budget)?;
                    }
                }
                TypeModel::Enum { variants } => {
                    for variant in variants {
                        for field in &variant.fields {
                            referenced(&field.ty, types, stack, path, depth + 1, budget)?;
                        }
                    }
                }
                TypeModel::Alias { value } => {
                    referenced(value, types, stack, path, depth + 1, budget)?
                }
            }
            stack.pop();
        }
        FieldType::Vec(inner) | FieldType::Option(inner) | FieldType::Array(inner, _) => {
            referenced(inner, types, stack, path, depth + 1, budget)?
        }
        FieldType::Tuple(items) => {
            for item in items {
                referenced(item, types, stack, path, depth + 1, budget)?;
            }
        }
        FieldType::Primitive(_) => {}
    }
    Ok(())
}

pub fn normalize(bytes: &[u8]) -> Result<ProgramModel> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(error("idl", "IDL exceeds 8 MiB limit"));
    }
    let raw: Value = serde_json::from_slice(bytes).map_err(|e| error("idl", e.to_string()))?;
    let mut pending = vec![(&raw, 0usize)];
    let mut nodes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        if nodes > 100_000 || depth > 64 {
            return Err(error("idl", "IDL structure exceeds node/depth limit"));
        }
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|v| (v, depth + 1))),
            Value::Object(items) => pending.extend(items.values().map(|v| (v, depth + 1))),
            _ => {}
        }
    }
    if !raw.is_object() {
        return Err(error("idl", "expected JSON object"));
    }
    array(required(&raw, "instructions", "idl")?, "instructions")?;
    let metadata = raw.get("metadata").unwrap_or(&Value::Null);
    let mut model = ProgramModel {
        name: metadata
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| raw.get("name").and_then(Value::as_str))
            .unwrap_or("unknown")
            .into(),
        version: metadata
            .get("version")
            .and_then(Value::as_str)
            .or_else(|| raw.get("version").and_then(Value::as_str))
            .unwrap_or("unknown")
            .into(),
        address: raw
            .get("address")
            .and_then(Value::as_str)
            .map(str::to_owned),
        instructions: BTreeMap::new(),
        accounts: BTreeMap::new(),
        types: BTreeMap::new(),
        events: BTreeMap::new(),
        errors: BTreeMap::new(),
    };
    for value in entries(&raw, "types")? {
        let name = string(required(value, "name", "types")?, "types.name")?;
        if value
            .get("serialization")
            .and_then(Value::as_str)
            .is_some_and(|s| s != "borsh")
            || value.get("repr").is_some()
        {
            return Err(error(
                &name,
                "only Borsh serialization without custom repr is supported",
            ));
        }
        let def = type_definition(required(value, "type", "types")?, &format!("types.{name}"))?;
        unique(&mut model.types, name, def, "types")?;
    }
    for value in entries(&raw, "instructions")? {
        let name = string(
            required(value, "name", "instructions")?,
            "instructions.name",
        )?;
        let mut accounts = Vec::new();
        instruction_accounts(
            required(value, "accounts", &name)?,
            &name,
            "",
            &mut accounts,
        )?;
        if accounts
            .iter()
            .map(|a| &a.name)
            .collect::<BTreeSet<_>>()
            .len()
            != accounts.len()
        {
            return Err(error(&name, "duplicate instruction account"));
        }
        let instruction = InstructionModel {
            name: name.clone(),
            discriminator: discriminator(value, &name)?,
            accounts,
            args: fields(required(value, "args", &name)?, &name)?,
        };
        unique(&mut model.instructions, name, instruction, "instructions")?;
    }
    for value in entries(&raw, "accounts")? {
        let name = string(required(value, "name", "accounts")?, "accounts.name")?;
        let definition = if let Some(inline) = value.get("type") {
            type_definition(inline, &name)?
        } else {
            model
                .types
                .get(&name)
                .cloned()
                .ok_or_else(|| error(&name, "missing account type"))?
        };
        let TypeModel::Struct { fields } = definition else {
            return Err(error(&name, "account type must be struct"));
        };
        if value.get("type").is_some() {
            model
                .types
                .entry(name.clone())
                .or_insert_with(|| TypeModel::Struct {
                    fields: fields.clone(),
                });
        }
        unique(
            &mut model.accounts,
            name.clone(),
            AccountSchemaModel {
                name: name.clone(),
                discriminator: discriminator(value, &name)?,
                fields,
            },
            "accounts",
        )?;
    }
    for value in entries(&raw, "events")? {
        let name = string(required(value, "name", "events")?, "events.name")?;
        let fields = if let Some(raw_fields) = value.get("fields") {
            fields(raw_fields, &name)?
        } else if let Some(TypeModel::Struct { fields }) = model.types.get(&name) {
            fields.clone()
        } else {
            return Err(error(&name, "missing event fields/type"));
        };
        unique(
            &mut model.events,
            name.clone(),
            EventModel {
                name: name.clone(),
                discriminator: discriminator(value, &name)?,
                fields,
            },
            "events",
        )?;
    }
    for value in entries(&raw, "errors")? {
        let name = string(required(value, "name", "errors")?, "errors.name")?;
        let code = required(value, "code", &name)?
            .as_u64()
            .ok_or_else(|| error(&name, "error code must be integer"))?;
        unique(
            &mut model.errors,
            name.clone(),
            ErrorModel {
                name,
                code,
                message: value.get("msg").and_then(Value::as_str).map(str::to_owned),
            },
            "errors",
        )?;
    }
    let mut budget = 100_000usize;
    for (name, def) in &model.types {
        let mut stack = vec![name.clone()];
        match def {
            TypeModel::Struct { fields } => {
                for field in fields {
                    referenced(&field.ty, &model.types, &mut stack, name, 0, &mut budget)?;
                }
            }
            TypeModel::Enum { variants } => {
                for variant in variants {
                    for field in &variant.fields {
                        referenced(&field.ty, &model.types, &mut stack, name, 0, &mut budget)?;
                    }
                }
            }
            TypeModel::Alias { value } => {
                referenced(value, &model.types, &mut stack, name, 0, &mut budget)?
            }
        }
    }
    for (name, instruction) in &model.instructions {
        for arg in &instruction.args {
            referenced(&arg.ty, &model.types, &mut Vec::new(), name, 0, &mut budget)?;
        }
    }
    for (name, account) in &model.accounts {
        for field in &account.fields {
            referenced(
                &field.ty,
                &model.types,
                &mut Vec::new(),
                name,
                0,
                &mut budget,
            )?;
        }
    }
    for (name, event) in &model.events {
        for field in &event.fields {
            referenced(
                &field.ty,
                &model.types,
                &mut Vec::new(),
                name,
                0,
                &mut budget,
            )?;
        }
    }
    Ok(model)
}
