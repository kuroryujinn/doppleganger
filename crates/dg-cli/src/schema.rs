use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, fs, path::Path};

#[derive(Serialize)]
pub struct Schema {
    pub v1_account: String,
    pub v2_account: String,
    pub v1_bytes: usize,
    pub v2_bytes: usize,
    pub added_fields: Vec<String>,
    pub removed_fields: Vec<String>,
    pub changed_fields: Vec<String>,
    pub reordered_fields: bool,
    pub discriminator_changed: bool,
    pub pda_seeds_changed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountMapping {
    pub v1: String,
    pub v2: String,
    pub pda: Option<PdaMapping>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdaMapping {
    pub account: String,
    pub v1_instruction: String,
    pub v2_instructions: Vec<String>,
}

#[derive(Serialize)]
pub struct AccountDiff {
    pub v1_account: String,
    pub v2_account: String,
    pub added_fields: Vec<String>,
    pub removed_fields: Vec<String>,
    pub changed_fields: Vec<String>,
    pub reordered_fields: bool,
    pub discriminator_changed: bool,
    pub pda_seeds_changed: Option<bool>,
}

fn named<'a>(items: &'a Value, name: &str) -> Result<&'a Value, Box<dyn Error>> {
    items
        .as_array()
        .and_then(|items| items.iter().find(|item| item["name"] == name))
        .ok_or_else(|| format!("IDL entry {name} missing").into())
}

fn fields<'a>(idl: &'a Value, account: &str) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    named(&idl["types"], account)?["type"]["fields"]
        .as_array()
        .ok_or_else(|| format!("IDL type {account} has no struct fields").into())
}

fn field_map(fields: &[Value]) -> Result<BTreeMap<String, Value>, Box<dyn Error>> {
    let mut result = BTreeMap::new();
    for field in fields {
        let name = field["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or("IDL field has no name")?;
        let ty = field
            .get("type")
            .filter(|ty| !ty.is_null())
            .ok_or_else(|| format!("IDL field {name} has no type"))?;
        if result.insert(name.to_owned(), ty.clone()).is_some() {
            return Err(format!("IDL has duplicate field {name}").into());
        }
    }
    Ok(result)
}

fn discriminator<'a>(idl: &'a Value, account: &str) -> Result<&'a Value, Box<dyn Error>> {
    let value = &named(&idl["accounts"], account)?["discriminator"];
    if value.as_array().is_none_or(Vec::is_empty) {
        return Err(format!("IDL account {account} has no discriminator").into());
    }
    Ok(value)
}

fn account_pda<'a>(
    idl: &'a Value,
    instruction: &str,
    account: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    let value = &named(
        &named(&idl["instructions"], instruction)?["accounts"],
        account,
    )?["pda"];
    if value.is_null() {
        return Err(
            format!("IDL instruction {instruction} has no PDA metadata for {account}").into(),
        );
    }
    Ok(value)
}

pub fn compare_project(
    v1_path: &Path,
    v2_path: &Path,
    expected_program_id: &str,
    mappings: &[AccountMapping],
) -> Result<Vec<AccountDiff>, Box<dyn Error>> {
    let v1: Value = serde_json::from_slice(&fs::read(v1_path)?)?;
    let v2: Value = serde_json::from_slice(&fs::read(v2_path)?)?;
    if v1["address"] != expected_program_id || v2["address"] != expected_program_id {
        return Err("IDL address does not match the project program ID".into());
    }
    mappings
        .iter()
        .map(|mapping| diff_account(&v1, &v2, mapping))
        .collect()
}

fn diff_account(
    v1: &Value,
    v2: &Value,
    mapping: &AccountMapping,
) -> Result<AccountDiff, Box<dyn Error>> {
    let old_fields = fields(v1, &mapping.v1)?;
    let new_fields = fields(v2, &mapping.v2)?;
    let old = field_map(old_fields)?;
    let new = field_map(new_fields)?;
    let old_common: Vec<_> = old_fields
        .iter()
        .filter_map(|f| f["name"].as_str())
        .filter(|name| new.contains_key(*name))
        .collect();
    let new_common: Vec<_> = new_fields
        .iter()
        .filter_map(|f| f["name"].as_str())
        .filter(|name| old.contains_key(*name))
        .collect();
    let pda_seeds_changed = if let Some(pda) = &mapping.pda {
        if pda.v2_instructions.is_empty() {
            return Err("PDA mapping needs at least one V2 instruction".into());
        }
        let original = account_pda(v1, &pda.v1_instruction, &pda.account)?;
        let mut changed = false;
        for instruction in &pda.v2_instructions {
            changed |= original != account_pda(v2, instruction, &pda.account)?;
        }
        Some(changed)
    } else {
        None
    };
    Ok(AccountDiff {
        v1_account: mapping.v1.clone(),
        v2_account: mapping.v2.clone(),
        added_fields: new
            .keys()
            .filter(|key| !old.contains_key(*key))
            .cloned()
            .collect(),
        removed_fields: old
            .keys()
            .filter(|key| !new.contains_key(*key))
            .cloned()
            .collect(),
        changed_fields: old
            .iter()
            .filter_map(|(name, ty)| {
                new.get(name)
                    .filter(|new_ty| *new_ty != ty)
                    .map(|new_ty| format!("{name}: {ty} -> {new_ty}"))
            })
            .collect(),
        reordered_fields: old_common != new_common,
        discriminator_changed: discriminator(v1, &mapping.v1)? != discriminator(v2, &mapping.v2)?,
        pda_seeds_changed,
    })
}

fn compare_values(
    v1: &Value,
    v2: &Value,
    v1_account: &str,
    v2_account: &str,
    v1_bytes: usize,
    v2_bytes: usize,
    expected_program_id: &str,
) -> Result<Schema, Box<dyn Error>> {
    if v1["address"] != expected_program_id || v2["address"] != expected_program_id {
        return Err("IDL address does not match the program under test".into());
    }
    let mapping = AccountMapping {
        v1: v1_account.to_owned(),
        v2: v2_account.to_owned(),
        pda: Some(PdaMapping {
            account: "vault".to_owned(),
            v1_instruction: "deposit".to_owned(),
            v2_instructions: ["deposit", "migrate", "withdraw"]
                .map(str::to_owned)
                .to_vec(),
        }),
    };
    let diff = diff_account(v1, v2, &mapping)?;
    Ok(Schema {
        v1_account: diff.v1_account,
        v2_account: diff.v2_account,
        v1_bytes,
        v2_bytes,
        added_fields: diff.added_fields,
        removed_fields: diff.removed_fields,
        changed_fields: diff.changed_fields,
        reordered_fields: diff.reordered_fields,
        discriminator_changed: diff.discriminator_changed,
        pda_seeds_changed: diff.pda_seeds_changed.unwrap_or(false),
    })
}

pub fn compare(
    v1_path: &Path,
    v2_path: &Path,
    v1_account: &str,
    v2_account: &str,
    v1_bytes: usize,
    v2_bytes: usize,
    expected_program_id: &str,
) -> Result<Schema, Box<dyn Error>> {
    let v1: Value = serde_json::from_slice(&fs::read(v1_path)?)?;
    let v2: Value = serde_json::from_slice(&fs::read(v2_path)?)?;
    compare_values(
        &v1,
        &v2,
        v1_account,
        v2_account,
        v1_bytes,
        v2_bytes,
        expected_program_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_schema_and_seed_drift_and_rejects_missing_metadata() {
        let mut v1 = serde_json::json!({
            "address": "program",
            "accounts": [{"name": "Old", "discriminator": [1]}],
            "types": [{"name": "Old", "type": {"fields": [
                {"name": "owner", "type": "pubkey"},
                {"name": "balance", "type": "u64"}
            ]}}],
            "instructions": [{"name": "deposit", "accounts": [{"name": "vault", "pda": {"seeds": [1]}}]}]
        });
        let mut v2 = serde_json::json!({
            "address": "program",
            "accounts": [{"name": "New", "discriminator": [2]}],
            "types": [{"name": "New", "type": {"fields": [
                {"name": "balance", "type": "u32"},
                {"name": "owner", "type": "pubkey"},
                {"name": "limit", "type": "u64"}
            ]}}],
            "instructions": [
                {"name": "deposit", "accounts": [{"name": "vault", "pda": {"seeds": [1]}}]},
                {"name": "migrate", "accounts": [{"name": "vault", "pda": {"seeds": [2]}}]},
                {"name": "withdraw", "accounts": [{"name": "vault", "pda": {"seeds": [1]}}]}
            ]
        });
        let diff = compare_values(&v1, &v2, "Old", "New", 48, 56, "program").unwrap();
        assert_eq!(diff.added_fields, ["limit"]);
        assert_eq!(diff.changed_fields, ["balance: \"u64\" -> \"u32\""]);
        assert!(diff.reordered_fields && diff.discriminator_changed && diff.pda_seeds_changed);

        v2["instructions"][1]["accounts"][0]
            .as_object_mut()
            .unwrap()
            .remove("pda");
        assert!(compare_values(&v1, &v2, "Old", "New", 48, 56, "program").is_err());
        v1["address"] = serde_json::json!("another-program");
        assert!(compare_values(&v1, &v2, "Old", "New", 48, 56, "program").is_err());
    }
}
