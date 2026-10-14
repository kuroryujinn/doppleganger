use crate::model::{AccountSchemaModel, FieldType, TypeModel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeKind {
    Exact,
    Dynamic,
    PartiallyKnown,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeEstimate {
    pub kind: SizeKind,
    pub bytes: Option<usize>,
    pub minimum_bytes: Option<usize>,
}

impl SizeEstimate {
    fn exact(n: usize) -> Self {
        Self {
            kind: SizeKind::Exact,
            bytes: Some(n),
            minimum_bytes: Some(n),
        }
    }
    fn dynamic(min: usize) -> Self {
        Self {
            kind: SizeKind::Dynamic,
            bytes: None,
            minimum_bytes: Some(min),
        }
    }
    fn unknown() -> Self {
        Self {
            kind: SizeKind::Unknown,
            bytes: None,
            minimum_bytes: None,
        }
    }
    fn add(self, other: Self) -> Self {
        let Some(minimum) = self
            .minimum_bytes
            .zip(other.minimum_bytes)
            .and_then(|(a, b)| a.checked_add(b))
        else {
            return Self::unknown();
        };
        if let (Some(a), Some(b)) = (self.bytes, other.bytes) {
            if let Some(sum) = a.checked_add(b) {
                return Self::exact(sum);
            }
        }
        Self {
            kind: if self.kind == SizeKind::Dynamic || other.kind == SizeKind::Dynamic {
                SizeKind::Dynamic
            } else {
                SizeKind::PartiallyKnown
            },
            bytes: None,
            minimum_bytes: Some(minimum),
        }
    }
}

fn sequence<'a>(
    items: impl IntoIterator<Item = &'a FieldType>,
    types: &BTreeMap<String, TypeModel>,
    depth: usize,
    budget: &mut usize,
) -> SizeEstimate {
    items.into_iter().fold(SizeEstimate::exact(0), |n, ty| {
        n.add(estimate_inner(ty, types, depth + 1, budget))
    })
}

pub fn estimate_type(ty: &FieldType, types: &BTreeMap<String, TypeModel>) -> SizeEstimate {
    estimate_inner(ty, types, 0, &mut 100_000)
}

fn estimate_inner(
    ty: &FieldType,
    types: &BTreeMap<String, TypeModel>,
    depth: usize,
    budget: &mut usize,
) -> SizeEstimate {
    if depth > 64 || *budget == 0 {
        return SizeEstimate::unknown();
    }
    *budget -= 1;
    match ty {
        FieldType::Primitive(name) => match name.as_str() {
            "bool" | "u8" | "i8" => SizeEstimate::exact(1),
            "u16" | "i16" => SizeEstimate::exact(2),
            "u32" | "i32" | "f32" | "char" => SizeEstimate::exact(4),
            "u64" | "i64" | "f64" => SizeEstimate::exact(8),
            "u128" | "i128" => SizeEstimate::exact(16),
            "u256" | "i256" | "pubkey" | "publicKey" => SizeEstimate::exact(32),
            "string" | "bytes" => SizeEstimate::dynamic(4),
            _ => SizeEstimate::unknown(),
        },
        FieldType::Vec(_) => SizeEstimate::dynamic(4),
        FieldType::Option(_) => SizeEstimate::dynamic(1),
        FieldType::Array(inner, len) => {
            let part = estimate_inner(inner, types, depth + 1, budget);
            if *len == 0 {
                return SizeEstimate::exact(0);
            }
            let Some(min) = part.minimum_bytes.and_then(|n| n.checked_mul(*len)) else {
                return SizeEstimate::unknown();
            };
            if let Some(n) = part.bytes.and_then(|n| n.checked_mul(*len)) {
                SizeEstimate::exact(n)
            } else {
                SizeEstimate {
                    kind: part.kind,
                    bytes: None,
                    minimum_bytes: Some(min),
                }
            }
        }
        FieldType::Tuple(items) => sequence(items.iter(), types, depth + 1, budget),
        FieldType::Defined(name) => match types.get(name) {
            Some(TypeModel::Struct { fields }) => {
                sequence(fields.iter().map(|f| &f.ty), types, depth + 1, budget)
            }
            Some(TypeModel::Alias { value }) => estimate_inner(value, types, depth + 1, budget),
            Some(TypeModel::Enum { variants }) => {
                if variants.len() > 256 {
                    return SizeEstimate::unknown();
                }
                let sizes: Vec<_> = variants
                    .iter()
                    .map(|v| sequence(v.fields.iter().map(|f| &f.ty), types, depth + 1, budget))
                    .collect();
                let Some(min) = sizes.iter().filter_map(|v| v.minimum_bytes).min() else {
                    return SizeEstimate::unknown();
                };
                let exact = sizes.first().and_then(|v| v.bytes);
                if exact.is_some() && sizes.iter().all(|v| v.bytes == exact) {
                    exact
                        .and_then(|n| n.checked_add(1))
                        .map(SizeEstimate::exact)
                        .unwrap_or_else(SizeEstimate::unknown)
                } else {
                    min.checked_add(1)
                        .map(SizeEstimate::dynamic)
                        .unwrap_or_else(SizeEstimate::unknown)
                }
            }
            None => SizeEstimate::unknown(),
        },
    }
}

pub fn estimate_account(
    account: &AccountSchemaModel,
    types: &BTreeMap<String, TypeModel>,
) -> SizeEstimate {
    let Some(discriminator) = &account.discriminator else {
        return SizeEstimate::unknown();
    };
    SizeEstimate::exact(discriminator.len()).add(sequence(
        account.fields.iter().map(|f| &f.ty),
        types,
        0,
        &mut 100_000,
    ))
}
