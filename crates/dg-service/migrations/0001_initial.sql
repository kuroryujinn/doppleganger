CREATE TABLE IF NOT EXISTS submissions (
    id TEXT PRIMARY KEY, owner TEXT NOT NULL,
    metadata TEXT NOT NULL CHECK(json_valid(metadata)),
    archive_sha256 TEXT NOT NULL CHECK(length(archive_sha256) = 64),
    storage_key TEXT NOT NULL UNIQUE, bytes INTEGER NOT NULL CHECK(bytes > 0),
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY, owner TEXT NOT NULL,
    submission_id TEXT NOT NULL REFERENCES submissions(id),
    state TEXT NOT NULL CHECK(state IN ('queued','validating','building','running','publishing','cancelling','completed','failed','cancelled')),
    verdict TEXT CHECK(verdict IN ('PASS','FAIL','ERROR')),
    engine_exit_code INTEGER CHECK(engine_exit_code BETWEEN 0 AND 4),
    engine_run_id TEXT, engine_version TEXT, error_code TEXT,
    created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
    lease_token TEXT, lease_expires_at_ms INTEGER,
    idempotency_key TEXT, request_digest TEXT NOT NULL,
    UNIQUE(owner, idempotency_key),
    CHECK((state IN ('completed','failed','cancelled')) OR verdict IS NULL)
);
CREATE INDEX IF NOT EXISTS jobs_queue ON jobs(state, created_at_ms, id);
CREATE INDEX IF NOT EXISTS jobs_owner ON jobs(owner, created_at_ms, id);
CREATE TABLE IF NOT EXISTS job_events (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    state TEXT NOT NULL, created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS events_job ON job_events(job_id, cursor);
CREATE TABLE IF NOT EXISTS job_artifacts (
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('report_json','report_html','summary','portable_case')),
    storage_key TEXT NOT NULL, sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
    bytes INTEGER NOT NULL CHECK(bytes >= 0), PRIMARY KEY(job_id, kind)
);
PRAGMA user_version = 1;
