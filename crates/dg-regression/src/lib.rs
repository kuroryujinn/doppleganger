//! Local, immutable regression bundles. Execution belongs exclusively to R5-R8.
use dg_config::{addresses::AddressDefinition, ProjectManifest};
use dg_runtime::{
    lifecycle::LifecycleEngine, preservation, MigrationExecutor, RuntimeWorldBuilder,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, RegressionError>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionError {
    pub code: String,
    pub context: String,
    pub message: String,
}
impl fmt::Display for RegressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}: {}", self.code, self.context, self.message)
    }
}
impl std::error::Error for RegressionError {}
fn error(code: &str, context: impl ToString, message: impl ToString) -> RegressionError {
    RegressionError {
        code: code.into(),
        context: context.to_string(),
        message: message.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Source {
    Preservation(String),
    Lifecycle(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reconstruction {
    FixtureReplay,
    SnapshotReplay,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionCase {
    pub schema_version: u32,
    pub id: String,
    pub project: String,
    pub source_report: PathBuf,
    pub source: Source,
    pub reconstruction: Reconstruction,
    pub definition: Value,
    pub definition_sha256: String,
    pub hashes: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub path: PathBuf,
    pub sha256: String,
}
pub type Artifacts = BTreeMap<String, Artifact>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub source: Source,
    pub status: String,
    pub evidence: Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReplayStatus {
    Reproduced,
    NotReproduced,
    Error,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub schema_version: u32,
    pub regression_id: String,
    pub result: ReplayStatus,
    pub expected: Failure,
    pub observed: Failure,
    pub initial_state_verified: bool,
    pub artifacts: Artifacts,
    pub diagnostics: Vec<RegressionError>,
    pub policy_evaluated: bool,
}
pub use dg_snapshot::replay::CandidateOverride;
#[derive(Debug, Serialize)]
pub struct Created {
    pub case: RegressionCase,
    pub directory: PathBuf,
    pub already_exists: bool,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bytes<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(v).map_err(|e| error("INVALID_REGRESSION_SCHEMA", "serialization", e))
}
fn read(path: &Path, code: &str) -> Result<Vec<u8>> {
    dg_analysis::read_bounded(path, 128 * 1024 * 1024).map_err(|e| error(code, path.display(), e))
}
fn json_read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read(path, "REGRESSION_NOT_FOUND")?)
        .map_err(|e| error("INVALID_REGRESSION_SCHEMA", path.display(), e))
}
fn canonical(path: &Path, code: &str) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|e| error(code, path.display(), e))
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key)
        .ok_or_else(|| error("INVALID_REGRESSION_SCHEMA", key, "missing field"))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    required(v, key)?
        .as_array()
        .ok_or_else(|| error("INVALID_REGRESSION_SCHEMA", key, "expected array"))
}
fn string(v: &Value, key: &str) -> Result<String> {
    required(v, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| error("INVALID_REGRESSION_SCHEMA", key, "expected string"))
}
fn project_manifest(text: &str) -> Result<ProjectManifest> {
    ProjectManifest::parse(text).map_err(|e| error("INVALID_REGRESSION_SCHEMA", e.field, e.message))
}

// Copy only structured evaluator fields, never explanation strings or tx signatures.
fn selected(v: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .map(|k| ((*k).into(), v.get(k).cloned().unwrap_or(Value::Null)))
            .collect(),
    )
}
pub fn normalize(report: &Value, source: &Source) -> Result<(Failure, Value, Value)> {
    match source {
        Source::Preservation(id) => {
            let p = required(report, "preservation")?;
            let found = array(p, "findings")?
                .iter()
                .find(|f| f.get("id") == Some(&json!(id)));
            let rule_id = found
                .and_then(|f| f.get("rule_id"))
                .and_then(Value::as_str)
                .or_else(|| id.strip_prefix("PRESERVE-"))
                .ok_or_else(|| error("SOURCE_FINDING_NOT_FOUND", id, "missing finding"))?;
            let evaluation = array(p, "evaluations")?
                .iter()
                .find(|e| e.get("rule_id") == Some(&json!(rule_id)))
                .ok_or_else(|| error("SOURCE_FINDING_NOT_FOUND", id, "unknown rule"))?;
            let definition = array(p, "rules")?
                .iter()
                .find(|r| r.get("id") == Some(&json!(rule_id)))
                .ok_or_else(|| error("SOURCE_FINDING_NOT_FOUND", id, "missing rule definition"))?
                .clone();
            let before = required(required(report, "execution")?, "before_accounts")?.clone();
            Ok((
                Failure {
                    source: source.clone(),
                    status: string(evaluation, "status")?,
                    evidence: selected(
                        evaluation,
                        &[
                            "rule_id",
                            "rule",
                            "account",
                            "field",
                            "status",
                            "category",
                            "error_code",
                            "expected",
                            "before",
                            "after",
                            "before_present",
                            "after_present",
                        ],
                    ),
                },
                definition,
                before,
            ))
        }
        Source::Lifecycle(id) => {
            let report = report
                .get("lifecycle")
                .filter(|v| !v.is_null())
                .unwrap_or(report);
            let s = array(report, "scenarios")?
                .iter()
                .find(|s| s.get("id") == Some(&json!(id)))
                .ok_or_else(|| error("SOURCE_SCENARIO_NOT_FOUND", id, "unknown scenario"))?;
            let steps = array(s, "steps")?;
            let initial = steps
                .first()
                .ok_or_else(|| error("MISSING_FIXTURE", id, "world evidence missing"))?;
            let normalized: Vec<_> = steps
                .iter()
                .map(|s| {
                    let mut v = selected(
                        s,
                        &[
                            "index",
                            "operation",
                            "status",
                            "expected",
                            "expected_error",
                            "observed",
                            "state_unchanged",
                            "before",
                            "after",
                        ],
                    );
                    v["transaction_error"] = s
                        .pointer("/execution/transaction_error")
                        .cloned()
                        .unwrap_or(Value::Null);
                    v["diagnostics"] = json!(s
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|d| selected(d, &["code", "context"]))
                        .collect::<Vec<_>>());
                    v
                })
                .collect();
            Ok((
                Failure {
                    source: source.clone(),
                    status: string(s, "status")?,
                    evidence: json!({"scenario_id":id,"steps":normalized}),
                },
                required(s, "definition")?.clone(),
                required(initial, "after")?.clone(),
            ))
        }
    }
}

fn check_identities(m: &ProjectManifest) -> Result<()> {
    if m.fixture.is_none() && m.snapshot.is_none() {
        return Err(error(
            "MISSING_FIXTURE",
            "fixture",
            "fixture_replay requires a fixture",
        ));
    }
    for (name, address) in &m.addresses {
        if matches!(
            address,
            AddressDefinition::Signer {
                test_seed: None,
                ..
            }
        ) {
            return Err(error(
                "MISSING_TEST_IDENTITY",
                name,
                "configure an explicit public test_seed before creating the original report",
            ));
        }
    }
    Ok(())
}
fn check_secrets(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if [
                    "private_key",
                    "secret_key",
                    "seed_phrase",
                    "mnemonic",
                    "keypair",
                ]
                .contains(&k.to_lowercase().as_str())
                {
                    return Err(error(
                        "SECRET_MATERIAL",
                        k,
                        "secret fields are not permitted in regression configuration",
                    ));
                }
                check_secrets(v)?;
            }
        }
        Value::Array(a) => {
            for v in a {
                check_secrets(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn check_idls(m: &ProjectManifest) -> Result<()> {
    for version in [&m.versions.v1, &m.versions.v2] {
        let model = dg_analysis::normalize(&read(&version.idl, "MISSING_IDL")?)
            .map_err(|e| error("CANDIDATE_INCOMPATIBLE", e.path, e.message))?;
        if model.address.as_deref() != Some(&version.program_id) {
            return Err(error(
                "CANDIDATE_INCOMPATIBLE",
                version.idl.display(),
                "IDL must declare the configured program ID",
            ));
        }
    }
    Ok(())
}
fn declared_id_mismatch(v: &Value) -> bool {
    if let Some(e) = v.get("transaction_error") {
        if e.pointer("/InstructionError/1/Custom") == Some(&json!(4100)) {
            return true;
        }
    }
    match v {
        Value::Object(m) => m.values().any(declared_id_mismatch),
        Value::Array(a) => a.iter().any(declared_id_mismatch),
        _ => false,
    }
}
fn execute(
    m: &ProjectManifest,
    source: &Source,
    snapshot: Option<&dg_snapshot::DevnetSnapshot>,
) -> Result<Value> {
    check_identities(m)?;
    check_idls(m)?;
    if let Some(snapshot) = snapshot {
        let report = match source {
            Source::Preservation(_) => {
                dg_snapshot::replay::replay(snapshot, m, Path::new("."), &Default::default(), false)
                    .map_err(|e| error(&e.code, e.context, e.message))?
            }
            Source::Lifecycle(_) => serde_json::to_value(
                dg_snapshot::replay::lifecycle(snapshot, m, Path::new("."))
                    .map_err(|e| error(&e.code, e.context, e.message))?,
            )
            .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "lifecycle", e))?,
        };
        if declared_id_mismatch(&report) {
            return Err(error(
                "CANDIDATE_INCOMPATIBLE",
                "program_id",
                "candidate rejected its configured program ID",
            ));
        }
        if matches!(source, Source::Preservation(_))
            && report.pointer("/execution/transaction/success") != Some(&json!(true))
        {
            return Err(error(
                "REPLAY_EXECUTION_ERROR",
                "migration",
                "migration did not succeed",
            ));
        }
        if normalize(&report, source)?.0.status == "ERROR" {
            return Err(error(
                "REPLAY_EXECUTION_ERROR",
                "evaluation",
                "selected rule or scenario returned ERROR",
            ));
        }
        return Ok(report);
    }
    let report = match source {
        Source::Preservation(_) => {
            let mut built = RuntimeWorldBuilder::default()
                .build_manifest(m, Path::new("."))
                .map_err(|e| error("REPLAY_EXECUTION_ERROR", &e.context, &e.message))?;
            let execution = MigrationExecutor::default()
                .execute_manifest(&mut built.world, m, Path::new("."))
                .map_err(|e| error("REPLAY_EXECUTION_ERROR", &e.context, &e.message))?;
            let value = serde_json::to_value(&execution)
                .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "execution", e))?;
            if declared_id_mismatch(&value) {
                return Err(error(
                    "CANDIDATE_INCOMPATIBLE",
                    "program_id",
                    "candidate rejected its configured program ID",
                ));
            }
            if !execution.transaction.as_ref().is_some_and(|t| t.success) {
                return Err(error(
                    "REPLAY_EXECUTION_ERROR",
                    "migration",
                    "migration did not succeed",
                ));
            }
            let preservation =
                preservation::evaluate(&execution, &m.preservation, &built.world.model)
                    .map_err(|e| error("REPLAY_EXECUTION_ERROR", e.field, e.message))?;
            json!({"schema_version":1,"project":m.project.name,"execution":execution,"preservation":preservation,"preservation_evaluated":true})
        }
        Source::Lifecycle(_) => serde_json::to_value(
            LifecycleEngine
                .run(m, Path::new("."))
                .map_err(|e| error("REPLAY_EXECUTION_ERROR", e.field, e.message))?,
        )
        .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "lifecycle", e))?,
    };
    if declared_id_mismatch(&report) {
        return Err(error(
            "CANDIDATE_INCOMPATIBLE",
            "program_id",
            "candidate rejected its configured program ID",
        ));
    }
    let (failure, _, _) = normalize(&report, source)?;
    if failure.status == "ERROR" {
        return Err(error(
            "REPLAY_EXECUTION_ERROR",
            "evaluation",
            "selected rule or scenario returned ERROR",
        ));
    }
    Ok(report)
}

fn artifact_paths(m: &ProjectManifest) -> [(&'static str, &Path); 4] {
    [
        ("v1_binary", &m.versions.v1.program_path),
        ("v2_binary", &m.versions.v2.program_path),
        ("v1_idl", &m.versions.v1.idl),
        ("v2_idl", &m.versions.v2.idl),
    ]
}
fn artifacts(m: &ProjectManifest) -> Result<Artifacts> {
    artifact_paths(m)
        .into_iter()
        .map(|(role, path)| {
            Ok((
                role.into(),
                Artifact {
                    path: path.to_owned(),
                    sha256: sha256(&read(
                        path,
                        if role.ends_with("binary") {
                            "MISSING_BINARY"
                        } else {
                            "MISSING_IDL"
                        },
                    )?),
                },
            ))
        })
        .collect()
}
fn case_id(case: &RegressionCase) -> Result<String> {
    let mut identity = case.clone();
    identity.id.clear();
    Ok(format!("REG-{}", sha256(&bytes(&identity)?)))
}
fn report_artifacts(report: &Value, source: &Source) -> Result<(Value, Value)> {
    let pair = match source {
        Source::Preservation(_) => {
            let e = required(report, "execution")?;
            (required(e, "v1_program")?, required(e, "v2_program")?)
        }
        Source::Lifecycle(id) => {
            let report = report
                .get("lifecycle")
                .filter(|v| !v.is_null())
                .unwrap_or(report);
            let s = array(report, "scenarios")?
                .iter()
                .find(|s| s["id"] == *id)
                .ok_or_else(|| error("SOURCE_SCENARIO_NOT_FOUND", id, "unknown scenario"))?;
            let swap = array(s, "steps")?
                .iter()
                .find_map(|s| s.get("program_swap").filter(|s| !s.is_null()))
                .ok_or_else(|| {
                    error(
                        "INVALID_REGRESSION_SCHEMA",
                        id,
                        "missing candidate load evidence",
                    )
                })?;
            (required(swap, "previous")?, required(swap, "candidate")?)
        }
    };
    Ok((pair.0.clone(), pair.1.clone()))
}
fn validate_snapshots(v: &Value) -> Result<()> {
    let accounts = v
        .as_array()
        .ok_or_else(|| error("INVALID_REGRESSION_SCHEMA", "snapshots", "expected array"))?;
    for account in accounts {
        let raw = required(account, "snapshot")?;
        if !raw.is_null() {
            let _: dg_state::raw::RawAccountSnapshot = serde_json::from_value(raw.clone())
                .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "snapshot", e))?;
        }
        if let Some(decoded) = account.get("decoded").filter(|d| !d.is_null()) {
            let _: dg_state::DecodedAccount = serde_json::from_value(decoded.clone())
                .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "decoded", e))?;
        }
    }
    Ok(())
}

pub fn create(
    report_path: &Path,
    manifest_path: &Path,
    source: Source,
    store: &Path,
) -> Result<Created> {
    let report: Value = json_read(report_path)?;
    if report.get("schema_version") != Some(&json!(1)) {
        return Err(error(
            "INVALID_REGRESSION_SCHEMA",
            "source report",
            "unsupported schema version",
        ));
    }
    let (expected, definition, before) = normalize(&report, &source)?;
    if expected.status != "FAIL" {
        return Err(error(
            "EXPECTED_FAILURE_NOT_REPRODUCED",
            "source",
            "only evaluated FAIL findings/scenarios can be captured",
        ));
    }
    validate_snapshots(&before)?;
    let path = if manifest_path.is_dir() {
        manifest_path.join("doppelganger.toml")
    } else {
        manifest_path.to_owned()
    };
    let path = canonical(&path, "MISSING_FIXTURE")?;
    let text = String::from_utf8(read(&path, "MISSING_FIXTURE")?)
        .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?;
    let mut config: toml::Value =
        toml::from_str(&text).map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?;
    check_secrets(
        &serde_json::to_value(&config)
            .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?,
    )?;
    let original = project_manifest(&text)?;
    check_identities(&original)?;
    let snapshot: Option<dg_snapshot::DevnetSnapshot> = report
        .get("snapshot")
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "snapshot", e))
        })
        .transpose()?;
    if let Some(snapshot) = &snapshot {
        snapshot
            .verify()
            .map_err(|e| error(&e.code, e.context, e.message))?;
    }
    if report.get("project") != Some(&json!(original.project.name)) {
        return Err(error(
            "SOURCE_CONTEXT_MISMATCH",
            "project",
            "report and manifest project differ",
        ));
    }
    let base = path.parent().unwrap_or(Path::new("."));
    if let Some(settings) = config
        .get_mut("snapshot")
        .and_then(toml::Value::as_table_mut)
    {
        settings.remove("rpc_url");
        settings.remove("rpc_url_env");
        if let Some(programs) = settings
            .get_mut("programs")
            .and_then(toml::Value::as_array_mut)
        {
            for program in programs {
                let path = program
                    .get("program_path")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| {
                        error(
                            "INVALID_REGRESSION_SCHEMA",
                            "external_program",
                            "missing local artifact",
                        )
                    })?;
                let path = canonical(&base.join(path), "MISSING_BINARY")?;
                program["program_path"] = toml::Value::String(path.to_string_lossy().into_owned());
            }
        }
    }
    for version in ["v1", "v2"] {
        for field in ["program_path", "idl"] {
            let original = config["versions"][version][field].as_str().ok_or_else(|| {
                error("INVALID_REGRESSION_SCHEMA", field, "expected artifact path")
            })?;
            let configured = base.join(original);
            // R11 reports pin candidate overrides independently of the source project.
            let role = if field == "idl" {
                "v2_idl"
            } else {
                "v2_binary"
            };
            let reported = if snapshot.is_some() && version == "v2" {
                report.get("artifacts").and_then(|a| a.get(role))
            } else {
                None
            };
            let candidate = reported
                .and_then(|a| a.get("path"))
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let absolute = canonical(
                candidate.as_deref().unwrap_or(&configured),
                if field == "idl" {
                    "MISSING_IDL"
                } else {
                    "MISSING_BINARY"
                },
            )?;
            if let Some(reported) = reported {
                if reported.get("sha256")
                    != Some(&json!(sha256(&read(&absolute, "MISSING_BINARY")?)))
                {
                    return Err(error(
                        "ARTIFACT_HASH_MISMATCH",
                        role,
                        "candidate changed since source report",
                    ));
                }
            }
            config["versions"][version][field] =
                toml::Value::String(absolute.to_string_lossy().into_owned());
        }
    }
    let normalized_manifest = toml::to_string_pretty(&config)
        .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?;
    let parsed = project_manifest(&normalized_manifest)?;
    let (v1, v2) = report_artifacts(&report, &source)?;
    for (version, observed) in [(&parsed.versions.v1, v1), (&parsed.versions.v2, v2)] {
        if observed.get("program_id") != Some(&json!(version.program_id)) {
            return Err(error(
                "SOURCE_CONTEXT_MISMATCH",
                "program_id",
                "report program ID differs",
            ));
        }
        for (field, path) in [
            ("program_path", &version.program_path),
            ("idl_path", &version.idl),
        ] {
            if canonical(
                Path::new(&string(&observed, field)?),
                "SOURCE_CONTEXT_MISMATCH",
            )? != *path
            {
                return Err(error(
                    "SOURCE_CONTEXT_MISMATCH",
                    field,
                    "report and manifest artifacts differ",
                ));
            }
        }
    }
    let captured_artifacts = artifacts(&parsed)?;
    // Reports predating R10 have no fixture or historical hashes: verify the supplied context by executing it.
    let fresh = execute(&parsed, &source, snapshot.as_ref())?;
    let (actual, actual_definition, actual_before) = normalize(&fresh, &source)?;
    if expected != actual || definition != actual_definition || before != actual_before {
        return Err(error(
            "SOURCE_CONTEXT_MISMATCH",
            "source evidence",
            "deterministic replay does not match the supplied report",
        ));
    }
    let table = config
        .as_table_mut()
        .ok_or_else(|| error("INVALID_REGRESSION_SCHEMA", "manifest", "expected table"))?;
    match &source {
        Source::Preservation(_) => {
            let rule: dg_config::preservation::PreservationRule =
                serde_json::from_value(definition.clone())
                    .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "rule", e))?;
            table.insert(
                "preservation".into(),
                toml::Value::Array(vec![toml::Value::try_from(rule)
                    .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "rule", e))?]),
            );
            table.remove("scenario");
        }
        Source::Lifecycle(_) => {
            let scenario: dg_config::lifecycle::LifecycleScenario =
                serde_json::from_value(definition.clone())
                    .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "scenario", e))?;
            table.insert(
                "scenario".into(),
                toml::Value::Array(vec![toml::Value::try_from(scenario)
                    .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "scenario", e))?]),
            );
            table.remove("preservation");
        }
    }
    let manifest = toml::to_string_pretty(&config)
        .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?;
    project_manifest(&manifest)?;
    if artifacts(&parsed)? != captured_artifacts {
        return Err(error(
            "ARTIFACT_HASH_MISMATCH",
            "creation",
            "artifacts changed during capture",
        ));
    }
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::from([
        ("manifest.toml".into(), manifest.into_bytes()),
        ("expected.json".into(), bytes(&expected)?),
        ("artifacts.json".into(), bytes(&captured_artifacts)?),
        ("snapshots/before.json".into(), bytes(&before)?),
    ]);
    if let Some(snapshot) = &snapshot {
        files.insert("snapshots/source.json".into(), bytes(snapshot)?);
    }
    let mut case = RegressionCase {
        schema_version: 1,
        id: String::new(),
        project: parsed.project.name,
        source_report: canonical(report_path, "REGRESSION_NOT_FOUND")?,
        source,
        reconstruction: if snapshot.is_some() {
            Reconstruction::SnapshotReplay
        } else {
            Reconstruction::FixtureReplay
        },
        definition_sha256: sha256(&bytes(&definition)?),
        definition,
        hashes: files.iter().map(|(n, b)| (n.clone(), sha256(b))).collect(),
    };
    case.id = case_id(&case)?;
    fs::create_dir_all(store).map_err(|e| error("ARTIFACT_WRITE", store.display(), e))?;
    let directory = store.join(&case.id);
    if directory.exists() {
        let existing = load(&directory)?;
        if existing.case != case {
            return Err(error(
                "HASH_MISMATCH",
                directory.display(),
                "immutable regression collision",
            ));
        }
        return Ok(Created {
            case,
            directory,
            already_exists: true,
        });
    }
    // case.json is the commit marker; incomplete bundles are never treated as usable cases.
    fs::create_dir(&directory).map_err(|e| error("ARTIFACT_WRITE", directory.display(), e))?;
    let written = (|| {
        fs::create_dir(directory.join("snapshots"))
            .map_err(|e| error("ARTIFACT_WRITE", "snapshots", e))?;
        for (name, data) in files {
            fs::write(directory.join(name), data)
                .map_err(|e| error("ARTIFACT_WRITE", directory.display(), e))?;
        }
        fs::write(directory.join("case.json"), bytes(&case)?)
            .map_err(|e| error("ARTIFACT_WRITE", directory.display(), e))?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = fs::remove_dir_all(&directory);
        return Err(e);
    }
    Ok(Created {
        case,
        directory,
        already_exists: false,
    })
}

pub struct Loaded {
    pub case: RegressionCase,
    pub expected: Failure,
    pub before: Value,
    pub artifacts: Artifacts,
    pub manifest: ProjectManifest,
    pub snapshot: Option<dg_snapshot::DevnetSnapshot>,
}
pub fn load(directory: &Path) -> Result<Loaded> {
    let case: RegressionCase = json_read(&directory.join("case.json"))?;
    if case.schema_version != 1 {
        return Err(error(
            "INVALID_REGRESSION_SCHEMA",
            "schema_version",
            "only version 1 is supported",
        ));
    }
    if case.id != case_id(&case)? || case.definition_sha256 != sha256(&bytes(&case.definition)?) {
        return Err(error(
            "HASH_MISMATCH",
            "case.json",
            "case identity or definition changed",
        ));
    }
    let mut expected_files = vec![
        "artifacts.json",
        "expected.json",
        "manifest.toml",
        "snapshots/before.json",
    ];
    if case.reconstruction == Reconstruction::SnapshotReplay {
        expected_files.push("snapshots/source.json");
    }
    if case.hashes.keys().map(String::as_str).collect::<Vec<_>>() != expected_files {
        return Err(error(
            "INVALID_REGRESSION_SCHEMA",
            "hashes",
            "unexpected bundle file inventory",
        ));
    }
    for (name, hash) in &case.hashes {
        if sha256(&read(&directory.join(name), "REGRESSION_NOT_FOUND")?) != *hash {
            return Err(error("HASH_MISMATCH", name, "bundle content changed"));
        }
    }
    let expected: Failure = json_read(&directory.join("expected.json"))?;
    if expected.status != "FAIL" || expected.source != case.source {
        return Err(error(
            "INVALID_REGRESSION_SCHEMA",
            "expected",
            "expected failure does not match source",
        ));
    }
    let before: Value = json_read(&directory.join("snapshots/before.json"))?;
    validate_snapshots(&before)?;
    let snapshot = if case.reconstruction == Reconstruction::SnapshotReplay {
        Some(
            dg_snapshot::load(&directory.join("snapshots/source.json"))
                .map_err(|e| error(&e.code, e.context, e.message))?,
        )
    } else {
        None
    };
    let text = String::from_utf8(read(
        &directory.join("manifest.toml"),
        "REGRESSION_NOT_FOUND",
    )?)
    .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?;
    let config: toml::Value =
        toml::from_str(&text).map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?;
    check_secrets(
        &serde_json::to_value(config)
            .map_err(|e| error("INVALID_REGRESSION_SCHEMA", "manifest", e))?,
    )?;
    let manifest = project_manifest(&text)?;
    check_identities(&manifest)?;
    if manifest.project.name != case.project {
        return Err(error(
            "INVALID_REGRESSION_SCHEMA",
            "project",
            "project differs from case",
        ));
    }
    let definitions = match &case.source {
        Source::Preservation(_) => {
            if manifest.preservation.len() != 1 || !manifest.scenario.is_empty() {
                return Err(error(
                    "SOURCE_FINDING_NOT_FOUND",
                    "manifest",
                    "exactly one target rule required",
                ));
            }
            json!(&manifest.preservation[0])
        }
        Source::Lifecycle(_) => {
            if manifest.scenario.len() != 1 || !manifest.preservation.is_empty() {
                return Err(error(
                    "SOURCE_SCENARIO_NOT_FOUND",
                    "manifest",
                    "exactly one target scenario required",
                ));
            }
            json!(&manifest.scenario[0])
        }
    };
    if definitions != case.definition {
        return Err(error(
            "HASH_MISMATCH",
            "definition",
            "manifest target differs from captured definition",
        ));
    }
    let artifacts: Artifacts = json_read(&directory.join("artifacts.json"))?;
    if artifacts.len() != 4 {
        return Err(error(
            "INVALID_REGRESSION_SCHEMA",
            "artifacts",
            "four artifact identities required",
        ));
    }
    for (role, path) in artifact_paths(&manifest) {
        if !path.is_absolute() || artifacts.get(role).is_none_or(|a| a.path != path) {
            return Err(error(
                "INVALID_REGRESSION_SCHEMA",
                role,
                "artifact path differs from copied manifest",
            ));
        }
    }
    Ok(Loaded {
        case,
        expected,
        before,
        artifacts,
        manifest,
        snapshot,
    })
}

pub fn reproduce(directory: &Path, candidate: &CandidateOverride) -> Result<ReplayReport> {
    let mut loaded = load(directory)?;
    if candidate.idl.is_some() && candidate.binary.is_none() {
        return Err(error(
            "CANDIDATE_INCOMPATIBLE",
            "candidate",
            "IDL override requires an explicit binary override",
        ));
    }
    for (role, artifact) in &loaded.artifacts {
        if (role == "v2_binary" && candidate.binary.is_some())
            || (role == "v2_idl" && candidate.idl.is_some())
        {
            continue;
        }
        let code = if role.ends_with("binary") {
            "MISSING_BINARY"
        } else {
            "MISSING_IDL"
        };
        if sha256(&read(&artifact.path, code)?) != artifact.sha256 {
            return Err(error(
                "ARTIFACT_HASH_MISMATCH",
                role,
                "local artifact content changed",
            ));
        }
    }
    if let Some(path) = &candidate.binary {
        loaded.manifest.versions.v2.program_path = canonical(path, "MISSING_BINARY")?;
    }
    if let Some(path) = &candidate.idl {
        loaded.manifest.versions.v2.idl = canonical(path, "MISSING_IDL")?;
    }
    let effective = artifacts(&loaded.manifest)?;
    let report = execute(
        &loaded.manifest,
        &loaded.case.source,
        loaded.snapshot.as_ref(),
    )?;
    let (observed, _, before) = normalize(&report, &loaded.case.source)?;
    if before != loaded.before {
        return Err(error(
            "INITIAL_STATE_MISMATCH",
            "snapshots/before.json",
            "reconstructed V1 evidence differs from captured evidence",
        ));
    }
    if artifacts(&loaded.manifest)? != effective {
        return Err(error(
            "ARTIFACT_HASH_MISMATCH",
            "replay",
            "artifacts changed during execution",
        ));
    }
    let matched = observed == loaded.expected;
    Ok(ReplayReport {
        schema_version: 1,
        regression_id: loaded.case.id,
        result: if matched {
            ReplayStatus::Reproduced
        } else {
            ReplayStatus::NotReproduced
        },
        expected: loaded.expected,
        observed,
        initial_state_verified: true,
        artifacts: effective,
        diagnostics: if matched {
            vec![]
        } else {
            vec![error(
                "EXPECTED_FAILURE_NOT_REPRODUCED",
                "comparison",
                "execution completed but the normalized failure differs",
            )]
        },
        policy_evaluated: false,
    })
}
pub fn list(store: &Path) -> Result<Vec<RegressionCase>> {
    if !store.exists() {
        return Ok(vec![]);
    }
    let mut cases = vec![];
    for entry in
        fs::read_dir(store).map_err(|e| error("REGRESSION_NOT_FOUND", store.display(), e))?
    {
        let entry = entry.map_err(|e| error("REGRESSION_NOT_FOUND", store.display(), e))?;
        if entry
            .file_type()
            .map_err(|e| error("REGRESSION_NOT_FOUND", entry.path().display(), e))?
            .is_dir()
        {
            cases.push(load(&entry.path())?.case);
        }
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn captures_and_replays_preservation_and_lifecycle_failures() {
        let root = root();
        std::env::set_current_dir(&root).unwrap();
        let store = std::env::temp_dir().join(format!("dg-regression-{}", std::process::id()));
        let _ = fs::remove_dir_all(&store);

        let escrow = create(
            &root.join("report/escrow-broken-preservation.json"),
            &root.join("examples/escrow-runtime-broken/doppelganger.toml"),
            Source::Preservation("PRESERVE-amount-retained".into()),
            &store,
        )
        .unwrap();
        let first_replay = reproduce(&escrow.directory, &CandidateOverride::default()).unwrap();
        let second_replay = reproduce(&escrow.directory, &CandidateOverride::default()).unwrap();
        assert_eq!(first_replay, second_replay);
        assert_eq!(first_replay.result, ReplayStatus::Reproduced);
        let good_escrow = root.join("target/deploy/escrow-v2/dg_escrow_v2.so");
        assert_eq!(
            reproduce(
                &escrow.directory,
                &CandidateOverride {
                    binary: Some(good_escrow),
                    idl: None
                }
            )
            .unwrap()
            .result,
            ReplayStatus::NotReproduced
        );
        assert!(
            create(
                &root.join("report/escrow-broken-preservation.json"),
                &root.join("examples/escrow-runtime-broken/doppelganger.toml"),
                Source::Preservation("PRESERVE-amount-retained".into()),
                &store,
            )
            .unwrap()
            .already_exists
        );

        let vault = create(
            &root.join("report/lifecycle-broken.json"),
            &root.join("examples/vault-runtime-r8-broken/doppelganger.toml"),
            Source::Lifecycle("repeat-migration".into()),
            &store,
        )
        .unwrap();
        let first_replay = reproduce(&vault.directory, &CandidateOverride::default()).unwrap();
        let second_replay = reproduce(&vault.directory, &CandidateOverride::default()).unwrap();
        assert_eq!(first_replay, second_replay);
        assert_eq!(first_replay.result, ReplayStatus::Reproduced);
        let good_vault = root.join("target/deploy/v2/dg_vault_v2.so");
        assert_eq!(
            reproduce(
                &vault.directory,
                &CandidateOverride {
                    binary: Some(good_vault),
                    idl: None
                }
            )
            .unwrap()
            .result,
            ReplayStatus::NotReproduced
        );
        assert_eq!(list(&store).unwrap().len(), 2);

        let expected_path = escrow.directory.join("expected.json");
        let original = fs::read(&expected_path).unwrap();
        fs::write(&expected_path, b"{}").unwrap();
        assert!(matches!(load(&escrow.directory), Err(ref error) if error.code == "HASH_MISMATCH"));
        fs::write(expected_path, original).unwrap();
        fs::remove_dir_all(store).unwrap();
    }
}
