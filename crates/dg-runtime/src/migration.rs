//! One configured migration transaction on an existing world. No safety verdicts.
use crate::{ExecutionErrorKind, ExecutionRecord, Phase, RuntimeWorld};
use dg_analysis::{model::ProgramModel, normalize};
use dg_config::{MigrationDefinition, MigrationTarget, ProjectManifest};
use dg_state::{
    raw::RawAccountSnapshot, DecodeLimits, DecodedAccount, ProgramArtifact, StateError,
    TrailingBytes,
};
use serde::Serialize;
use solana_address::Address;
use solana_signer::Signer;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
    str::FromStr,
};

#[derive(Clone, Debug, Serialize)]
pub struct AccountEvidence {
    pub name: String,
    pub address: String,
    pub account_type: Option<String>,
    /// None means absent, not a synthetic zero-filled account.
    pub snapshot: Option<RawAccountSnapshot>,
    pub decoded: Option<DecodedAccount>,
    pub decode_error: Option<StateError>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgramSwapRecord {
    pub program_id: String,
    pub previous: ProgramArtifact,
    pub candidate: ProgramArtifact,
    pub loaded: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MigrationExecution {
    pub schema_version: u32,
    pub project: String,
    pub v1_program: ProgramArtifact,
    pub v2_program: ProgramArtifact,
    pub program_swap: Option<ProgramSwapRecord>,
    pub before_accounts: Vec<AccountEvidence>,
    pub after_accounts: Vec<AccountEvidence>,
    pub transaction: Option<ExecutionRecord>,
    pub preservation_evaluated: bool,
}

#[derive(Debug, Serialize)]
pub struct MigrationError {
    pub code: String,
    pub context: String,
    pub message: String,
    pub evidence: Option<MigrationExecution>,
}
impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.context, self.message)
    }
}
impl std::error::Error for MigrationError {}
pub type Result<T> = std::result::Result<T, Box<MigrationError>>;
fn error(code: &str, context: impl ToString, message: impl ToString) -> Box<MigrationError> {
    Box::new(MigrationError {
        code: code.into(),
        context: context.to_string(),
        message: message.to_string(),
        evidence: None,
    })
}

pub struct MigrationRequest<'a> {
    pub project: &'a str,
    pub candidate: ProgramArtifact,
    pub model: &'a ProgramModel,
    pub definition: &'a MigrationDefinition,
    pub targets: &'a [MigrationTarget],
    pub payer: Option<&'a str>,
}

#[derive(Default)]
pub struct MigrationExecutor {
    pub limits: DecodeLimits,
}

impl MigrationExecutor {
    pub fn candidate(
        &self,
        manifest: &ProjectManifest,
        base: &Path,
    ) -> Result<(ProgramArtifact, ProgramModel)> {
        let version = &manifest.versions.v2;
        let artifact = ProgramArtifact {
            program_id: version.program_id.clone(),
            program_path: base.join(&version.program_path),
            idl_path: base.join(&version.idl),
        };
        let bytes = dg_analysis::read_bounded(&artifact.idl_path, 8 * 1024 * 1024)
            .map_err(|e| error("IDL", artifact.idl_path.display(), e))?;
        let model = normalize(&bytes).map_err(|e| error("IDL", e.path, e.message))?;
        Ok((artifact, model))
    }

    pub fn targets(manifest: &ProjectManifest) -> Vec<MigrationTarget> {
        if !manifest.migration_targets.is_empty() {
            return manifest.migration_targets.clone();
        }
        manifest
            .fixture
            .iter()
            .flat_map(|f| &f.account)
            .map(|a| MigrationTarget {
                name: a.name.clone(),
                address: a.address.clone(),
                v1_type: Some(a.account_type.clone()),
                v2_type: Some(a.account_type.clone()),
            })
            .collect()
    }

    pub fn snapshots(
        &self,
        world: &RuntimeWorld,
        targets: &[MigrationTarget],
        v2: bool,
    ) -> Result<Vec<AccountEvidence>> {
        let mut output = Vec::new();
        self.observe(world, targets, v2, &world.model, &mut output)?;
        Ok(output)
    }

    pub fn execute_manifest(
        &self,
        world: &mut RuntimeWorld,
        manifest: &ProjectManifest,
        base: &Path,
    ) -> Result<MigrationExecution> {
        if manifest.versions.v1.program_id != world.artifact.program_id {
            return Err(error(
                "PROGRAM_ID",
                "v1",
                "manifest does not match existing world",
            ));
        }
        let (candidate, model) = self.candidate(manifest, base)?;
        let targets = Self::targets(manifest);
        self.execute(
            world,
            MigrationRequest {
                project: &manifest.project.name,
                candidate,
                model: &model,
                definition: &manifest.migration,
                targets: &targets,
                payer: manifest.fixture.as_ref().and_then(|f| f.payer.as_deref()),
            },
        )
    }

    /// Stage replacement in a clone of the current VM, not a new empty world.
    /// A loader error cannot leave the caller with a partially installed candidate.
    pub fn load_candidate(
        &self,
        world: &mut RuntimeWorld,
        candidate: &ProgramArtifact,
        model: &ProgramModel,
    ) -> Result<ProgramSwapRecord> {
        if candidate.program_id != world.artifact.program_id
            || model
                .address
                .as_ref()
                .is_some_and(|id| id != &candidate.program_id)
        {
            return Err(error(
                "PROGRAM_ID",
                "v2",
                "in-place migration requires matching world, candidate and IDL program IDs",
            ));
        }
        let id =
            Address::from_str(&candidate.program_id).map_err(|e| error("PROGRAM_ID", "v2", e))?;
        let bytes =
            dg_analysis::read_bounded(&candidate.program_path, 64 * 1024 * 1024).map_err(|e| {
                error(
                    if e.kind() == std::io::ErrorKind::NotFound {
                        "MISSING_PROGRAM"
                    } else {
                        "PROGRAM_ARTIFACT"
                    },
                    candidate.program_path.display(),
                    e,
                )
            })?;
        if !bytes.starts_with(b"\x7fELF") {
            return Err(error(
                "INVALID_PROGRAM",
                candidate.program_path.display(),
                "expected an ELF binary",
            ));
        }
        let mut staged = world.runtime.clone();
        staged
            .add_program(id, &bytes)
            .map_err(|e| error("PROGRAM_LOAD", candidate.program_path.display(), e))?;
        let record = ProgramSwapRecord {
            program_id: candidate.program_id.clone(),
            previous: world.artifact.clone(),
            candidate: candidate.clone(),
            loaded: true,
        };
        world.runtime = staged;
        world.artifact = candidate.clone();
        world.model = model.clone();
        Ok(record)
    }

    pub fn execute(
        &self,
        world: &mut RuntimeWorld,
        request: MigrationRequest<'_>,
    ) -> Result<MigrationExecution> {
        self.execute_inner(world, request, true)
    }

    /// Run an IDL-configured instruction on the existing loaded candidate, without reloading it.
    /// Both sides use the candidate schema; raw state remains available on decode failure.
    pub fn execute_loaded(
        &self,
        world: &mut RuntimeWorld,
        project: &str,
        definition: &MigrationDefinition,
        targets: &[MigrationTarget],
        payer: Option<&str>,
    ) -> Result<MigrationExecution> {
        let model = world.model.clone();
        let candidate = world.artifact.clone();
        self.execute_inner(
            world,
            MigrationRequest {
                project,
                candidate,
                model: &model,
                definition,
                targets,
                payer,
            },
            false,
        )
    }

    fn execute_inner(
        &self,
        world: &mut RuntimeWorld,
        request: MigrationRequest<'_>,
        replace_candidate: bool,
    ) -> Result<MigrationExecution> {
        let mut evidence = MigrationExecution {
            schema_version: 1,
            project: request.project.into(),
            v1_program: world.artifact.clone(),
            v2_program: request.candidate.clone(),
            program_swap: None,
            before_accounts: vec![],
            after_accounts: vec![],
            transaction: None,
            preservation_evaluated: false,
        };
        let result = (|| {
            self.observe(
                world,
                request.targets,
                !replace_candidate,
                &world.model,
                &mut evidence.before_accounts,
            )?;
            let mut definition = request.definition.clone();
            let bindings = world.resolved.strings();
            for value in definition.arguments.values_mut() {
                *value = dg_config::fixture::resolve_value(value, &bindings, 0)
                    .map_err(|e| error("ARGUMENT", e.field, e.message))?;
            }
            if !request
                .model
                .instructions
                .contains_key(&definition.instruction)
            {
                return Err(error(
                    "INSTRUCTION",
                    &definition.instruction,
                    "instruction not found in V2 IDL",
                ));
            }
            let binding = definition
                .resolve(request.model, &request.candidate.program_id)
                .map_err(|e| {
                    let code = if e.field.starts_with("migration.arguments") {
                        "ARGUMENT"
                    } else if e.field.starts_with("migration.account_mapping") {
                        "ACCOUNT_BINDING"
                    } else {
                        "INSTRUCTION"
                    };
                    error(code, e.field, e.message)
                })?;
            let instruction = binding
                .build(&world.resolved.addresses)
                .map_err(|e| error("ACCOUNT_BINDING", e.field, e.message))?;
            let mut seen = BTreeMap::new();
            for meta in &instruction.accounts {
                let flags = (meta.is_signer, meta.is_writable);
                if seen
                    .insert(meta.pubkey, flags)
                    .is_some_and(|previous| previous != flags)
                {
                    return Err(error(
                        "ACCOUNT_BINDING",
                        meta.pubkey,
                        "duplicate account has incompatible privileges",
                    ));
                }
                if meta.is_signer
                    && !world
                        .resolved
                        .signers
                        .values()
                        .any(|key| key.pubkey() == meta.pubkey)
                {
                    return Err(error(
                        "MISSING_SIGNER",
                        meta.pubkey,
                        "required public key has no keypair",
                    ));
                }
            }
            if let Some(name) = request.payer {
                let payer = world
                    .resolved
                    .addresses
                    .get(name)
                    .ok_or_else(|| error("ACCOUNT_BINDING", name, "unresolved fee payer"))?;
                if !world
                    .resolved
                    .signers
                    .values()
                    .any(|key| key.pubkey() == *payer)
                {
                    return Err(error("MISSING_SIGNER", name, "fee payer has no keypair"));
                }
            }
            if replace_candidate {
                evidence.program_swap =
                    Some(self.load_candidate(world, &request.candidate, request.model)?);
            }
            let record = world.execute(
                if replace_candidate {
                    Phase::Migration
                } else {
                    Phase::Instruction
                },
                0,
                &definition.instruction,
                instruction,
                request.payer,
            );
            let kind = record.error_kind;
            evidence.transaction = Some(record);
            self.observe(
                world,
                request.targets,
                true,
                request.model,
                &mut evidence.after_accounts,
            )?;
            if matches!(
                kind,
                Some(
                    ExecutionErrorKind::Preparation
                        | ExecutionErrorKind::Submission
                        | ExecutionErrorKind::Runtime
                )
            ) {
                let record = evidence.transaction.as_ref().unwrap();
                return Err(error(
                    if kind == Some(ExecutionErrorKind::Preparation) {
                        "TRANSACTION_PREPARATION"
                    } else if kind == Some(ExecutionErrorKind::Runtime) {
                        "RUNTIME_INTERNAL"
                    } else {
                        "TRANSACTION_SUBMISSION"
                    },
                    &definition.instruction,
                    record.error.as_deref().unwrap_or("transaction failed"),
                ));
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(evidence),
            Err(mut e) => {
                e.evidence = Some(evidence);
                Err(e)
            }
        }
    }

    fn observe(
        &self,
        world: &RuntimeWorld,
        targets: &[MigrationTarget],
        after: bool,
        model: &ProgramModel,
        output: &mut Vec<AccountEvidence>,
    ) -> Result<()> {
        let mut names = BTreeSet::new();
        let mut addresses = BTreeSet::new();
        for target in targets {
            let address = if let Some(name) = target.address.strip_prefix('$') {
                *world.resolved.addresses.get(name).ok_or_else(|| {
                    error("ACCOUNT_BINDING", name, "unresolved observation address")
                })?
            } else {
                Address::from_str(&target.address)
                    .map_err(|e| error("ACCOUNT_BINDING", &target.name, e))?
            };
            if target.name.is_empty() || !names.insert(&target.name) || !addresses.insert(address) {
                return Err(error(
                    "ACCOUNT_BINDING",
                    &target.name,
                    "duplicate/empty observation name or address",
                ));
            }
            let account_type = if after {
                &target.v2_type
            } else {
                &target.v1_type
            };
            let snapshot = world
                .runtime
                .get_account(&address)
                .map(|account| {
                    RawAccountSnapshot::capture(
                        address.to_string(),
                        account.owner.to_string(),
                        account.lamports,
                        account.executable,
                        account.data,
                        self.limits,
                    )
                })
                .transpose()
                .map_err(|e| error("SNAPSHOT", &target.name, e))?;
            let (decoded, decode_error) = match (&snapshot, account_type) {
                (Some(raw), Some(name)) => match dg_state::decode_account(
                    model,
                    name,
                    &raw.raw_data,
                    self.limits,
                    TrailingBytes::Preserve,
                ) {
                    Ok(value) => (Some(value), None),
                    Err(e) => (None, Some(e)),
                },
                _ => (None, None),
            };
            output.push(AccountEvidence {
                name: target.name.clone(),
                address: address.to_string(),
                account_type: account_type.clone(),
                snapshot,
                decoded,
                decode_error,
            });
        }
        Ok(())
    }
}
