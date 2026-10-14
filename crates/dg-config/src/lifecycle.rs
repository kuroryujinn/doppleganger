use crate::{fail, MigrationDefinition, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioType {
    StandardMigration,
    UnauthorizedMigration,
    RepeatMigration,
    FailedMigrationRollback,
    UnmigratedStateAgainstV2,
    PostMigrationInstruction,
    ZeroValueState,
    BoundaryValueState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleScenario {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub scenario_type: ScenarioType,
    pub expected: Option<Outcome>,
    pub first: Option<Outcome>,
    #[serde(alias = "expected_second")]
    pub second: Option<Outcome>,
    /// Exact custom/Anchor numeric error code, never a log substring.
    pub expected_error: Option<u32>,
    pub signer_account: Option<String>,
    pub signer_override: Option<String>,
    pub fixture: Option<String>,
    pub instruction: Option<String>,
    #[serde(default)]
    pub account_mapping: BTreeMap<String, String>,
    #[serde(default, alias = "args")]
    pub arguments: BTreeMap<String, toml::Value>,
    pub migration: Option<MigrationDefinition>,
    #[serde(default)]
    pub state_unchanged: bool,
}

pub fn validate_scenarios(scenarios: &[LifecycleScenario]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for (index, s) in scenarios.iter().enumerate() {
        let context = format!("scenario[{index}]");
        let reject = |message| Err(fail(&context, message));
        if let Some(id) = &s.id {
            if id.trim().is_empty() || !ids.insert(id) {
                return reject("empty or duplicate scenario ID");
            }
        }
        use ScenarioType::*;
        if s.scenario_type == RepeatMigration {
            if s.first.is_none() || s.second.is_none() || s.expected.is_some() {
                return reject("repeat requires first and second outcomes, not expected");
            }
        } else if s.expected.is_none() || s.first.is_some() || s.second.is_some() {
            return reject("scenario requires expected outcome; first/second are repeat-only");
        }
        if s.expected_error.is_some() && s.expected.or(s.second) != Some(Outcome::Error) {
            return reject("expected_error requires expected rejection");
        }
        if s.scenario_type == FailedMigrationRollback && s.expected != Some(Outcome::Error) {
            return reject("rollback requires expected='error'");
        }
        let operation = matches!(
            s.scenario_type,
            UnmigratedStateAgainstV2 | PostMigrationInstruction
        );
        if operation {
            if s.instruction.as_ref().is_none_or(|n| n.trim().is_empty()) {
                return reject("operation requires instruction");
            }
        } else if s.instruction.is_some()
            || !s.account_mapping.is_empty()
            || !s.arguments.is_empty()
        {
            return reject("instruction/account_mapping/arguments are operation-only; use migration for migration overrides");
        }
        if s.scenario_type == UnauthorizedMigration {
            if s.signer_account
                .as_ref()
                .is_none_or(|n| n.trim().is_empty())
                || s.signer_override
                    .as_ref()
                    .is_none_or(|n| n.trim().is_empty())
            {
                return reject("unauthorized scenario requires signer_account and signer_override");
            }
        } else if s.signer_account.is_some() || s.signer_override.is_some() {
            return reject("signer override is unauthorized-only");
        }
        if matches!(s.scenario_type, ZeroValueState | BoundaryValueState) && s.fixture.is_none() {
            return reject("boundary scenario requires named fixture");
        }
        if s.fixture.as_ref().is_some_and(|n| n.trim().is_empty()) {
            return reject("empty fixture reference");
        }
        if s.scenario_type == UnmigratedStateAgainstV2 && s.migration.is_some() {
            return reject("unmigrated scenario cannot override a skipped migration");
        }
    }
    Ok(())
}
