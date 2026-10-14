//! Construct V1 state and execute configured migrations without preservation policy.
pub mod lifecycle;
pub mod migration;
pub mod preservation;
use dg_analysis::{model::ProgramModel, normalize};
use dg_config::{
    fixture::FixtureInstructions, FixtureDefinition, ProjectManifest, ResolvedAddresses,
};
use dg_state::{
    raw::RawAccountSnapshot, AccountBinding, DecodeLimits, DecodedAccount, ProgramArtifact,
    StateError, TrailingBytes,
};
use litesvm::LiteSVM;
pub use migration::{MigrationExecution, MigrationExecutor};
use serde::Serialize;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::Instruction;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
    str::FromStr,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Initialize,
    Setup,
    Migration,
    Instruction,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedAccountMeta {
    pub address: String,
    pub signer: bool,
    pub writable: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ExecutionRecord {
    pub phase: Phase,
    pub action_index: usize,
    pub instruction_name: String,
    pub program_id: String,
    pub accounts: Vec<ResolvedAccountMeta>,
    pub signer_public_keys: Vec<String>,
    pub instruction_data: Vec<u8>,
    pub success: bool,
    pub error: Option<String>,
    pub error_kind: Option<ExecutionErrorKind>,
    pub transaction_error: Option<solana_transaction_error::TransactionError>,
    pub logs: Vec<String>,
    pub compute_units: Option<u64>,
    pub signature: Option<String>,
    pub fee: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionErrorKind {
    Preparation,
    Submission,
    Program,
    Runtime,
}
impl From<&solana_transaction_error::TransactionError> for ExecutionErrorKind {
    fn from(error: &solana_transaction_error::TransactionError) -> Self {
        use solana_transaction_error::TransactionError;
        match error {
            TransactionError::InstructionError(..) => Self::Program,
            TransactionError::AccountBorrowOutstanding
            | TransactionError::ResanitizationNeeded
            | TransactionError::ProgramCacheHitMaxLimit
            | TransactionError::CommitCancelled => Self::Runtime,
            _ => Self::Submission,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ObservedAccount {
    pub binding: AccountBinding,
    pub snapshot: RawAccountSnapshot,
    pub decoded: Option<DecodedAccount>,
    pub decode_error: Option<StateError>,
}
#[derive(Debug, Serialize)]
pub struct BuildError {
    pub code: String,
    pub context: String,
    pub message: String,
    pub executed_actions: Vec<ExecutionRecord>,
    pub observed_accounts: Vec<ObservedAccount>,
}
impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.context, self.message)
    }
}
impl std::error::Error for BuildError {}
pub type Result<T> = std::result::Result<T, Box<BuildError>>;
fn fail(code: &str, context: impl Into<String>, message: impl ToString) -> Box<BuildError> {
    Box::new(BuildError {
        code: code.into(),
        context: context.into(),
        message: message.to_string(),
        executed_actions: vec![],
        observed_accounts: vec![],
    })
}
fn address(value: &str, context: &str) -> Result<Address> {
    Address::from_str(value).map_err(|e| fail("ACCOUNT_BINDING", context, e))
}

fn install_account(
    runtime: &mut LiteSVM,
    address: Address,
    account: Account,
    context: &str,
) -> Result<()> {
    // LiteSVM's normal setter purges zero balances. Explicit local boundary fixtures
    // need their bytes retained; transactions still use normal runtime rules.
    if account.lamports == 0 && !account.executable {
        runtime.set_account_no_checks(address, account.into());
        Ok(())
    } else {
        runtime
            .set_account(address, account)
            .map_err(|e| fail("ACCOUNT_CREATE", context, e))
    }
}

pub struct RuntimeWorld {
    pub runtime: LiteSVM,
    pub resolved: ResolvedAddresses,
    pub artifact: ProgramArtifact,
    pub model: ProgramModel,
    pub funded_signers: usize,
}
pub struct WorldBuildResult {
    pub world: RuntimeWorld,
    pub executed_actions: Vec<ExecutionRecord>,
    /// Declaration order, independent of the runtime's internal map order.
    pub observed_accounts: Vec<ObservedAccount>,
}

#[derive(Default)]
pub struct RuntimeWorldBuilder {
    pub limits: DecodeLimits,
}
impl RuntimeWorldBuilder {
    /// Reads V1 only. V2 artifact paths and migration definitions are not loaded or resolved.
    pub fn build_project(&self, path: &Path) -> Result<WorldBuildResult> {
        let path = if path.is_dir() {
            path.join("doppelganger.toml")
        } else {
            path.to_owned()
        };
        let text = dg_analysis::read_bounded(&path, 4 * 1024 * 1024)
            .and_then(|b| String::from_utf8(b).map_err(std::io::Error::other))
            .map_err(|e| fail("CONFIG", path.display().to_string(), e))?;
        let manifest =
            ProjectManifest::parse(&text).map_err(|e| fail("CONFIG", &e.field, &e.message))?;
        self.build_manifest(&manifest, path.parent().unwrap_or(Path::new(".")))
    }
    pub fn build_manifest(
        &self,
        manifest: &ProjectManifest,
        base: &Path,
    ) -> Result<WorldBuildResult> {
        let version = &manifest.versions.v1;
        let artifact = ProgramArtifact {
            program_id: version.program_id.clone(),
            program_path: base.join(&version.program_path),
            idl_path: base.join(&version.idl),
        };
        let idl = dg_analysis::read_bounded(&artifact.idl_path, 8 * 1024 * 1024)
            .map_err(|e| fail("IDL", artifact.idl_path.display().to_string(), e))?;
        let model = normalize(&idl).map_err(|e| fail("IDL", &e.path, &e.message))?;
        let programs = BTreeMap::from([
            (
                "v1".into(),
                address(&version.program_id, "versions.v1.program_id")?,
            ),
            (
                "v2".into(),
                address(&manifest.versions.v2.program_id, "versions.v2.program_id")?,
            ),
        ]);
        let resolved = ResolvedAddresses::resolve(&manifest.addresses, &programs)
            .map_err(|e| fail("ADDRESS", &e.field, &e.message))?;
        let fixture = manifest
            .fixture
            .as_ref()
            .ok_or_else(|| fail("CONFIG", "fixture", "fixture is required"))?;
        let actions = fixture
            .instructions(&model, &artifact.program_id, &resolved)
            .map_err(|e| fail("CONFIG", &e.field, &e.message))?;
        self.build(artifact, &model, fixture, resolved, actions)
    }
    pub fn build(
        &self,
        artifact: ProgramArtifact,
        model: &ProgramModel,
        fixture: &FixtureDefinition,
        resolved: ResolvedAddresses,
        actions: FixtureInstructions,
    ) -> Result<WorldBuildResult> {
        if fixture.account.len() > 10_000 {
            return Err(fail("CONFIG", "fixture", "maximum 10,000 accounts"));
        }
        let program_id = address(&artifact.program_id, "program_id")?;
        if model
            .address
            .as_ref()
            .is_some_and(|id| id != &artifact.program_id)
        {
            return Err(fail("PROGRAM_ID", "v1", "IDL and program IDs differ"));
        }
        if actions.initialize.len() != fixture.initialize.len()
            || actions.setup.len() != fixture.setup.len()
        {
            return Err(fail(
                "CONFIG",
                "fixture.actions",
                "compiled action counts differ from fixture",
            ));
        }
        let binary =
            dg_analysis::read_bounded(&artifact.program_path, 64 * 1024 * 1024).map_err(|e| {
                fail(
                    if e.kind() == std::io::ErrorKind::NotFound {
                        "MISSING_PROGRAM"
                    } else {
                        "PROGRAM_ARTIFACT"
                    },
                    artifact.program_path.display().to_string(),
                    e,
                )
            })?;
        if !binary.starts_with(b"\x7fELF") {
            return Err(fail(
                "INVALID_PROGRAM",
                artifact.program_path.display().to_string(),
                "expected an ELF program binary",
            ));
        }
        let mut runtime = LiteSVM::new();
        if runtime.get_account(&program_id).is_some() {
            return Err(fail(
                "PROGRAM_ID",
                "v1",
                "program ID collides with a built-in account",
            ));
        }
        runtime.add_program(program_id, &binary).map_err(|e| {
            fail(
                "PROGRAM_LOAD",
                artifact.program_path.display().to_string(),
                e,
            )
        })?;
        let mut funded = BTreeSet::new();
        for (name, key) in &resolved.signers {
            let pubkey = key.pubkey();
            if resolved.addresses.get(name) != Some(&pubkey) {
                return Err(fail(
                    "SIGNER",
                    name,
                    "signer public key differs from address table",
                ));
            }
            let lamports = *resolved
                .funding
                .get(name)
                .ok_or_else(|| fail("SIGNER", name, "missing funding definition"))?;
            if !funded.insert(pubkey) {
                return Err(fail("SIGNER", name, "duplicate signer identity"));
            }
            if runtime.get_account(&pubkey).is_some() {
                return Err(fail(
                    "SIGNER",
                    name,
                    "signer collides with an existing runtime account",
                ));
            }
            install_account(
                &mut runtime,
                pubkey,
                Account {
                    lamports,
                    owner: Address::default(),
                    ..Account::default()
                },
                name,
            )?;
        }
        let mut world = RuntimeWorld {
            model: model.clone(),
            runtime,
            resolved,
            artifact,
            funded_signers: funded.len(),
        };
        // Reuse the state encoder only for explicitly synthetic accounts.
        let mut synthetic = fixture.clone();
        synthetic.initialize.clear();
        synthetic.setup.clear();
        synthetic.account.retain(|account| account.fields.is_some());
        let state =
            dg_state::load_fixture(&synthetic, model, &world.resolved.strings(), self.limits)
                .map_err(|e| fail("SYNTHETIC_ACCOUNT", &e.path, &e.message))?;
        for snapshot in state.accounts.values() {
            let pubkey = address(&snapshot.binding.address, &snapshot.binding.name)?;
            if world.runtime.get_account(&pubkey).is_some() {
                return Err(fail(
                    "SYNTHETIC_ACCOUNT",
                    &snapshot.binding.name,
                    "would overwrite an existing runtime account",
                ));
            }
            let lamports = u64::try_from(&snapshot.lamports)
                .map_err(|e| fail("SYNTHETIC_ACCOUNT", &snapshot.binding.name, e))?;
            install_account(
                &mut world.runtime,
                pubkey,
                Account {
                    lamports,
                    owner: address(&snapshot.binding.owner, &snapshot.binding.name)?,
                    data: snapshot.raw_data.clone(),
                    executable: snapshot.executable,
                    ..Account::default()
                },
                &snapshot.binding.name,
            )?;
        }
        let mut records = Vec::new();
        for (phase, instructions, definitions) in [
            (Phase::Initialize, actions.initialize, &fixture.initialize),
            (Phase::Setup, actions.setup, &fixture.setup),
        ] {
            for (index, (instruction, definition)) in
                instructions.into_iter().zip(definitions).enumerate()
            {
                let record = world.execute(
                    phase,
                    index,
                    &definition.instruction,
                    instruction,
                    fixture.payer.as_deref(),
                );
                let failed = !record.success;
                records.push(record);
                if failed {
                    let last = &records[records.len() - 1];
                    let mut err = fail(
                        "ACTION_FAILED",
                        format!("{phase:?}[{index}] {}", definition.instruction),
                        last.error.as_deref().unwrap_or("unknown execution failure"),
                    );
                    err.executed_actions = records;
                    return Err(err);
                }
            }
        }
        let observations = world
            .observe(fixture, model, self.limits)
            .map_err(|mut e| {
                e.executed_actions = records.clone();
                e
            })?;
        Ok(WorldBuildResult {
            world,
            executed_actions: records,
            observed_accounts: observations,
        })
    }
}

impl RuntimeWorld {
    fn execute(
        &mut self,
        phase: Phase,
        index: usize,
        name: &str,
        instruction: Instruction,
        payer_name: Option<&str>,
    ) -> ExecutionRecord {
        let mut record = ExecutionRecord {
            phase,
            action_index: index,
            instruction_name: name.into(),
            program_id: instruction.program_id.to_string(),
            accounts: instruction
                .accounts
                .iter()
                .map(|m| ResolvedAccountMeta {
                    address: m.pubkey.to_string(),
                    signer: m.is_signer,
                    writable: m.is_writable,
                })
                .collect(),
            signer_public_keys: vec![],
            instruction_data: instruction.data.clone(),
            success: false,
            error: None,
            error_kind: None,
            transaction_error: None,
            logs: vec![],
            compute_units: None,
            signature: None,
            fee: None,
        };
        let result = (|| -> std::result::Result<_, String> {
            if instruction.program_id.to_string() != self.artifact.program_id {
                return Err("action must target the loaded program".into());
            }
            let payer = if let Some(name) = payer_name {
                *self
                    .resolved
                    .addresses
                    .get(name)
                    .ok_or_else(|| format!("unresolved fee payer {name}"))?
            } else {
                self.resolved
                    .signers
                    .iter()
                    .find(|(name, _)| {
                        self.resolved
                            .funding
                            .get(*name)
                            .is_some_and(|amount| *amount > 0)
                    })
                    .or_else(|| self.resolved.signers.iter().next())
                    .map(|(_, key)| key.pubkey())
                    .ok_or("no signer available for fee payer")?
            };
            let mut account_keys: BTreeSet<_> = instruction
                .accounts
                .iter()
                .map(|meta| meta.pubkey)
                .collect();
            account_keys.insert(payer);
            account_keys.insert(instruction.program_id);
            if account_keys.len() > 256 {
                return Err("fixture transaction exceeds 256 unique accounts".into());
            }
            let message = Message::new(&[instruction], Some(&payer));
            let required = &message.account_keys[..message.header.num_required_signatures as usize];
            let keys = required
                .iter()
                .map(|pubkey| {
                    self.resolved
                        .signers
                        .values()
                        .find(|key| key.pubkey() == *pubkey)
                        .ok_or_else(|| format!("missing required signer keypair {pubkey}"))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            record.signer_public_keys = required.iter().map(ToString::to_string).collect();
            self.runtime.expire_blockhash();
            let mut transaction = Transaction::new_unsigned(message);
            transaction
                .try_sign(&keys, self.runtime.latest_blockhash())
                .map_err(|e| format!("transaction signing failed: {e}"))?;
            record.signature = transaction.signatures.first().map(ToString::to_string);
            Ok(self.runtime.send_transaction(transaction))
        })();
        match result {
            Err(error) => {
                record.error = Some(error);
                record.error_kind = Some(ExecutionErrorKind::Preparation);
            }
            Ok(result) => {
                let (metadata, error) = match result {
                    Ok(meta) => (meta, None),
                    Err(failure) => {
                        record.transaction_error = Some(failure.err.clone());
                        record.error_kind = Some(ExecutionErrorKind::from(&failure.err));
                        (failure.meta, Some(format!("{:?}", failure.err)))
                    }
                };
                record.success = error.is_none();
                record.error = error;
                record.logs = metadata.logs;
                record.compute_units = Some(metadata.compute_units_consumed);
                record.fee = Some(metadata.fee);
            }
        }
        record
    }
    pub fn observe(
        &self,
        fixture: &FixtureDefinition,
        model: &ProgramModel,
        limits: DecodeLimits,
    ) -> Result<Vec<ObservedAccount>> {
        let mut observations = Vec::new();
        let mut names = BTreeSet::new();
        let mut seen = BTreeSet::new();
        for definition in &fixture.account {
            let result = (|| {
                let resolve = |source: &str| -> Result<Address> {
                    match source.strip_prefix('$') {
                        Some(name) => self.resolved.addresses.get(name).copied().ok_or_else(|| {
                            fail("ACCOUNT_BINDING", name, "unresolved observation address")
                        }),
                        None => address(source, &definition.name),
                    }
                };
                let pubkey = resolve(&definition.address)?;
                if definition.name.is_empty()
                    || !names.insert(&definition.name)
                    || !seen.insert(pubkey)
                {
                    return Err(fail(
                        "ACCOUNT_BINDING",
                        &definition.name,
                        "duplicate/empty observation name or address",
                    ));
                }
                let expected_owner = resolve(&definition.owner)?;
                let actual = self.runtime.get_account(&pubkey).ok_or_else(|| {
                    fail(
                        "MISSING_ACCOUNT",
                        &definition.name,
                        "observation target was not created",
                    )
                })?;
                let snapshot = RawAccountSnapshot::capture(
                    pubkey.to_string(),
                    actual.owner.to_string(),
                    actual.lamports,
                    actual.executable,
                    actual.data,
                    limits,
                )
                .map_err(|e| fail("SNAPSHOT", &definition.name, e))?;
                let (decoded, decode_error) = match dg_state::decode_account(
                    model,
                    &definition.account_type,
                    &snapshot.raw_data,
                    limits,
                    TrailingBytes::Preserve,
                ) {
                    Ok(decoded) => (Some(decoded), None),
                    Err(error) => (None, Some(error)),
                };
                Ok(ObservedAccount {
                    binding: AccountBinding {
                        name: definition.name.clone(),
                        address: pubkey.to_string(),
                        owner: expected_owner.to_string(),
                        account_type: definition.account_type.clone(),
                    },
                    snapshot,
                    decoded,
                    decode_error,
                })
            })();
            match result {
                Ok(observation) => observations.push(observation),
                Err(mut error) => {
                    error.observed_accounts = observations;
                    return Err(error);
                }
            }
        }
        Ok(observations)
    }
}
