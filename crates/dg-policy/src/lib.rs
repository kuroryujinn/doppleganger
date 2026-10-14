//! Pure relationship and upgrade-policy evaluation. No VM or network operations.
pub mod relationships;
use dg_analysis::diff::Finding;
use relationships::Relationship;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub relationship: Vec<Relationship>,
    #[serde(default)]
    pub expected_change: Vec<ExpectedChange>,
}
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StaticAction {
    #[default]
    Fail,
    Warn,
    Ignore,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub unexpected_static: StaticAction,
    #[serde(default = "yes")]
    pub require_preservation: bool,
    #[serde(default = "yes")]
    pub require_relationships: bool,
    #[serde(default = "yes")]
    pub require_lifecycle: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            unexpected_static: StaticAction::Fail,
            require_preservation: true,
            require_relationships: true,
            require_lifecycle: true,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedChange {
    pub classification: String,
    pub path: String,
    pub before_json: String,
    pub after_json: String,
    pub reason: String,
}
impl Profile {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 4 * 1024 * 1024 {
            return Err("policy exceeds 4 MiB limit".into());
        }
        let result: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if result.schema_version != 1 {
            return Err("unsupported policy schema_version (expected 1)".into());
        }
        relationships::validate(&result.relationship)?;
        let mut selectors = BTreeSet::new();
        for change in &result.expected_change {
            if change.classification.trim().is_empty()
                || change.path.trim().is_empty()
                || change.reason.trim().is_empty()
            {
                return Err("expected changes need classification, path, and reason".into());
            }
            if !selectors.insert((
                &change.classification,
                &change.path,
                &change.before_json,
                &change.after_json,
            )) {
                return Err("duplicate expected change".into());
            }
            serde_json::from_str::<Value>(&change.before_json)
                .map_err(|e| format!("before_json: {e}"))?;
            serde_json::from_str::<Value>(&change.after_json)
                .map_err(|e| format!("after_json: {e}"))?;
        }
        Ok(result)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckStatus {
    Pass,
    Fail,
    Error,
    NotApplicable,
}
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub category: String,
    pub status: CheckStatus,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Coverage {
    pub preservation: usize,
    pub relationships: usize,
    pub lifecycle: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct Decision {
    pub id: String,
    pub category: String,
    pub action: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct PolicyReport {
    pub schema_version: u32,
    pub policy_evaluated: bool,
    pub passed: bool,
    pub expected: usize,
    pub warnings: usize,
    pub ignored: usize,
    pub blocking: usize,
    pub evaluation_errors: usize,
    pub coverage: Coverage,
    pub decisions: Vec<Decision>,
}
/// Runtime violations and evaluation errors cannot be hidden by static-change policy.
pub fn evaluate(
    profile: &Profile,
    findings: &[Finding],
    checks: &[Check],
    coverage: Coverage,
) -> PolicyReport {
    let mut report = PolicyReport {
        schema_version: 1,
        policy_evaluated: true,
        passed: true,
        expected: 0,
        warnings: 0,
        ignored: 0,
        blocking: 0,
        evaluation_errors: 0,
        coverage,
        decisions: vec![],
    };
    let mut used = BTreeSet::new();
    for finding in findings {
        let matched = profile.expected_change.iter().enumerate().find(|(_, c)| {
            c.classification == finding.classification
                && c.path == finding.path
                && serde_json::from_str::<Value>(&c.before_json).ok().as_ref()
                    == Some(&finding.before)
                && serde_json::from_str::<Value>(&c.after_json).ok().as_ref()
                    == Some(&finding.after)
        });
        let (action, message) = if let Some((index, change)) = matched {
            used.insert(index);
            report.expected += 1;
            ("expected", change.reason.clone())
        } else {
            match profile.policy.unexpected_static {
                StaticAction::Fail => {
                    report.blocking += 1;
                    ("fail", finding.explanation.clone())
                }
                StaticAction::Warn => {
                    report.warnings += 1;
                    ("warn", finding.explanation.clone())
                }
                StaticAction::Ignore => {
                    report.ignored += 1;
                    ("ignore", finding.explanation.clone())
                }
            }
        };
        report.decisions.push(Decision {
            id: finding.id.clone(),
            category: "STATIC".into(),
            action: action.into(),
            message,
        });
    }
    for (index, change) in profile.expected_change.iter().enumerate() {
        if !used.contains(&index) {
            report.blocking += 1;
            report.decisions.push(Decision {
                id: format!("EXPECTED-{index}"),
                category: "POLICY".into(),
                action: "fail".into(),
                message: format!(
                    "Declared change was not observed: {} {}",
                    change.classification, change.path
                ),
            });
        }
    }
    for (required, total, name) in [
        (
            profile.policy.require_preservation,
            report.coverage.preservation,
            "PRESERVATION",
        ),
        (
            profile.policy.require_relationships,
            report.coverage.relationships,
            "RELATIONSHIP",
        ),
        (
            profile.policy.require_lifecycle,
            report.coverage.lifecycle,
            "LIFECYCLE",
        ),
    ] {
        if required && total == 0 {
            report.blocking += 1;
            report.decisions.push(Decision {
                id: format!("COVERAGE-{name}"),
                category: "COVERAGE".into(),
                action: "fail".into(),
                message: format!("Required {name} coverage is empty"),
            });
        }
    }
    for check in checks {
        let action = match check.status {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail | CheckStatus::NotApplicable => {
                report.blocking += 1;
                "fail"
            }
            CheckStatus::Error => {
                report.blocking += 1;
                report.evaluation_errors += 1;
                "error"
            }
        };
        report.decisions.push(Decision {
            id: check.id.clone(),
            category: check.category.clone(),
            action: action.into(),
            message: check.message.clone(),
        });
    }
    report.passed = report.blocking == 0;
    report
}
