use crate::{fail, FixtureDefinition, MigrationDefinition, ResolvedAddresses, Result};
use dg_analysis::model::{FieldType, ProgramModel};
use solana_address::Address;
use solana_instruction::Instruction;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

pub struct FixtureInstructions {
    pub initialize: Vec<Instruction>,
    pub setup: Vec<Instruction>,
}

impl FixtureDefinition {
    /// Compile V1 actions only; executing them belongs to the runtime world builder.
    pub fn instructions(
        &self,
        model: &ProgramModel,
        program_id: &str,
        resolved: &ResolvedAddresses,
    ) -> Result<FixtureInstructions> {
        if model
            .address
            .as_ref()
            .is_some_and(|address| address != program_id)
        {
            return Err(fail(
                "fixture.program_id",
                "program ID does not match V1 IDL",
            ));
        }
        let bindings = resolved.strings();
        let mut names = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        for account in &self.account {
            if account.name.is_empty() || !names.insert(&account.name) {
                return Err(fail("fixture.account", "empty or duplicate account name"));
            }
            for (label, source) in [("address", &account.address), ("owner", &account.owner)] {
                let value = resolve_value(&toml::Value::String(source.clone()), &bindings, 0)?;
                let address = Address::from_str(
                    value
                        .as_str()
                        .ok_or_else(|| fail(label, "expected address"))?,
                )
                .map_err(|e| fail(format!("fixture.{}.{}", account.name, label), e.to_string()))?;
                if label == "address" && !addresses.insert(address) {
                    return Err(fail("fixture.account", "duplicate account address"));
                }
            }
            let schema = model
                .accounts
                .get(&account.account_type)
                .ok_or_else(|| fail(&account.name, "unknown account type"))?;
            crate::encode(
                &account.lamports,
                &FieldType::Primitive("u64".into()),
                &model.types,
                "fixture.lamports",
            )?;
            if let Some(fields) = &account.fields {
                if fields.len() != schema.fields.len() {
                    return Err(fail(&account.name, "account field count mismatch"));
                }
                for field in &schema.fields {
                    let value = fields
                        .get(&field.name)
                        .ok_or_else(|| fail(&field.name, "missing account field"))?;
                    crate::encode(
                        &resolve_value(value, &bindings, 0)?,
                        &field.ty,
                        &model.types,
                        &field.name,
                    )?;
                }
            } else if self.initialize.is_empty() && self.setup.is_empty() {
                return Err(fail(
                    &account.name,
                    "account needs fields or configured initialization/setup",
                ));
            }
        }
        let build = |phase: &str, actions: &[MigrationDefinition]| -> Result<Vec<Instruction>> {
            actions
                .iter()
                .enumerate()
                .map(|(index, definition)| {
                    let compile = || -> Result<Instruction> {
                        let action = definition;
                        let mut action = action.clone();
                        for value in action.arguments.values_mut() {
                            *value = resolve_value(value, &bindings, 0)?;
                        }
                        let instruction = action
                            .resolve(model, program_id)?
                            .build(&resolved.addresses)?;
                        for account in &instruction.accounts {
                            if account.is_signer
                                && !resolved
                                    .signers
                                    .values()
                                    .any(|key| solana_signer::Signer::pubkey(key) == account.pubkey)
                            {
                                return Err(fail(
                                    "fixture.instructions",
                                    format!("missing keypair for signer {}", account.pubkey),
                                ));
                            }
                        }
                        Ok(instruction)
                    };
                    compile().map_err(|e| {
                        fail(
                            format!(
                                "fixture.{phase}[{index}] {}: {}",
                                definition.instruction, e.field
                            ),
                            e.message,
                        )
                    })
                })
                .collect()
        };
        Ok(FixtureInstructions {
            initialize: build("initialize", &self.initialize)?,
            setup: build("setup", &self.setup)?,
        })
    }
}

pub fn resolve_value(
    value: &toml::Value,
    bindings: &BTreeMap<String, String>,
    depth: usize,
) -> Result<toml::Value> {
    if depth > 64 {
        return Err(fail("fixture.fields", "maximum nesting depth exceeded"));
    }
    Ok(match value {
        toml::Value::String(s) if s.starts_with('$') => toml::Value::String(
            bindings
                .get(&s[1..])
                .cloned()
                .ok_or_else(|| fail(s, "unknown binding"))?,
        ),
        toml::Value::Array(items) => toml::Value::Array(
            items
                .iter()
                .map(|v| resolve_value(v, bindings, depth + 1))
                .collect::<Result<_>>()?,
        ),
        toml::Value::Table(fields) => toml::Value::Table(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), resolve_value(v, bindings, depth + 1)?)))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}
