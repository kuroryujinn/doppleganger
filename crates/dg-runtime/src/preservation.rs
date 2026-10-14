//! Pure evaluation of configured rules over R6 evidence; no transactions or policy.
use crate::migration::{AccountEvidence, MigrationExecution};
use dg_analysis::model::{FieldModel, FieldType, ProgramModel, TypeModel};
use dg_config::{
    preservation::{valid_field_path, validate_rules, PreservationRule, RuleKind},
    ConfigError,
};
use dg_state::{DecodeLimits, StateError, Value};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum Resolution {
    Found(Value),
    Missing,
    AccountAbsent,
    TypeMismatch,
    InvalidPath,
    Undecodable(Option<StateError>),
}

pub fn resolve_field(account: &AccountEvidence, path: &str) -> Resolution {
    if !valid_field_path(path) {
        return Resolution::InvalidPath;
    }
    if account.snapshot.is_none() {
        return Resolution::AccountAbsent;
    }
    let Some(decoded) = &account.decoded else {
        return Resolution::Undecodable(account.decode_error.clone());
    };
    if account.decode_error.is_some() {
        return Resolution::Undecodable(account.decode_error.clone());
    }
    let mut fields = &decoded.fields;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let Some(value) = fields.get(part) else {
            return Resolution::Missing;
        };
        if parts.peek().is_none() {
            return Resolution::Found(value.clone());
        }
        match value {
            Value::Struct(nested) => fields = nested,
            _ => return Resolution::TypeMismatch,
        }
    }
    Resolution::InvalidPath
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Pass,
    Fail,
    Error,
    NotApplicable,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleEvaluation {
    pub rule_id: String,
    pub rule: RuleKind,
    pub account: String,
    pub field: Option<String>,
    pub status: Status,
    pub category: Option<String>,
    pub error_code: Option<String>,
    pub expected: Option<Value>,
    pub before: Option<Resolution>,
    pub after: Option<Resolution>,
    pub before_present: Option<bool>,
    pub after_present: Option<bool>,
    pub explanation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreservationFinding {
    pub id: String,
    #[serde(flatten)]
    pub evaluation: RuleEvaluation,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreservationReport {
    pub schema_version: u32,
    pub project: String,
    pub migration_signature: Option<String>,
    pub migration_success: Option<bool>,
    pub preservation_evaluated: bool,
    pub lifecycle_evaluated: bool,
    pub policy_evaluated: bool,
    pub rules: Vec<PreservationRule>,
    pub rules_total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub not_applicable: usize,
    pub evaluations: Vec<RuleEvaluation>,
    pub findings: Vec<PreservationFinding>,
}

fn config_error(message: impl Into<String>) -> ConfigError {
    ConfigError {
        field: "preservation".into(),
        message: message.into(),
    }
}

fn normalized_rules(rules: &[PreservationRule]) -> Result<Vec<PreservationRule>, ConfigError> {
    validate_rules(rules)?;
    let mut ids = BTreeSet::new();
    let mut result = Vec::new();
    for rule in rules {
        let mut rule = rule.clone();
        if rule.id.is_none() {
            let bytes = serde_json::to_vec(&rule).map_err(|e| config_error(e.to_string()))?;
            rule.id = Some(format!("rule-{:x}", Sha256::digest(bytes)));
        }
        if !ids.insert(rule.id.clone()) {
            return Err(config_error(
                "duplicate normalized rule ID; give repeated rules distinct explicit IDs",
            ));
        }
        result.push(rule);
    }
    Ok(result)
}

fn target<'a>(
    accounts: &'a [AccountEvidence],
    name: &str,
) -> Result<&'a AccountEvidence, &'static str> {
    let mut matches = accounts.iter().filter(|a| a.name == name);
    let first = matches.next().ok_or("TARGET_MISSING")?;
    if matches.next().is_some() {
        return Err("AMBIGUOUS_ACCOUNT");
    }
    Ok(first)
}

fn field_type<'a>(
    model: &'a ProgramModel,
    account: &str,
    path: &str,
) -> Result<&'a FieldType, &'static str> {
    let mut fields: &[FieldModel] = &model.accounts.get(account).ok_or("UNKNOWN_SCHEMA")?.fields;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let ty = &fields
            .iter()
            .find(|f| f.name == part)
            .ok_or("FIELD_MISSING")?
            .ty;
        if parts.peek().is_none() {
            return Ok(ty);
        }
        let mut current = ty;
        let mut depth = 0;
        loop {
            depth += 1;
            if depth > 64 {
                return Err("TYPE_DEPTH");
            }
            match current {
                FieldType::Defined(name) => match model.types.get(name) {
                    Some(TypeModel::Struct { fields: nested }) => {
                        fields = nested;
                        break;
                    }
                    Some(TypeModel::Alias { value }) => current = value,
                    _ => return Err("TYPE_MISMATCH"),
                },
                _ => return Err("TYPE_MISMATCH"),
            }
        }
    }
    Err("INVALID_FIELD_PATH")
}

fn resolution_error(value: &Resolution) -> Option<&'static str> {
    match value {
        Resolution::Found(_) => None,
        Resolution::Missing => Some("FIELD_MISSING"),
        Resolution::AccountAbsent => Some("ACCOUNT_MISSING"),
        Resolution::TypeMismatch => Some("TYPE_MISMATCH"),
        Resolution::InvalidPath => Some("INVALID_FIELD_PATH"),
        Resolution::Undecodable(_) => Some("UNDECODABLE"),
    }
}

impl RuleEvaluation {
    fn error(&mut self, code: &str, message: impl Into<String>) {
        self.status = Status::Error;
        self.category = Some("RULE_EVALUATION_ERROR".into());
        self.error_code = Some(code.into());
        self.explanation = message.into();
    }
    fn condition(&mut self, passed: bool, category: &str) {
        self.status = if passed { Status::Pass } else { Status::Fail };
        self.category = (!passed).then(|| category.into());
        self.explanation = if passed {
            "Configured condition satisfied."
        } else {
            "Configured condition violated; see before, after and expected evidence."
        }
        .into();
    }
}

/// The V2 model supplies expected-value types; values use the existing Borsh codec.
/// R6 evidence is borrowed and never mutated. All independent rules are evaluated.
pub fn evaluate(
    execution: &MigrationExecution,
    rules: &[PreservationRule],
    v2: &ProgramModel,
) -> Result<PreservationReport, ConfigError> {
    let rules = normalized_rules(rules)?;
    let mut report = PreservationReport {
        schema_version: 1,
        project: execution.project.clone(),
        migration_signature: execution
            .transaction
            .as_ref()
            .and_then(|t| t.signature.clone()),
        migration_success: execution.transaction.as_ref().map(|t| t.success),
        preservation_evaluated: true,
        lifecycle_evaluated: false,
        policy_evaluated: false,
        rules_total: rules.len(),
        rules: rules.clone(),
        passed: 0,
        failed: 0,
        errors: 0,
        not_applicable: 0,
        evaluations: vec![],
        findings: vec![],
    };
    for rule in &rules {
        let mut evaluation = RuleEvaluation {
            rule_id: rule.id.clone().unwrap_or_default(),
            rule: rule.rule,
            account: rule.account.clone(),
            field: rule.field.clone(),
            status: Status::Error,
            category: None,
            error_code: None,
            expected: None,
            before: None,
            after: None,
            before_present: None,
            after_present: None,
            explanation: String::new(),
        };
        let before = target(&execution.before_accounts, &rule.account);
        let after = target(&execution.after_accounts, &rule.account);
        match (before, after) {
            (Ok(before), Ok(after)) => evaluate_rule(&mut evaluation, rule, before, after, v2),
            (Err(code), _) | (_, Err(code)) => {
                let unknown = !execution
                    .before_accounts
                    .iter()
                    .chain(&execution.after_accounts)
                    .any(|a| a.name == rule.account);
                evaluation.error(
                    if unknown { "UNKNOWN_ACCOUNT" } else { code },
                    "Logical account is missing or ambiguous in execution evidence.",
                );
            }
        }
        match evaluation.status {
            Status::Pass => report.passed += 1,
            Status::Fail => report.failed += 1,
            Status::Error => report.errors += 1,
            Status::NotApplicable => report.not_applicable += 1,
        }
        if matches!(evaluation.status, Status::Fail | Status::Error) {
            report.findings.push(PreservationFinding {
                id: format!("PRESERVE-{}", evaluation.rule_id),
                evaluation: evaluation.clone(),
            });
        }
        report.evaluations.push(evaluation);
    }
    Ok(report)
}

fn evaluate_rule(
    out: &mut RuleEvaluation,
    rule: &PreservationRule,
    before: &AccountEvidence,
    after: &AccountEvidence,
    model: &ProgramModel,
) {
    use RuleKind::*;
    out.before_present = Some(before.snapshot.is_some());
    out.after_present = Some(after.snapshot.is_some());
    let metadata = |account: &AccountEvidence| -> Resolution {
        if rule.rule == PdaPreserved {
            return Resolution::Found(Value::Pubkey(account.address.clone()));
        }
        let Some(raw) = &account.snapshot else {
            return Resolution::AccountAbsent;
        };
        Resolution::Found(match rule.rule {
            OwnerPreserved => Value::Pubkey(raw.owner.clone()),
            LamportsPreserved => Value::Integer(raw.lamports.clone()),
            AccountSizeMinimum => Value::Integer(raw.data_length.into()),
            _ => Value::Bool(true),
        })
    };
    let b = rule
        .field
        .as_ref()
        .map(|p| resolve_field(before, p))
        .unwrap_or_else(|| metadata(before));
    let a = rule
        .field
        .as_ref()
        .map(|p| resolve_field(after, p))
        .unwrap_or_else(|| metadata(after));
    out.before = Some(b.clone());
    out.after = Some(a.clone());

    // Presence tests distinguish a missing field from an undecodable or absent account.
    if matches!(rule.rule, Exists | NotExists) {
        let exists = if rule.field.is_none() {
            after.snapshot.is_some()
        } else {
            match &a {
                Resolution::Found(_) => true,
                Resolution::Missing => false,
                other => {
                    out.error(
                        resolution_error(other).unwrap_or("RESOLUTION"),
                        "Field existence cannot be determined.",
                    );
                    return;
                }
            }
        };
        let category = match (rule.rule, rule.field.is_some()) {
            (Exists, true) => "FIELD_MISSING",
            (Exists, false) => "ACCOUNT_MISSING",
            (_, true) => "FIELD_UNEXPECTEDLY_EXISTS",
            (_, false) => "ACCOUNT_UNEXPECTEDLY_EXISTS",
        };
        out.condition(if rule.rule == Exists { exists } else { !exists }, category);
        return;
    }
    if rule.rule == Initialized {
        if matches!(
            a,
            Resolution::Undecodable(_) | Resolution::TypeMismatch | Resolution::InvalidPath
        ) {
            out.error(
                resolution_error(&a).unwrap_or("RESOLUTION"),
                "AFTER field availability is unknown.",
            );
            return;
        }
        match &b {
            Resolution::Found(_) => {
                out.status = Status::NotApplicable;
                out.explanation =
                    "Field already existed in BEFORE; initialization rule is not applicable."
                        .into();
                return;
            }
            Resolution::Missing | Resolution::AccountAbsent => {}
            other => {
                out.error(
                    resolution_error(other).unwrap_or("RESOLUTION"),
                    "BEFORE field availability is unknown.",
                );
                return;
            }
        }
        match &a {
            Resolution::Found(_) => out.condition(true, "FIELD_NOT_INITIALIZED"),
            Resolution::Missing | Resolution::AccountAbsent => {
                out.condition(false, "FIELD_NOT_INITIALIZED")
            }
            other => out.error(
                resolution_error(other).unwrap_or("RESOLUTION"),
                "AFTER field availability is unknown.",
            ),
        }
        return;
    }
    let compare_before = matches!(
        rule.rule,
        Equal | NotEqual | OwnerPreserved | LamportsPreserved | PdaPreserved
    );
    if let Some(code) =
        resolution_error(&a).or_else(|| compare_before.then(|| resolution_error(&b)).flatten())
    {
        out.error(
            code,
            "Required value could not be resolved; see side-specific evidence.",
        );
        return;
    }
    if let Some(configured) = &rule.value {
        let ty = if rule.rule == AccountSizeMinimum {
            Ok(FieldType::Primitive("u64".into()))
        } else {
            after
                .account_type
                .as_deref()
                .ok_or("UNKNOWN_SCHEMA")
                .and_then(|name| {
                    field_type(model, name, rule.field.as_deref().unwrap_or_default()).cloned()
                })
        };
        let expected = ty.map_err(str::to_string).and_then(|ty| {
            let bytes = dg_config::encode(configured, &ty, &model.types, "preservation.value")
                .map_err(|e| e.to_string())?;
            dg_state::decode_value(model, &ty, &bytes, DecodeLimits::default())
                .map_err(|e| e.to_string())
        });
        match expected {
            Ok(value) => out.expected = Some(value),
            Err(message) => {
                out.error("EXPECTED_VALUE", message);
                return;
            }
        }
    }
    match rule.rule {
        Equal => out.condition(b == a, "FIELD_NOT_PRESERVED"),
        NotEqual => out.condition(b != a, "FIELD_UNEXPECTEDLY_EQUAL"),
        OwnerPreserved => out.condition(b == a, "OWNER_CHANGED"),
        LamportsPreserved => out.condition(b == a, "LAMPORTS_CHANGED"),
        PdaPreserved => out.condition(b == a, "PDA_ADDRESS_CHANGED"),
        Default => out.condition(
            Some(a) == out.expected.clone().map(Resolution::Found),
            "DEFAULT_VALUE_MISMATCH",
        ),
        GreaterOrEqual | LessOrEqual | AccountSizeMinimum => match (&a, &out.expected) {
            (Resolution::Found(Value::Integer(actual)), Some(Value::Integer(expected))) => {
                let (passed, category) = match rule.rule {
                    LessOrEqual => (actual <= expected, "VALUE_ABOVE_MAXIMUM"),
                    AccountSizeMinimum => (actual >= expected, "ACCOUNT_TOO_SMALL"),
                    _ => (actual >= expected, "VALUE_BELOW_MINIMUM"),
                };
                out.condition(passed, category);
            }
            _ => out.error(
                "TYPE_MISMATCH",
                "Ordering comparisons require integer values; no coercion is performed.",
            ),
        },
        Exists | NotExists | Initialized => unreachable!("handled above"),
    }
}
