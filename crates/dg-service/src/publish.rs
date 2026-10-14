//! Validate engine output before making it an authoritative service result.
use crate::{
    model::{JobState, Verdict, ENGINE_VERSION},
    Result, ServiceError,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub struct ValidatedReport {
    report: Value,
    exit: i32,
    verdict: Verdict,
}

fn invalid() -> ServiceError {
    ServiceError::new(
        "ENGINE_PROTOCOL_ERROR",
        "Engine report is missing, unsupported or inconsistent with its process exit",
    )
}

impl ValidatedReport {
    /// Expected bytes from the pinned engine, not a client-supplied report.
    pub fn parse(process_exit: Option<i32>, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 128 * 1024 * 1024 {
            return Err(ServiceError::new(
                "OUTPUT_TOO_LARGE",
                "Engine report exceeds publication budget",
            ));
        }
        let exit = process_exit
            .filter(|e| (0..=4).contains(e))
            .ok_or_else(invalid)?;
        let report: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let verdict = match exit {
            0 => Verdict::Pass,
            1 => Verdict::Fail,
            _ => Verdict::Error,
        };
        if report["schema_version"] != 1
            || report["exit_code"] != exit
            || report["result"] != verdict.as_str()
        {
            return Err(invalid());
        }
        if report.get("origin").is_some()
            && !(report["origin"] == "backend_validation"
                && exit == 2
                && report.get("engine_version").is_none())
        {
            return Err(invalid());
        }
        if report.get("engine_version").is_some() {
            if report["engine_version"] != ENGINE_VERSION
                || report["workflow_version"] != 1
                || report["manifest_schema_version"] != 1
                || report["legacy_unversioned_manifest"] != false
                || report["remote_transactions_sent"] != false
                || report["rpc_write_calls"] != 0
                || !report["run_id"].as_str().is_some_and(|s| {
                    s.len() == 68
                        && s.starts_with("RUN-")
                        && s[4..]
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            {
                return Err(invalid());
            }
            for flag in [
                "migration_executed_locally",
                "preservation_evaluated",
                "lifecycle_evaluated",
                "policy_evaluated",
            ] {
                if report[flag] != true {
                    return Err(invalid());
                }
            }
            for key in [
                "analysis",
                "execution",
                "preservation",
                "relationships",
                "lifecycle",
                "policy",
                "artifacts",
            ] {
                if !report[key].is_object() {
                    return Err(invalid());
                }
            }
            let blocking = report["policy"]["blocking"].as_u64().ok_or_else(invalid)?;
            let errors = report["policy"]["evaluation_errors"]
                .as_u64()
                .ok_or_else(invalid)?;
            if report["policy"]["passed"] != (exit == 0)
                || report["policy"]["schema_version"] != 1
                || report["policy"]["policy_evaluated"] != true
                || (exit == 0 && (blocking != 0 || errors != 0))
                || (exit == 1 && (blocking == 0 || errors != 0))
                || (exit == 3 && errors == 0)
                || matches!(exit, 2 | 4)
            {
                return Err(invalid());
            }
        } else if !matches!(exit, 2..=4)
            || !report["diagnostics"].as_array().is_some_and(|a| {
                !a.is_empty()
                    && a.iter().all(|d| {
                        d.is_object() && d["message"].is_string() && d["exit_code"] == exit
                    })
            })
        {
            return Err(invalid());
        }
        Ok(Self {
            report,
            exit,
            verdict,
        })
    }

    pub fn state(&self) -> JobState {
        if self.exit == 4 {
            JobState::Failed
        } else {
            JobState::Completed
        }
    }
    pub fn verdict(&self) -> Verdict {
        self.verdict
    }
    pub fn exit_code(&self) -> i32 {
        self.exit
    }
    pub fn run_id(&self) -> Option<&str> {
        self.report["run_id"].as_str()
    }
    pub fn engine_exit_code(&self) -> Option<i32> {
        (self.report["origin"] != "backend_validation").then_some(self.exit)
    }
    pub fn engine_version(&self) -> Option<&str> {
        self.engine_exit_code().map(|_| ENGINE_VERSION)
    }

    pub fn backend_error(error: &ServiceError) -> Self {
        Self {
            exit: 2,
            verdict: Verdict::Error,
            report: json!({"schema_version":1,"result":"ERROR","exit_code":2,"origin":"backend_validation","diagnostics":[{"code":error.code,"message":error.message,"exit_code":2}]}),
        }
    }
    pub fn original(&self) -> &Value {
        &self.report
    }

    /// A derived client view: its digest intentionally differs from the original.
    pub fn client_view(&self, sandbox_root: &str) -> Result<Value> {
        if sandbox_root.is_empty() || !sandbox_root.starts_with('/') {
            return Err(ServiceError::new(
                "INVALID_PUBLICATION",
                "Expected a sandbox root",
            ));
        }
        fn clean(value: &mut Value, root: &str) {
            match value {
                Value::String(s) => {
                    *s = s.replace(root, "project");
                    if s.starts_with('/') {
                        *s = "[internal path]".into();
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        clean(v, root)
                    }
                }
                Value::Object(o) => {
                    for v in o.values_mut() {
                        clean(v, root)
                    }
                }
                _ => {}
            }
        }
        let mut report = self.report.clone();
        clean(&mut report, sandbox_root);
        let bytes = serde_json::to_vec(&report).map_err(|_| invalid())?;
        Ok(
            json!({"schema_version":1,"derived_view":true,"sha256":format!("{:x}",Sha256::digest(&bytes)),"report":report}),
        )
    }
}
