//! Fixed lifecycle recipes composed from R5/R6. No instruction semantics or policy.
use crate::{
    migration::{AccountEvidence, MigrationError, ProgramSwapRecord},
    ExecutionErrorKind, ExecutionRecord, MigrationExecution, MigrationExecutor,
    RuntimeWorldBuilder,
};
use dg_config::{
    lifecycle::{validate_scenarios, LifecycleScenario, Outcome, ScenarioType},
    ConfigError, MigrationDefinition, ProjectManifest,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use solana_instruction_error::InstructionError;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScenarioStatus {
    Pass,
    Fail,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    BuildWorld,
    LoadCandidate,
    Migrate,
    ExecuteInstruction,
}
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub context: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ScenarioStepRecord {
    pub index: usize,
    pub operation: Operation,
    pub status: ScenarioStatus,
    pub expected: Option<Outcome>,
    pub expected_error: Option<u32>,
    pub state_unchanged: bool,
    pub observed: Option<Outcome>,
    pub execution: Option<ExecutionRecord>,
    pub setup_actions: Vec<ExecutionRecord>,
    pub program_swap: Option<ProgramSwapRecord>,
    pub before: Vec<AccountEvidence>,
    pub after: Vec<AccountEvidence>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ScenarioResult {
    pub id: String,
    pub definition: LifecycleScenario,
    pub status: ScenarioStatus,
    pub steps: Vec<ScenarioStepRecord>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Clone, Debug, Serialize)]
pub struct LifecycleReport {
    pub schema_version: u32,
    pub project: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub lifecycle_evaluated: bool,
    pub preservation_evaluated: bool,
    pub policy_evaluated: bool,
    pub scenarios: Vec<ScenarioResult>,
}

fn diagnostic(code: &str, context: impl ToString, message: impl ToString) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        context: context.to_string(),
        message: message.to_string(),
    }
}
fn step(index: usize, operation: Operation) -> ScenarioStepRecord {
    ScenarioStepRecord {
        index,
        operation,
        status: ScenarioStatus::Pass,
        expected: None,
        expected_error: None,
        state_unchanged: false,
        observed: None,
        execution: None,
        setup_actions: vec![],
        program_swap: None,
        before: vec![],
        after: vec![],
        diagnostics: vec![],
    }
}
fn normalize(scenarios: &[LifecycleScenario]) -> Result<Vec<LifecycleScenario>, ConfigError> {
    validate_scenarios(scenarios)?;
    let mut ids = BTreeSet::new();
    scenarios
        .iter()
        .map(|s| {
            let mut s = s.clone();
            if s.id.is_none() {
                let bytes = serde_json::to_vec(&s).map_err(|e| ConfigError {
                    field: "scenario".into(),
                    message: e.to_string(),
                })?;
                s.id = Some(format!("scenario-{:x}", Sha256::digest(bytes)));
            }
            if !ids.insert(s.id.clone()) {
                return Err(ConfigError {
                    field: "scenario.id".into(),
                    message: "duplicate normalized scenario ID".into(),
                });
            }
            Ok(s)
        })
        .collect()
}

/// Raw comparison only. Fee payers are not implicitly exempted from selected targets.
pub fn changed_accounts(before: &[AccountEvidence], after: &[AccountEvidence]) -> Vec<String> {
    let b: BTreeMap<_, _> = before.iter().map(|a| (&a.name, a)).collect();
    let a: BTreeMap<_, _> = after.iter().map(|a| (&a.name, a)).collect();
    b.keys()
        .chain(a.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| match (b.get(name), a.get(name)) {
            (Some(b), Some(a)) => b.address != a.address || b.snapshot != a.snapshot,
            _ => true,
        })
        .cloned()
        .collect()
}

fn absorb(step: &mut ScenarioStepRecord, evidence: MigrationExecution) {
    step.before = evidence.before_accounts;
    step.after = evidence.after_accounts;
    step.execution = evidence.transaction;
    step.program_swap = evidence.program_swap;
}
fn migration_error(step: &mut ScenarioStepRecord, err: MigrationError) {
    step.status = ScenarioStatus::Error;
    step.diagnostics
        .push(diagnostic(&err.code, &err.context, &err.message));
    if let Some(evidence) = err.evidence {
        absorb(step, evidence);
    }
}
fn evaluate_step(
    step: &mut ScenarioStepRecord,
    expected: Outcome,
    expected_error: Option<u32>,
    unchanged: bool,
) {
    step.expected = Some(expected);
    step.expected_error = expected_error;
    step.state_unchanged = unchanged;
    if step.status == ScenarioStatus::Error {
        return;
    }
    let Some(record) = &step.execution else {
        step.status = ScenarioStatus::Error;
        step.diagnostics.push(diagnostic(
            "MISSING_EXECUTION",
            "transaction",
            "no transaction evidence",
        ));
        return;
    };
    let observed = if record.success {
        Some(Outcome::Success)
    } else if record.error_kind == Some(ExecutionErrorKind::Program) {
        Some(Outcome::Error)
    } else {
        None
    };
    step.observed = observed;
    if observed.is_none() {
        step.status = ScenarioStatus::Error;
        step.diagnostics.push(diagnostic(
            "ENGINE_FAILURE",
            &record.instruction_name,
            record
                .error
                .as_deref()
                .unwrap_or("non-program transaction failure"),
        ));
        return;
    }
    if observed != Some(expected) {
        step.status = ScenarioStatus::Fail;
        step.diagnostics.push(diagnostic(
            "OUTCOME_MISMATCH",
            &record.instruction_name,
            format!("expected {expected:?}, observed {observed:?}"),
        ));
    }
    if let Some(code) = expected_error {
        if !matches!(&record.transaction_error,Some(TransactionError::InstructionError(_,InstructionError::Custom(actual))) if *actual==code)
        {
            step.status = ScenarioStatus::Fail;
            step.diagnostics.push(diagnostic(
                "ERROR_MISMATCH",
                &record.instruction_name,
                format!(
                    "expected custom code {code}, observed {:?}",
                    record.transaction_error
                ),
            ));
        }
    }
    if unchanged {
        if step.before.is_empty() || step.after.is_empty() {
            step.status = ScenarioStatus::Error;
            step.diagnostics.push(diagnostic(
                "OBSERVATION_TARGETS",
                "state_unchanged",
                "at least one observation target is required",
            ));
            return;
        }
        for name in changed_accounts(&step.before, &step.after) {
            step.status = ScenarioStatus::Fail;
            step.diagnostics.push(diagnostic(
                "STATE_CHANGED",
                name,
                "selected account raw state differs",
            ));
        }
    }
}

#[derive(Default)]
pub struct LifecycleEngine;
impl LifecycleEngine {
    pub fn run(
        &self,
        manifest: &ProjectManifest,
        base: &Path,
    ) -> Result<LifecycleReport, ConfigError> {
        self.run_with_world_builder(manifest, base, |m, b| {
            RuntimeWorldBuilder::default().build_manifest(m, b)
        })
    }

    /// Supply a fresh initial world for every scenario; recipes and verdicts are unchanged.
    pub fn run_with_world_builder(
        &self,
        manifest: &ProjectManifest,
        base: &Path,
        build: impl Fn(&ProjectManifest, &Path) -> crate::Result<crate::WorldBuildResult>,
    ) -> Result<LifecycleReport, ConfigError> {
        let scenarios = normalize(&manifest.scenario)?;
        let mut report = LifecycleReport {
            schema_version: 1,
            project: manifest.project.name.clone(),
            total: scenarios.len(),
            passed: 0,
            failed: 0,
            errors: 0,
            lifecycle_evaluated: true,
            preservation_evaluated: false,
            policy_evaluated: false,
            scenarios: vec![],
        };
        for definition in scenarios {
            let mut result = ScenarioResult {
                id: definition.id.clone().unwrap_or_default(),
                definition: definition.clone(),
                status: ScenarioStatus::Pass,
                steps: vec![],
                diagnostics: vec![],
            };
            if let Err(error) =
                self.run_scenario(manifest, base, &definition, &mut result.steps, &build)
            {
                result.diagnostics.push(error);
                result.status = ScenarioStatus::Error;
            }
            for s in &result.steps {
                if s.status == ScenarioStatus::Error {
                    result.status = ScenarioStatus::Error;
                } else if s.status == ScenarioStatus::Fail && result.status != ScenarioStatus::Error
                {
                    result.status = ScenarioStatus::Fail;
                }
            }
            match result.status {
                ScenarioStatus::Pass => report.passed += 1,
                ScenarioStatus::Fail => report.failed += 1,
                ScenarioStatus::Error => report.errors += 1,
            }
            report.scenarios.push(result);
        }
        Ok(report)
    }

    fn run_scenario(
        &self,
        manifest: &ProjectManifest,
        base: &Path,
        s: &LifecycleScenario,
        steps: &mut Vec<ScenarioStepRecord>,
        build: &impl Fn(&ProjectManifest, &Path) -> crate::Result<crate::WorldBuildResult>,
    ) -> Result<(), Diagnostic> {
        use ScenarioType::*;
        let mut local = manifest.clone();
        if let Some(name) = &s.fixture {
            local.fixture = Some(
                manifest
                    .fixtures
                    .get(name)
                    .ok_or_else(|| diagnostic("FIXTURE_REFERENCE", name, "unknown named fixture"))?
                    .clone(),
            );
        }
        if let Some(migration) = &s.migration {
            local.migration = migration.clone();
        }
        let mut build_step = step(0, Operation::BuildWorld);
        let built = build(&local, base);
        let mut world = match built {
            Ok(built) => {
                build_step.setup_actions = built.executed_actions;
                built.world
            }
            Err(err) => {
                build_step.status = ScenarioStatus::Error;
                build_step.setup_actions = err.executed_actions;
                build_step.after = err
                    .observed_accounts
                    .into_iter()
                    .map(|a| AccountEvidence {
                        name: a.binding.name,
                        address: a.snapshot.address.clone(),
                        account_type: Some(a.binding.account_type),
                        snapshot: Some(a.snapshot),
                        decoded: a.decoded,
                        decode_error: a.decode_error,
                    })
                    .collect();
                build_step
                    .diagnostics
                    .push(diagnostic(&err.code, &err.context, &err.message));
                steps.push(build_step);
                return Ok(());
            }
        };
        let executor = MigrationExecutor::default();
        let targets = MigrationExecutor::targets(&local);
        match executor.snapshots(&world, &targets, false) {
            Ok(after) => build_step.after = after,
            Err(err) => {
                migration_error(&mut build_step, *err);
                steps.push(build_step);
                return Ok(());
            }
        }
        steps.push(build_step);
        let payer = local.fixture.as_ref().and_then(|f| f.payer.as_deref());
        if s.scenario_type == UnauthorizedMigration {
            let name = s
                .signer_override
                .as_deref()
                .unwrap_or_default()
                .trim_start_matches('$');
            let public = world.resolved.addresses.get(name).ok_or_else(|| {
                diagnostic("SIGNER_OVERRIDE", name, "unresolved configured signer")
            })?;
            if !world
                .resolved
                .signers
                .values()
                .any(|key| key.pubkey() == *public)
            {
                return Err(diagnostic(
                    "MISSING_SIGNER",
                    name,
                    "override requires an existing test keypair",
                ));
            }
            let (_, model) = executor
                .candidate(&local, base)
                .map_err(|e| diagnostic(&e.code, &e.context, &e.message))?;
            let account = s.signer_account.as_deref().unwrap_or_default();
            if !model
                .instructions
                .get(&local.migration.instruction)
                .is_some_and(|i| i.accounts.iter().any(|a| a.name == account && a.signer))
            {
                return Err(diagnostic(
                    "SIGNER_ACCOUNT",
                    account,
                    "override must name a required IDL signer account",
                ));
            }
            local
                .migration
                .account_mapping
                .insert(account.into(), name.into());
        }
        if s.scenario_type == UnmigratedStateAgainstV2 {
            let mut load = step(steps.len(), Operation::LoadCandidate);
            load.before = steps[0].after.clone();
            match executor
                .candidate(&local, base)
                .and_then(|(artifact, model)| {
                    executor.load_candidate(&mut world, &artifact, &model)
                }) {
                Ok(swap) => load.program_swap = Some(swap),
                Err(err) => {
                    migration_error(&mut load, *err);
                    steps.push(load);
                    return Ok(());
                }
            }
            match executor.snapshots(&world, &targets, true) {
                Ok(after) => load.after = after,
                Err(err) => {
                    migration_error(&mut load, *err);
                    steps.push(load);
                    return Ok(());
                }
            }
            steps.push(load);
        } else {
            let mut first = step(steps.len(), Operation::Migrate);
            match executor.execute_manifest(&mut world, &local, base) {
                Ok(evidence) => absorb(&mut first, evidence),
                Err(err) => migration_error(&mut first, *err),
            }
            let expected = if s.scenario_type == PostMigrationInstruction {
                Outcome::Success
            } else {
                s.first.or(s.expected).unwrap_or(Outcome::Success)
            };
            let final_step = !matches!(s.scenario_type, RepeatMigration | PostMigrationInstruction);
            evaluate_step(
                &mut first,
                expected,
                if final_step { s.expected_error } else { None },
                final_step && (s.state_unchanged || s.scenario_type == FailedMigrationRollback),
            );
            let proceed = first.status == ScenarioStatus::Pass;
            steps.push(first);
            if !proceed || final_step {
                return Ok(());
            }
        }
        let (operation, definition, expected) = if s.scenario_type == RepeatMigration {
            (
                Operation::Migrate,
                local.migration.clone(),
                s.second.unwrap_or(Outcome::Error),
            )
        } else {
            let mut mappings = if s.account_mapping.is_empty() {
                local.migration.account_mapping.clone()
            } else {
                s.account_mapping.clone()
            };
            for value in mappings.values_mut() {
                if let Some(name) = value.strip_prefix('$') {
                    *value = name.to_owned();
                }
            }
            (
                Operation::ExecuteInstruction,
                MigrationDefinition {
                    instruction: s.instruction.clone().unwrap_or_default(),
                    account_mapping: mappings,
                    arguments: s.arguments.clone(),
                },
                s.expected.unwrap_or(Outcome::Success),
            )
        };
        let mut last = step(steps.len(), operation);
        match executor.execute_loaded(
            &mut world,
            &local.project.name,
            &definition,
            &targets,
            payer,
        ) {
            Ok(evidence) => absorb(&mut last, evidence),
            Err(err) => migration_error(&mut last, *err),
        }
        evaluate_step(
            &mut last,
            expected,
            s.expected_error,
            s.state_unchanged || s.scenario_type == RepeatMigration && expected == Outcome::Error,
        );
        steps.push(last);
        Ok(())
    }
}
