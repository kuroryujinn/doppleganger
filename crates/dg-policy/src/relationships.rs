//! Graph edges evaluated over immutable observed state and resolved public bindings.
use crate::CheckStatus;
use dg_config::preservation::valid_field_path;
use dg_state::Value;
use serde::{Deserialize, Serialize};
use solana_address::Address;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Authority,
    Owner,
    Reference,
    Parent,
    Mint,
    ProgramOwner,
    Pda,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub id: String,
    pub kind: Kind,
    pub account: String,
    pub field: Option<String>,
    pub before_field: Option<String>,
    pub target: String,
    #[serde(default = "crate::yes")]
    pub check_before: bool,
    #[serde(default = "crate::yes")]
    pub preserve: bool,
}
pub fn validate(rules: &[Relationship]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for r in rules {
        if r.id.trim().is_empty() || !ids.insert(&r.id) {
            return Err("empty or duplicate relationship ID".into());
        }
        if r.account.trim().is_empty() || r.target.trim().is_empty() {
            return Err("relationship needs account and target".into());
        }
        let field_kind = !matches!(r.kind, Kind::ProgramOwner | Kind::Pda);
        if field_kind != r.field.is_some() || !field_kind && r.before_field.is_some() {
            return Err(
                "field is required for field edges and forbidden for program_owner/PDA edges"
                    .into(),
            );
        }
        for field in r.field.iter().chain(r.before_field.iter()) {
            if !valid_field_path(field) {
                return Err("invalid relationship field path".into());
            }
        }
        if !r.check_before && r.preserve {
            return Err("preserve requires check_before; set preserve=false for a new edge".into());
        }
        if r.kind == Kind::Pda && !r.target.starts_with('$') {
            return Err("PDA target must name a configured PDA binding".into());
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
pub struct Account {
    pub name: String,
    pub address: String,
    pub owner: Option<String>,
    pub exists: bool,
    pub fields: Option<BTreeMap<String, Value>>,
    pub decode_error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Edge {
    pub id: String,
    pub kind: Kind,
    pub account: String,
    pub target: String,
    pub expected: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub status: CheckStatus,
    pub code: Option<String>,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct GraphReport {
    pub schema_version: u32,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub edges: Vec<Edge>,
}
fn account<'a>(accounts: &'a [Account], name: &str) -> Result<&'a Account, &'static str> {
    let mut matches = accounts.iter().filter(|a| a.name == name);
    let a = matches.next().ok_or("ACCOUNT_NOT_OBSERVED")?;
    if matches.next().is_some() {
        return Err("AMBIGUOUS_ACCOUNT");
    }
    if !a.exists {
        return Err("ACCOUNT_ABSENT");
    }
    Ok(a)
}
fn observed(a: &Account, rule: &Relationship, before: bool) -> Result<String, &'static str> {
    match rule.kind {
        Kind::ProgramOwner => return a.owner.clone().ok_or("OWNER_MISSING"),
        Kind::Pda => return Ok(a.address.clone()),
        _ => {}
    }
    if a.decode_error.is_some() {
        return Err("UNDECODABLE_ACCOUNT");
    }
    let fields = a.fields.as_ref().ok_or("UNDECODABLE_ACCOUNT")?;
    let path = if before {
        rule.before_field.as_ref().or(rule.field.as_ref())
    } else {
        rule.field.as_ref()
    }
    .ok_or("FIELD_MISSING")?;
    let mut parts = path.split('.');
    let mut value = fields
        .get(parts.next().ok_or("FIELD_MISSING")?)
        .ok_or("FIELD_MISSING")?;
    for part in parts {
        let Value::Struct(fields) = value else {
            return Err("FIELD_TYPE_MISMATCH");
        };
        value = fields.get(part).ok_or("FIELD_MISSING")?;
    }
    let Value::Pubkey(key) = value else {
        return Err("FIELD_TYPE_MISMATCH");
    };
    Address::from_str(key).map_err(|_| "INVALID_PUBLIC_KEY")?;
    Ok(key.clone())
}
pub fn evaluate(
    rules: &[Relationship],
    before: &[Account],
    after: &[Account],
    bindings: &BTreeMap<String, String>,
    pda_bindings: &BTreeSet<String>,
) -> Result<GraphReport, String> {
    validate(rules)?;
    let mut report = GraphReport {
        schema_version: 1,
        total: rules.len(),
        passed: 0,
        failed: 0,
        errors: 0,
        edges: vec![],
    };
    for rule in rules {
        let mut edge = Edge {
            id: format!("REL-{}", rule.id),
            kind: rule.kind,
            account: rule.account.clone(),
            target: rule.target.clone(),
            expected: None,
            before: None,
            after: None,
            status: CheckStatus::Pass,
            code: None,
            message: "Declared relationship satisfied".into(),
        };
        let result = (|| {
            let expected = if let Some(name) = rule.target.strip_prefix('$') {
                if rule.kind == Kind::Pda && !pda_bindings.contains(name) {
                    return Err("TARGET_IS_NOT_PDA");
                }
                bindings
                    .get(name)
                    .cloned()
                    .ok_or("UNKNOWN_TARGET_BINDING")?
            } else {
                rule.target.clone()
            };
            Address::from_str(&expected).map_err(|_| "INVALID_TARGET_ADDRESS")?;
            edge.expected = Some(expected.clone());
            if rule.check_before {
                edge.before = Some(observed(account(before, &rule.account)?, rule, true)?);
            }
            edge.after = Some(observed(account(after, &rule.account)?, rule, false)?);
            let target_required = matches!(rule.kind, Kind::Reference | Kind::Parent | Kind::Mint);
            if target_required {
                for accounts in [if rule.check_before { before } else { after }, after] {
                    let matches: Vec<_> =
                        accounts.iter().filter(|a| a.address == expected).collect();
                    if matches.len() > 1 {
                        return Err("AMBIGUOUS_TARGET");
                    }
                    if !matches.first().is_some_and(|a| a.exists) {
                        return Err("REFERENCED_ACCOUNT_MISSING");
                    }
                }
            }
            let preserved = !rule.preserve || edge.before == edge.after;
            if edge.after.as_ref() != Some(&expected)
                || !preserved
                || rule.check_before && edge.before.as_ref() != Some(&expected)
            {
                edge.status = CheckStatus::Fail;
                edge.code = Some("RELATIONSHIP_CHANGED".into());
                edge.message =
                    "Relationship differs from its declared target or was not preserved".into();
            }
            Ok(())
        })();
        if let Err(code) = result {
            edge.status = if matches!(code, "REFERENCED_ACCOUNT_MISSING" | "ACCOUNT_ABSENT") {
                CheckStatus::Fail
            } else {
                CheckStatus::Error
            };
            edge.code = Some(code.into());
            edge.message = code.into();
        }
        match edge.status {
            CheckStatus::Pass => report.passed += 1,
            CheckStatus::Fail => report.failed += 1,
            _ => report.errors += 1,
        }
        report.edges.push(edge);
    }
    Ok(report)
}
