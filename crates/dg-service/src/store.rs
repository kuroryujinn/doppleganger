use crate::{intake::ReceivedArchive, model::*, publish::ValidatedReport, Result, ServiceError};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
    },
    Row, SqlitePool,
};
use std::{path::Path, time::Duration};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

fn id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}
fn valid_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 128
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn validate_key(key: Option<&str>) -> Result<()> {
    if key
        .is_some_and(|k| k.is_empty() || k.len() > 128 || !k.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err(ServiceError::new(
            "INVALID_IDEMPOTENCY_KEY",
            "Use 1–128 printable ASCII characters",
        ));
    }
    Ok(())
}

fn row_job(row: SqliteRow) -> Result<Job> {
    let state: String = row.try_get("state")?;
    let verdict: Option<String> = row.try_get("verdict")?;
    let decode = |text: &str| {
        serde_json::from_value(serde_json::Value::String(text.into()))
            .map_err(|_| ServiceError::new("STORAGE_ERROR", "Unknown persisted job state"))
    };
    Ok(Job {
        schema_version: 1,
        job_id: row.try_get("id")?,
        submission_id: row.try_get("submission_id")?,
        state: decode(&state)?,
        verdict: verdict
            .map(|v| serde_json::from_value(serde_json::Value::String(v)))
            .transpose()
            .map_err(|_| ServiceError::new("STORAGE_ERROR", "Unknown persisted verdict"))?,
        engine_exit_code: row.try_get("engine_exit_code")?,
        engine_run_id: row.try_get("engine_run_id")?,
        engine_version: row.try_get("engine_version")?,
        error_code: row.try_get("error_code")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

impl Store {
    /// Operator-selected private local database; never accept this path from an upload.
    pub async fn open(path: &Path) -> Result<Self> {
        if !path.is_absolute()
            || path.parent().is_none_or(|p| !p.is_dir())
            || std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file())
        {
            return Err(ServiceError::new(
                "INVALID_STORAGE",
                "Database requires an absolute regular-file path in an existing private directory",
            ));
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        let mut tx = pool.begin().await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *tx)
            .await?;
        match version {
            0 => {
                let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
                    .fetch_one(&mut *tx).await?;
                if tables != 0 {
                    return Err(ServiceError::new(
                        "UNSUPPORTED_STORAGE",
                        "Unversioned existing database cannot be upgraded implicitly",
                    ));
                }
                sqlx::raw_sql(include_str!("../migrations/0001_initial.sql"))
                    .execute(&mut *tx)
                    .await?;
            }
            1 => {}
            2 | 3 => {}
            _ => {
                return Err(ServiceError::new(
                    "UNSUPPORTED_STORAGE",
                    "Unsupported database schema version",
                ))
            }
        }
        if version < 2 {
            sqlx::raw_sql(include_str!("../migrations/0002_worker.sql"))
                .execute(&mut *tx)
                .await?;
        }
        if version < 3 {
            sqlx::raw_sql(include_str!("../migrations/0003_api.sql"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(Self { pool })
    }

    /// Register only a completely received, synced and atomically published archive.
    pub async fn add_submission(
        &self,
        owner: &str,
        metadata: &PreparedUpload,
        archive: &ReceivedArchive,
        now: i64,
    ) -> Result<String> {
        metadata.validate()?;
        if !valid_owner(owner)
            || archive.bytes <= 0
            || archive.bytes > 384 * 1024 * 1024
            || archive.sha256.len() != 64
        {
            return Err(ServiceError::new(
                "INVALID_SUBMISSION",
                "Invalid owner, archive size or SHA-256",
            ));
        }
        let submission = archive.id.clone();
        let metadata = serde_json::to_string(metadata).map_err(|_| {
            ServiceError::new("INVALID_SUBMISSION", "Cannot encode upload metadata")
        })?;
        sqlx::query("INSERT INTO submissions(id,owner,metadata,archive_sha256,storage_key,bytes,created_at_ms) VALUES(?,?,?,?,?,?,?)")
            .bind(&submission).bind(owner).bind(metadata).bind(&archive.sha256).bind(&archive.storage_key).bind(archive.bytes).bind(now).execute(&self.pool).await?;
        Ok(submission)
    }

    pub async fn submission(
        &self,
        owner: &str,
        submission: &str,
    ) -> Result<Option<PreparedUpload>> {
        let metadata: Option<String> =
            sqlx::query_scalar("SELECT metadata FROM submissions WHERE id=? AND owner=?")
                .bind(submission)
                .bind(owner)
                .fetch_optional(&self.pool)
                .await?;
        metadata
            .map(|v| {
                serde_json::from_str(&v).map_err(|_| {
                    ServiceError::new("STORAGE_ERROR", "Invalid persisted upload metadata")
                })
            })
            .transpose()
    }

    pub async fn idempotent_job(
        &self,
        owner: &str,
        submission: &str,
        key: Option<&str>,
    ) -> Result<Option<Job>> {
        validate_key(key)?;
        let Some(key) = key else { return Ok(None) };
        let row = sqlx::query("SELECT * FROM jobs WHERE owner=? AND idempotency_key=?")
            .bind(owner)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(row) = row {
            if row.try_get::<String, _>("request_digest")?
                != format!("{:x}", Sha256::digest(submission.as_bytes()))
            {
                return Err(ServiceError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "Key already belongs to different input",
                ));
            }
            Ok(Some(row_job(row)?))
        } else {
            Ok(None)
        }
    }
    pub async fn enqueue(
        &self,
        owner: &str,
        submission: &str,
        key: Option<&str>,
        now: i64,
    ) -> Result<Job> {
        validate_key(key)?;
        if self.submission(owner, submission).await?.is_none() {
            return Err(ServiceError::new("NOT_FOUND", "Submission not found"));
        }
        let digest = format!("{:x}", Sha256::digest(submission.as_bytes()));
        let mut tx = self.pool.begin().await?;
        // One INSERT evaluates quotas and creates the reservation under SQLite's writer lock.
        let inserted: Option<String> = sqlx::query_scalar("INSERT INTO jobs(id,owner,submission_id,state,created_at_ms,updated_at_ms,idempotency_key,request_digest) SELECT ?,?,?,'queued',?,?,?,? WHERE EXISTS(SELECT 1 FROM submissions WHERE id=? AND owner=? AND created_at_ms>?) AND (SELECT count(*) FROM jobs WHERE state='queued')<16 AND (SELECT count(*) FROM jobs WHERE state='queued' AND owner=?)<4 ON CONFLICT(owner,idempotency_key) DO NOTHING RETURNING id")
            .bind(id("job")).bind(owner).bind(submission).bind(now).bind(now).bind(key).bind(&digest).bind(submission).bind(owner).bind(now.saturating_sub(604_800_000)).bind(owner)
            .fetch_optional(&mut *tx).await?;
        let job_id = if let Some(job) = inserted {
            sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,'queued',?)")
                .bind(&job)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            job
        } else if let Some(row) =
            sqlx::query("SELECT id,request_digest FROM jobs WHERE owner=? AND idempotency_key=?")
                .bind(owner)
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?
        {
            if row.try_get::<String, _>("request_digest")? != digest {
                return Err(ServiceError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "Key already belongs to different input",
                ));
            }
            row.try_get("id")?
        } else {
            let created: Option<i64> =
                sqlx::query_scalar("SELECT created_at_ms FROM submissions WHERE id=? AND owner=?")
                    .bind(submission)
                    .bind(owner)
                    .fetch_optional(&mut *tx)
                    .await?;
            if created.is_none() {
                return Err(ServiceError::new("NOT_FOUND", "Submission not found"));
            }
            if created.is_some_and(|c| c <= now.saturating_sub(604_800_000)) {
                return Err(ServiceError::new(
                    "SUBMISSION_EXPIRED",
                    "Upload a new submission",
                ));
            }
            return Err(ServiceError::new("QUEUE_FULL", "Queue quota reached"));
        };
        tx.commit().await?;
        self.job(owner, &job_id)
            .await?
            .ok_or_else(|| ServiceError::new("STORAGE_ERROR", "Committed job unavailable"))
    }

    pub async fn job(&self, owner: &str, job: &str) -> Result<Option<Job>> {
        sqlx::query("SELECT * FROM jobs WHERE id=? AND owner=?")
            .bind(job)
            .bind(owner)
            .fetch_optional(&self.pool)
            .await?
            .map(row_job)
            .transpose()
    }

    pub async fn events(&self, owner: &str, job: &str, after: i64) -> Result<Vec<Event>> {
        if after < 0 {
            return Err(ServiceError::new(
                "INVALID_CURSOR",
                "Cursor must be nonnegative",
            ));
        }
        if self.job(owner, job).await?.is_none() {
            return Err(ServiceError::new("NOT_FOUND", "Job not found"));
        }
        sqlx::query("SELECT e.cursor,e.state,e.created_at_ms,e.message FROM job_events e JOIN jobs j ON j.id=e.job_id WHERE j.id=? AND j.owner=? AND e.cursor>? ORDER BY e.cursor LIMIT 100")
            .bind(job).bind(owner).bind(after).fetch_all(&self.pool).await?.into_iter()
            .map(|r| Ok(Event { cursor:r.try_get("cursor")?, state:r.try_get("state")?, created_at_ms:r.try_get("created_at_ms")?, message:r.try_get("message")? })).collect()
    }

    pub async fn worker_submission(
        &self,
        claim: &Claim,
        now: i64,
    ) -> Result<Option<WorkerSubmission>> {
        let row=sqlx::query("SELECT s.* FROM submissions s JOIN jobs j ON j.submission_id=s.id WHERE j.id=? AND j.lease_token=? AND j.lease_expires_at_ms>? AND j.state IN ('validating','running','publishing','cancelling')")
            .bind(&claim.job_id).bind(&claim.lease_token).bind(now).fetch_optional(&self.pool).await?;
        row.map(|r| {
            Ok(WorkerSubmission {
                metadata: serde_json::from_str(&r.try_get::<String, _>("metadata")?).map_err(
                    |_| ServiceError::new("STORAGE_ERROR", "Invalid stored upload metadata"),
                )?,
                storage_key: r.try_get("storage_key")?,
                sha256: r.try_get("archive_sha256")?,
                bytes: r.try_get("bytes")?,
            })
        })
        .transpose()
    }

    pub async fn claim_state(&self, claim: &Claim) -> Result<Option<JobState>> {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM jobs WHERE id=? AND lease_token=?")
                .bind(&claim.job_id)
                .bind(&claim.lease_token)
                .fetch_optional(&self.pool)
                .await?;
        state
            .map(|s| {
                serde_json::from_value(serde_json::Value::String(s))
                    .map_err(|_| ServiceError::new("STORAGE_ERROR", "Invalid persisted state"))
            })
            .transpose()
    }

    /// Fixed operator codes only; raw container stderr never becomes an API event.
    pub async fn note(&self, claim: &Claim, code: &str, now: i64) -> Result<()> {
        if !matches!(
            code,
            "LOG_TRUNCATED" | "PORTABLE_CASE_UNAVAILABLE" | "ARTIFACT_COLLECTION_FAILED"
        ) {
            return Err(ServiceError::new("INVALID_EVENT", "Unknown worker event"));
        }
        sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms,message) SELECT id,state,?,? FROM jobs WHERE id=? AND lease_token=? AND lease_expires_at_ms>? AND state IN ('validating','running','publishing')")
            .bind(now).bind(code).bind(&claim.job_id).bind(&claim.lease_token).bind(now).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn claim(&self, now: i64) -> Result<Option<Claim>> {
        let expiry = now
            .checked_add(30_000)
            .filter(|_| now >= 0)
            .ok_or_else(|| ServiceError::new("INVALID_CLOCK", "Invalid worker timestamp"))?;
        let mut tx = self.pool.begin().await?;
        let token = id("lease");
        let row = sqlx::query("UPDATE jobs SET state='validating',lease_token=?,lease_expires_at_ms=?,updated_at_ms=? WHERE id=(SELECT id FROM jobs WHERE state='queued' ORDER BY created_at_ms,id LIMIT 1) AND NOT EXISTS(SELECT 1 FROM jobs WHERE state IN ('validating','building','running','publishing','cancelling')) RETURNING id,submission_id")
            .bind(&token).bind(expiry).bind(now).fetch_optional(&mut *tx).await?;
        let claim = if let Some(row) = row {
            let job: String = row.try_get("id")?;
            sqlx::query(
                "INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,'validating',?)",
            )
            .bind(&job)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            Some(Claim {
                job_id: job,
                submission_id: row.try_get("submission_id")?,
                lease_token: token,
                lease_expires_at_ms: expiry,
            })
        } else {
            None
        };
        tx.commit().await?;
        Ok(claim)
    }

    pub async fn heartbeat(&self, claim: &Claim, now: i64) -> Result<bool> {
        let expiry = now
            .checked_add(30_000)
            .filter(|_| now >= 0)
            .ok_or_else(|| ServiceError::new("INVALID_CLOCK", "Invalid worker timestamp"))?;
        Ok(sqlx::query("UPDATE jobs SET lease_expires_at_ms=?,updated_at_ms=? WHERE id=? AND lease_token=? AND lease_expires_at_ms>? AND state IN ('validating','building','running','publishing','cancelling')")
            .bind(expiry).bind(now).bind(&claim.job_id).bind(&claim.lease_token).bind(now).execute(&self.pool).await?.rows_affected()==1)
    }

    /// Trusted worker recovery inventory, never exposed through owner-facing routes.
    pub async fn expired_claims(&self, now: i64) -> Result<Vec<Claim>> {
        sqlx::query("SELECT id,submission_id,lease_token,lease_expires_at_ms FROM jobs WHERE lease_expires_at_ms<=? AND state IN ('validating','building','running','publishing','cancelling') ORDER BY lease_expires_at_ms LIMIT 100")
            .bind(now).fetch_all(&self.pool).await?.into_iter().map(|row| Ok(Claim {
                job_id:row.try_get("id")?,submission_id:row.try_get("submission_id")?,lease_token:row.try_get("lease_token")?,lease_expires_at_ms:row.try_get("lease_expires_at_ms")?
            })).collect()
    }

    /// Call only after the attempt is confirmed stopped. Cancellation wins its race.
    pub async fn fail_attempt(
        &self,
        claim: &Claim,
        reason: FailureReason,
        now: i64,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let exit = matches!(reason, FailureReason::EngineInternal).then_some(4i32);
        let updated=sqlx::query("UPDATE jobs SET state='failed',verdict='ERROR',error_code=?,engine_exit_code=?,updated_at_ms=?,lease_token=NULL,lease_expires_at_ms=NULL WHERE id=? AND lease_token=? AND lease_expires_at_ms>? AND state IN ('validating','running','publishing')")
            .bind(reason.as_str()).bind(exit).bind(now).bind(&claim.job_id).bind(&claim.lease_token).bind(now).execute(&mut *tx).await?.rows_affected()==1;
        if updated {
            sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,'failed',?)")
                .bind(&claim.job_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn advance(
        &self,
        claim: &Claim,
        from: JobState,
        to: JobState,
        now: i64,
    ) -> Result<bool> {
        if !matches!(
            (from, to),
            (JobState::Validating, JobState::Running)
                | (JobState::Validating, JobState::Publishing)
                | (JobState::Running, JobState::Publishing)
        ) {
            return Err(ServiceError::new(
                "INVALID_TRANSITION",
                "Transition is not allowed for prepared jobs",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query("UPDATE jobs SET state=?,updated_at_ms=? WHERE id=? AND lease_token=? AND lease_expires_at_ms>? AND state=?")
            .bind(to.as_str()).bind(now).bind(&claim.job_id).bind(&claim.lease_token).bind(now).bind(from.as_str()).execute(&mut *tx).await?.rows_affected()==1;
        if updated {
            sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,?,?)")
                .bind(&claim.job_id)
                .bind(to.as_str())
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn cancel(&self, owner: &str, job: &str, now: i64) -> Result<Job> {
        let mut tx = self.pool.begin().await?;
        let updated: Option<String> = sqlx::query_scalar("UPDATE jobs SET state=CASE WHEN state='queued' THEN 'cancelled' ELSE 'cancelling' END,updated_at_ms=? WHERE id=? AND owner=? AND state IN ('queued','validating','building','running','publishing') RETURNING state")
            .bind(now).bind(job).bind(owner).fetch_optional(&mut *tx).await?;
        if let Some(state) = updated {
            sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,?,?)")
                .bind(job)
                .bind(state)
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let job = self
            .job(owner, job)
            .await?
            .ok_or_else(|| ServiceError::new("NOT_FOUND", "Job not found"))?;
        if matches!(job.state, JobState::Completed | JobState::Failed) {
            return Err(ServiceError::new("JOB_TERMINAL", "Job already finished"));
        }
        Ok(job)
    }

    /// Only call after the associated container/process tree is confirmed stopped.
    pub async fn acknowledge_cancel(&self, claim: &Claim, now: i64) -> Result<bool> {
        self.stop(claim, "cancelled", None, now, false).await
    }

    /// Only call after stopping/reconciling the old container. Expired claims stay reserved until then.
    pub async fn reconcile_lost(&self, claim: &Claim, now: i64) -> Result<bool> {
        self.stop(claim, "failed", Some("WORKER_LOST"), now, true)
            .await
    }

    async fn stop(
        &self,
        claim: &Claim,
        state: &str,
        error: Option<&str>,
        now: i64,
        expired: bool,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let updated=sqlx::query("UPDATE jobs SET state=?,verdict=?,error_code=?,updated_at_ms=?,lease_token=NULL,lease_expires_at_ms=NULL WHERE id=? AND lease_token=? AND ((?=1 AND lease_expires_at_ms<=? AND state IN ('validating','building','running','publishing','cancelling')) OR (?=0 AND state='cancelling'))")
            .bind(state).bind(error.map(|_| "ERROR")).bind(error).bind(now).bind(&claim.job_id).bind(&claim.lease_token).bind(expired).bind(now).bind(expired)
            .execute(&mut *tx).await?.rows_affected()==1;
        if updated {
            sqlx::query("INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,?,?)")
                .bind(&claim.job_id)
                .bind(state)
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// Artifacts must already be atomically published. Cancellation/lease loss fences publication.
    pub async fn complete(
        &self,
        claim: &Claim,
        report: &ValidatedReport,
        artifacts: &[Artifact],
        now: i64,
    ) -> Result<bool> {
        let prefix = format!("jobs/{}/", claim.job_id);
        if report.state() == JobState::Failed
            || artifacts.iter().any(|a| {
                a.bytes < 0
                    || a.sha256.len() != 64
                    || !a
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    || !a.storage_key.starts_with(&prefix)
                    || relative_path(&a.storage_key).is_err()
            })
        {
            return Err(ServiceError::new(
                "INVALID_PUBLICATION",
                "Invalid completed report or artifact metadata",
            ));
        }
        if !artifacts.iter().any(|a| a.kind == "report_json") {
            return Err(ServiceError::new(
                "INVALID_PUBLICATION",
                "Completed result requires a published report",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query("UPDATE jobs SET state='completed',verdict=?,engine_exit_code=?,engine_run_id=?,engine_version=?,updated_at_ms=?,lease_token=NULL,lease_expires_at_ms=NULL WHERE id=? AND lease_token=? AND lease_expires_at_ms>? AND state='publishing'")
            .bind(report.verdict().as_str()).bind(report.engine_exit_code()).bind(report.run_id()).bind(report.engine_version()).bind(now).bind(&claim.job_id).bind(&claim.lease_token).bind(now)
            .execute(&mut *tx).await?.rows_affected()==1;
        if updated {
            for a in artifacts {
                sqlx::query("INSERT INTO job_artifacts(job_id,kind,storage_key,sha256,bytes) VALUES(?,?,?,?,?)")
                    .bind(&claim.job_id).bind(&a.kind).bind(&a.storage_key).bind(&a.sha256).bind(a.bytes).execute(&mut *tx).await?;
            }
            sqlx::query(
                "INSERT INTO job_events(job_id,state,created_at_ms) VALUES(?,'completed',?)",
            )
            .bind(&claim.job_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn artifacts(&self, owner: &str, job: &str) -> Result<Vec<Artifact>> {
        if self.job(owner, job).await?.is_none() {
            return Err(ServiceError::new("NOT_FOUND", "Job not found"));
        }
        sqlx::query("SELECT a.* FROM job_artifacts a JOIN jobs j ON j.id=a.job_id WHERE j.id=? AND j.owner=? AND j.state='completed' ORDER BY a.kind")
            .bind(job).bind(owner).fetch_all(&self.pool).await?.into_iter().map(|r| Ok(Artifact {
                kind:r.try_get("kind")?,storage_key:r.try_get("storage_key")?,sha256:r.try_get("sha256")?,bytes:r.try_get("bytes")?
            })).collect()
    }
    pub async fn submission_info(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT * FROM submissions WHERE id=? AND owner=?")
            .bind(id)
            .bind(owner)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r|->Result<_>{
            let metadata:PreparedUpload=serde_json::from_str(&r.try_get::<String,_>("metadata")?).map_err(|_|ServiceError::new("STORAGE_ERROR","Invalid persisted metadata"))?;
            let created:i64=r.try_get("created_at_ms")?;
            Ok(serde_json::json!({"schema_version":1,"submission_id":r.try_get::<String,_>("id")?,"metadata":metadata,"sha256":r.try_get::<String,_>("archive_sha256")?,"bytes":r.try_get::<i64,_>("bytes")?,"created_at_ms":created,"expires_at_ms":created.checked_add(604_800_000).ok_or_else(||ServiceError::new("STORAGE_ERROR","Invalid creation timestamp"))?}))
        }).transpose()
    }
    pub async fn jobs(
        &self,
        owner: &str,
        after: Option<&str>,
    ) -> Result<(Vec<Job>, Option<String>)> {
        let (stamp, key) = if let Some(key) = after {
            let job = self
                .job(owner, key)
                .await?
                .ok_or_else(|| ServiceError::new("INVALID_CURSOR", "Unknown owner cursor"))?;
            (job.created_at_ms, job.job_id)
        } else {
            (-1, String::new())
        };
        let rows=sqlx::query("SELECT * FROM jobs WHERE owner=? AND (created_at_ms>? OR (created_at_ms=? AND id>?)) ORDER BY created_at_ms,id LIMIT 101")
            .bind(owner).bind(stamp).bind(stamp).bind(key).fetch_all(&self.pool).await?;
        let mut jobs = rows.into_iter().map(row_job).collect::<Result<Vec<_>>>()?;
        let more = jobs.len() > 100;
        jobs.truncate(100);
        let cursor = if more {
            jobs.last().map(|j| j.job_id.clone())
        } else {
            None
        };
        Ok((jobs, cursor))
    }
    pub async fn record_worker_ready(&self, ready: bool, now: i64) -> Result<()> {
        sqlx::query("INSERT INTO worker_status VALUES(1,?,?) ON CONFLICT(singleton) DO UPDATE SET ready=excluded.ready,observed_at_ms=excluded.observed_at_ms").bind(ready).bind(now).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn worker_ready(&self, now: i64) -> Result<bool> {
        Ok(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM worker_status WHERE singleton=1 AND ready=1 AND observed_at_ms>? AND observed_at_ms<=?").bind(now-15_000).bind(now).fetch_one(&self.pool).await?==1)
    }
    pub async fn upload_quota(&self, owner: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM submissions WHERE owner=?")
                .bind(owner)
                .fetch_one(&self.pool)
                .await?
                < 64,
        )
    }
    pub async fn delete_submission(&self, owner: &str, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let removed=sqlx::query("DELETE FROM submissions WHERE id=? AND owner=? AND NOT EXISTS(SELECT 1 FROM jobs WHERE submission_id=?)").bind(id).bind(owner).bind(id).execute(&mut *tx).await?.rows_affected();
        if removed == 0 {
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM submissions WHERE id=? AND owner=?")
                    .bind(id)
                    .bind(owner)
                    .fetch_optional(&mut *tx)
                    .await?;
            return Err(ServiceError::new(
                if exists.is_some() {
                    "SUBMISSION_IN_USE"
                } else {
                    "NOT_FOUND"
                },
                "Submission unavailable or retained by a job",
            ));
        }
        sqlx::query("INSERT INTO deletions VALUES('submission',?)")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn delete_job(&self, owner: &str, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM job_artifacts WHERE job_id IN (SELECT id FROM jobs WHERE id=? AND owner=? AND state IN ('completed','failed','cancelled'))").bind(id).bind(owner).execute(&mut *tx).await?;
        let removed=sqlx::query("DELETE FROM jobs WHERE id=? AND owner=? AND state IN ('completed','failed','cancelled')").bind(id).bind(owner).execute(&mut *tx).await?.rows_affected();
        if removed == 0 {
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM jobs WHERE id=? AND owner=?")
                    .bind(id)
                    .bind(owner)
                    .fetch_optional(&mut *tx)
                    .await?;
            return Err(ServiceError::new(
                if exists.is_some() {
                    "JOB_ACTIVE"
                } else {
                    "NOT_FOUND"
                },
                "Job unavailable or still active",
            ));
        }
        sqlx::query("INSERT INTO deletions VALUES('job',?)")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn pending_deletions(&self) -> Result<Vec<(String, String)>> {
        sqlx::query("SELECT kind,id FROM deletions ORDER BY kind,id LIMIT 100")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| Ok((r.try_get("kind")?, r.try_get("id")?)))
            .collect()
    }
    pub async fn finish_deletion(&self, kind: &str, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM deletions WHERE kind=? AND id=?")
            .bind(kind)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
