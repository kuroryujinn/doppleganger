use crate::{fail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Equal,
    NotEqual,
    Initialized,
    Default,
    GreaterOrEqual,
    LessOrEqual,
    Exists,
    NotExists,
    OwnerPreserved,
    LamportsPreserved,
    AccountSizeMinimum,
    PdaPreserved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreservationRule {
    pub id: Option<String>,
    pub account: String,
    pub field: Option<String>,
    pub rule: RuleKind,
    pub value: Option<toml::Value>,
}

pub fn valid_field_path(path: &str) -> bool {
    path.split('.').all(|part| {
        let mut chars = part.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

pub fn validate_rules(rules: &[PreservationRule]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for (index, rule) in rules.iter().enumerate() {
        let context = format!("preservation[{index}]");
        if let Some(id) = &rule.id {
            if id.trim().is_empty() || !ids.insert(id) {
                return Err(fail(&context, "empty or duplicate explicit rule ID"));
            }
        }
        if rule.account.trim().is_empty()
            || rule.account.starts_with('$')
            || rule.account.chars().any(char::is_whitespace)
        {
            return Err(fail(&context, "account must be a logical observation name"));
        }
        if rule
            .field
            .as_ref()
            .is_some_and(|path| !valid_field_path(path))
        {
            return Err(fail(&context, "invalid field path"));
        }
        use RuleKind::*;
        let needs_field = matches!(
            rule.rule,
            Equal | NotEqual | Initialized | Default | GreaterOrEqual | LessOrEqual
        );
        let forbids_field = matches!(
            rule.rule,
            OwnerPreserved | LamportsPreserved | AccountSizeMinimum | PdaPreserved
        );
        if needs_field && rule.field.is_none() || forbids_field && rule.field.is_some() {
            return Err(fail(&context, "field is missing or incompatible with rule"));
        }
        let needs_value = matches!(
            rule.rule,
            Default | GreaterOrEqual | LessOrEqual | AccountSizeMinimum
        );
        if needs_value != rule.value.is_some() {
            return Err(fail(
                &context,
                "expected value is missing or unexpected for rule",
            ));
        }
    }
    Ok(())
}
