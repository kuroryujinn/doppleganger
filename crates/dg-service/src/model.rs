use crate::{Result, ServiceError};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub const SCHEMA_VERSION: u32 = 1;
pub const ENGINE_VERSION: &str = "0.1.0";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedUpload {
    pub schema_version: u32,
    pub mode: String,
    pub manifest_path: String,
    pub policy_path: String,
    pub snapshot_path: Option<String>,
}

impl PreparedUpload {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION || self.mode != "prepared" {
            return Err(ServiceError::new(
                "UNSUPPORTED_INPUT",
                "Only prepared upload schema 1 is enabled",
            ));
        }
        for path in [&self.manifest_path, &self.policy_path]
            .into_iter()
            .chain(self.snapshot_path.iter())
        {
            relative_path(path)?;
        }
        Ok(())
    }
}

/// Strict POSIX archive names; rejecting aliases avoids ambiguous identities.
pub fn relative_path(name: &str) -> Result<&Path> {
    let path = Path::new(name);
    if name.is_empty()
        || name.len() > 4096
        || name.contains(['\\', ':', '\0'])
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(ServiceError::new(
            "INVALID_PATH",
            "Expected a contained relative POSIX file path",
        ));
    }
    Ok(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Validating,
    Building,
    Running,
    Publishing,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Validating => "validating",
            Self::Building => "building",
            Self::Running => "running",
            Self::Publishing => "publishing",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "ERROR")]
    Error,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub schema_version: u32,
    pub job_id: String,
    pub submission_id: String,
    pub state: JobState,
    pub verdict: Option<Verdict>,
    pub engine_exit_code: Option<i32>,
    pub engine_run_id: Option<String>,
    pub engine_version: Option<String>,
    pub error_code: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct Claim {
    pub job_id: String,
    pub submission_id: String,
    pub lease_token: String,
    pub lease_expires_at_ms: i64,
}

#[derive(Clone, Copy, Debug)]
pub enum FailureReason {
    EngineProtocol,
    EngineInternal,
    Timeout,
    Storage,
    Sandbox,
    InputChanged,
}
impl FailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EngineProtocol => "ENGINE_PROTOCOL_ERROR",
            Self::EngineInternal => "ENGINE_INTERNAL_ERROR",
            Self::Timeout => "JOB_TIMEOUT",
            Self::Storage => "STORAGE_ERROR",
            Self::Sandbox => "SANDBOX_ERROR",
            Self::InputChanged => "INPUT_CHANGED",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Event {
    pub cursor: i64,
    pub state: String,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkerSubmission {
    pub metadata: PreparedUpload,
    pub storage_key: String,
    pub sha256: String,
    pub bytes: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub kind: String,
    pub storage_key: String,
    pub sha256: String,
    pub bytes: i64,
}
