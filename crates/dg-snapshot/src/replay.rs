//! Offline orchestration. No network client or endpoint is consulted here.
use super::*;
use dg_config::FixtureDefinition;
use dg_runtime::{
    lifecycle::LifecycleEngine, preservation, MigrationExecutor, RuntimeWorldBuilder,
    WorldBuildResult,
};
use serde_json::{json, Value};
use solana_account::Account;
use solana_signer::Signer;

#[derive(Default)]
pub struct CandidateOverride {
    pub binary: Option<PathBuf>,
    pub idl: Option<PathBuf>,
}

pub fn apply_candidate(m: &mut ProjectManifest, replacement: &CandidateOverride) -> Result<()> {
    if replacement.idl.is_some() && replacement.binary.is_none() {
        return Err(err(
            "CANDIDATE_INCOMPATIBLE",
            "candidate",
            "IDL override requires explicit binary override",
        ));
    }
    if let Some(binary) = &replacement.binary {
        m.versions.v2.program_path = fs::canonicalize(binary).map_err(|_| {
            err(
                "MISSING_PROGRAM_ARTIFACT",
                "candidate",
                "candidate binary is missing",
            )
        })?;
    }
    if let Some(idl) = &replacement.idl {
        let path = fs::canonicalize(idl).map_err(|_| {
            err(
                "MISSING_PROGRAM_ARTIFACT",
                "candidate_idl",
                "candidate IDL is missing",
            )
        })?;
        let model = dg_analysis::normalize(&fs::read(&path).map_err(|_| {
            err(
                "MISSING_PROGRAM_ARTIFACT",
                "candidate_idl",
                "cannot read IDL",
            )
        })?)
        .map_err(|_| err("CANDIDATE_INCOMPATIBLE", "candidate_idl", "invalid IDL"))?;
        if model.address.as_deref() != Some(&m.versions.v1.program_id) {
            return Err(err(
                "CANDIDATE_INCOMPATIBLE",
                "candidate_idl",
                "IDL program ID differs from imported program",
            ));
        }
        m.versions.v2.idl = path;
    }
    Ok(())
}

pub fn build_world(
    snapshot: &DevnetSnapshot,
    manifest: &ProjectManifest,
    base: &Path,
) -> Result<WorldBuildResult> {
    snapshot.verify()?;
    let c = config(manifest)?;
    let resolved = resolve(manifest)?;
    if manifest.versions.v1.program_id != snapshot.provenance.program_id
        || manifest.versions.v2.program_id != manifest.versions.v1.program_id
    {
        return Err(err(
            "CANDIDATE_INCOMPATIBLE",
            "program_id",
            "configured program differs from snapshot source",
        ));
    }
    let mut expected = BTreeMap::new();
    let targets = MigrationExecutor::targets(manifest);
    for def in &c.accounts {
        let pubkey = binding(&def.address, &resolved)?;
        let captured = snapshot
            .accounts
            .iter()
            .find(|a| a.logical_name == def.name)
            .ok_or_else(|| {
                err(
                    "ACCOUNT_NOT_FOUND",
                    &def.name,
                    "configured account is not present in snapshot",
                )
            })?;
        let signer = resolved
            .signers
            .get(&def.name)
            .or_else(|| resolved.signers.values().find(|k| k.pubkey() == pubkey));
        if signer.is_some_and(|k| k.pubkey().to_string() != captured.address) {
            return Err(err(
                "SIGNER_ADDRESS_MISMATCH",
                &def.name,
                "configured local key does not match imported signer",
            ));
        }
        if pubkey.to_string() != captured.address {
            return Err(err(
                "SNAPSHOT_BINDING_MISMATCH",
                &def.name,
                "configured address differs from captured address",
            ));
        }
        if !captured.exists && !def.optional {
            return Err(err(
                "ACCOUNT_NOT_FOUND",
                &def.name,
                "required imported account is absent",
            ));
        }
        let owner = match &def.owner {
            Some(o) => binding(o, &resolved)?,
            None if targets.iter().any(|t| t.name == def.name) => {
                address(&manifest.versions.v1.program_id)?
            }
            None => {
                return Err(err(
                    "CONFIG",
                    &def.name,
                    "external account requires explicit owner",
                ))
            }
        };
        if captured
            .raw
            .as_ref()
            .is_some_and(|r| r.owner != owner.to_string())
        {
            return Err(err(
                "OWNER_MISMATCH",
                &def.name,
                "captured owner differs from configuration",
            ));
        }
        if captured
            .raw
            .as_ref()
            .is_some_and(|r| r.data_length > c.max_account_bytes)
        {
            return Err(err(
                "ACCOUNT_DATA_TOO_LARGE",
                &def.name,
                "account exceeds configured byte limit",
            ));
        }
        if expected.insert(pubkey, captured).is_some() {
            return Err(err("CONFIG", &def.name, "duplicate imported address"));
        }
    }
    if expected.len() != snapshot.accounts.len() {
        return Err(err(
            "SNAPSHOT_BINDING_MISMATCH",
            "accounts",
            "snapshot and configuration inventories differ",
        ));
    }
    for t in &targets {
        if !expected.contains_key(&binding(&t.address, &resolved)?) {
            return Err(err(
                "ACCOUNT_NOT_FOUND",
                &t.name,
                "every migration target must be explicitly captured, including absent accounts",
            ));
        }
    }
    for (role, path) in [
        ("v1_binary", &manifest.versions.v1.program_path),
        ("v1_idl", &manifest.versions.v1.idl),
    ] {
        let bytes = fs::read(base.join(path)).map_err(|_| {
            err(
                "MISSING_PROGRAM_ARTIFACT",
                role,
                "local artifact is required",
            )
        })?;
        if snapshot
            .provenance
            .artifacts
            .get(role)
            .and_then(|a| a.sha256.as_ref())
            .is_some_and(|hash| *hash != digest(&bytes))
        {
            return Err(err(
                "CANDIDATE_INCOMPATIBLE",
                role,
                "V1 artifact differs from captured reference",
            ));
        }
    }
    let mut local = manifest.clone();
    // R5 initializes only the VM and explicit local signers. Imported accounts are never reinitialized.
    local.fixture = Some(FixtureDefinition {
        name: "snapshot-import".into(),
        payer: manifest.fixture.as_ref().and_then(|f| f.payer.clone()),
        account: vec![],
        initialize: vec![],
        setup: vec![],
    });
    let mut built = RuntimeWorldBuilder::default()
        .build_manifest(&local, base)
        .map_err(|e| err("LOCAL_REPLAY_FAILED", e.context, e.message))?;
    for p in &c.programs {
        let id = address(&p.program_id)?;
        if built.world.runtime.get_account(&id).is_some() {
            return Err(err(
                "CONFIG",
                "external_program",
                "external program collides with existing runtime account",
            ));
        }
        let data = fs::read(base.join(&p.program_path)).map_err(|_| {
            err(
                "MISSING_PROGRAM_ARTIFACT",
                "external_program",
                "external binary is required",
            )
        })?;
        let pinned = snapshot
            .provenance
            .artifacts
            .get(&format!("external:{}", p.program_id))
            .and_then(|a| a.sha256.as_ref())
            .ok_or_else(|| {
                err(
                    "MISSING_PROGRAM_ARTIFACT",
                    "external_program",
                    "snapshot must pin the external program hash",
                )
            })?;
        if *pinned != digest(&data) {
            return Err(err(
                "CANDIDATE_INCOMPATIBLE",
                "external_program",
                "external binary hash differs",
            ));
        }
        built.world.runtime.add_program(id, &data).map_err(|_| {
            err(
                "LOCAL_REPLAY_FAILED",
                "external_program",
                "cannot load external program",
            )
        })?;
    }
    for (pubkey, captured) in &expected {
        if let Some(existing) = built.world.runtime.get_account(pubkey) {
            if existing.executable {
                return Err(err(
                    "CONFIG",
                    &captured.logical_name,
                    "do not import executable programs; use local artifacts or built-ins",
                ));
            }
        }
        let Some(raw) = &captured.raw else {
            built
                .world
                .runtime
                .set_account(*pubkey, Account::default())
                .map_err(|_| {
                    err(
                        "LOCAL_REPLAY_FAILED",
                        &captured.logical_name,
                        "cannot restore absence",
                    )
                })?;
            continue;
        };
        if raw.executable {
            return Err(err(
                "CONFIG",
                &captured.logical_name,
                "executable account imports require a local program artifact",
            ));
        }
        let account = Account {
            lamports: u64::try_from(&raw.lamports).map_err(|_| {
                err(
                    "RPC_RESPONSE_INVALID",
                    &captured.logical_name,
                    "invalid lamports",
                )
            })?,
            owner: address(&raw.owner)?,
            data: raw.raw_data.clone(),
            executable: raw.executable,
            rent_epoch: captured
                .rent_epoch
                .as_ref()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| {
                    err(
                        "RPC_RESPONSE_INVALID",
                        &captured.logical_name,
                        "invalid rent epoch",
                    )
                })?,
        };
        if account.lamports == 0 {
            built
                .world
                .runtime
                .set_account_no_checks(*pubkey, account.into());
        } else {
            built
                .world
                .runtime
                .set_account(*pubkey, account)
                .map_err(|_| {
                    err(
                        "LOCAL_REPLAY_FAILED",
                        &captured.logical_name,
                        "account import failed",
                    )
                })?;
        }
    }
    for (pubkey, captured) in expected {
        let observed = built.world.runtime.get_account(&pubkey);
        match (&captured.raw, observed) {
            (None, None) => {}
            (Some(raw), Some(a))
                if a.data == raw.raw_data
                    && a.owner.to_string() == raw.owner
                    && a.executable == raw.executable
                    && a.lamports.to_string() == raw.lamports.to_string()
                    && Some(a.rent_epoch.to_string()) == captured.rent_epoch => {}
            _ => {
                return Err(err(
                    "LOCAL_REPLAY_FAILED",
                    &captured.logical_name,
                    "imported account differs from immutable snapshot",
                ))
            }
        }
    }
    Ok(built)
}

pub fn lifecycle(
    snapshot: &DevnetSnapshot,
    m: &ProjectManifest,
    base: &Path,
) -> Result<dg_runtime::lifecycle::LifecycleReport> {
    let base_fixture = m.fixture.as_ref().map(|f| f.name.clone());
    LifecycleEngine
        .run_with_world_builder(m, base, |local, base| {
            let result = if local.fixture.as_ref().map(|f| f.name.clone()) != base_fixture {
                Err(err(
                    "SNAPSHOT_FIXTURE_INCOMPATIBLE",
                    "scenario.fixture",
                    "alternate setup fixtures cannot replace pinned imported state",
                ))
            } else {
                build_world(snapshot, local, base)
            };
            result.map_err(|e| {
                Box::new(dg_runtime::BuildError {
                    code: e.code,
                    context: e.context,
                    message: e.message,
                    executed_actions: vec![],
                    observed_accounts: vec![],
                })
            })
        })
        .map_err(|e| err("LOCAL_REPLAY_FAILED", e.field, e.message))
}

pub fn replay(
    snapshot: &DevnetSnapshot,
    manifest: &ProjectManifest,
    base: &Path,
    candidate: &CandidateOverride,
    run_lifecycle: bool,
) -> Result<Value> {
    snapshot.verify()?;
    let mut m = manifest.clone();
    apply_candidate(&mut m, candidate)?;
    let mut artifacts = BTreeMap::new();
    for (role, path) in [
        ("v1_binary", &m.versions.v1.program_path),
        ("v1_idl", &m.versions.v1.idl),
        ("v2_binary", &m.versions.v2.program_path),
        ("v2_idl", &m.versions.v2.idl),
    ] {
        let path = base.join(path);
        let data = fs::read(&path).map_err(|_| {
            err(
                "MISSING_PROGRAM_ARTIFACT",
                role,
                "local artifact is missing",
            )
        })?;
        artifacts.insert(
            role,
            ArtifactReference {
                path,
                sha256: Some(digest(&data)),
            },
        );
    }
    let mut built = build_world(snapshot, &m, base)?;
    let execution = MigrationExecutor::default()
        .execute_manifest(&mut built.world, &m, base)
        .map_err(|e| {
            err(
                match e.code.as_str() {
                    "MISSING_SIGNER" => "MISSING_LOCAL_SIGNER",
                    "PROGRAM_ID" => "CANDIDATE_INCOMPATIBLE",
                    _ => "LOCAL_REPLAY_FAILED",
                },
                e.context,
                e.message,
            )
        })?;
    let preservation = preservation::evaluate(&execution, &m.preservation, &built.world.model)
        .map_err(|e| err("LOCAL_REPLAY_FAILED", e.field, e.message))?;
    let lifecycle = if run_lifecycle {
        Some(lifecycle(snapshot, &m, base)?)
    } else {
        None
    };
    for (role, a) in &artifacts {
        if fs::read(&a.path).ok().map(|d| digest(&d)) != a.sha256 {
            return Err(err(
                "CANDIDATE_INCOMPATIBLE",
                role,
                "artifact changed during replay",
            ));
        }
    }
    Ok(
        json!({"schema_version":1,"project":m.project.name,"snapshot":snapshot,"source":{"cluster":snapshot.provenance.cluster,"slot":snapshot.provenance.slot,"snapshot_sha256":snapshot.snapshot_sha256},"execution":execution,"preservation":preservation,"lifecycle":lifecycle,"artifacts":artifacts,"preservation_evaluated":true,"lifecycle_evaluated":run_lifecycle,"policy_evaluated":false,"remote_state_imported":snapshot.provenance.acquisition==Acquisition::Rpc,"remote_transactions_sent":false,"rpc_write_calls":0,"migration_executed_locally":true}),
    )
}
