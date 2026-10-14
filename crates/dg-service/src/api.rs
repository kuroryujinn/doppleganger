//! Prepared-project HTTP adapter. Never parses ZIP/project data or controls Docker.
use crate::{
    intake::{Limits, UploadWriter},
    model::*,
    runner::{now, private_dir},
    store::Store,
    Result, ServiceError,
};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Read,
    net::SocketAddr,
    os::unix::fs::OpenOptionsExt,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub owner: String,
    pub sha256: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    pub storage_root: PathBuf,
    pub bind: SocketAddr,
    pub tokens: Vec<Credential>,
    #[serde(default)]
    pub cors_origins: Vec<String>,
    #[serde(default = "default_timeout")]
    pub request_timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    120
}
fn err(code: &str, message: &str) -> ServiceError {
    ServiceError::new(code, message)
}
fn digest(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0; 32];
    for (i, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        bytes[i] = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(bytes)
}
fn opaque(text: &str, prefix: &str) -> bool {
    text.strip_prefix(prefix).is_some_and(|s| {
        s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
impl ApiConfig {
    pub fn load(path: &FsPath) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let info = fs::symlink_metadata(path)?;
        if !info.is_file() || info.permissions().mode() & 0o077 != 0 {
            return Err(err(
                "INVALID_CONFIG",
                "API configuration must be a private regular file",
            ));
        }
        let config: Self = serde_json::from_slice(&dg_analysis::read_bounded(path, 64 * 1024)?)
            .map_err(|_| err("INVALID_CONFIG", "Invalid API configuration"))?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        if !self.storage_root.is_absolute()
            || !self.bind.ip().is_loopback()
            || self.request_timeout_seconds == 0
            || self.request_timeout_seconds > 120
            || self.tokens.is_empty()
            || self.tokens.len() > 128
            || self.cors_origins.len() > 32
        {
            return Err(err("INVALID_CONFIG","Require private absolute storage, loopback bind, tokens and a timeout of 1–120 seconds"));
        }
        let mut seen = HashSet::new();
        for token in &self.tokens {
            if token.owner.is_empty()
                || token.owner.len() > 128
                || !token
                    .owner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || digest(&token.sha256).is_none()
                || !seen.insert(&token.sha256)
            {
                return Err(err(
                    "INVALID_CONFIG",
                    "Invalid owner or duplicate/invalid token digest",
                ));
            }
        }
        for origin in &self.cors_origins {
            let uri: axum::http::Uri = origin
                .parse()
                .map_err(|_| err("INVALID_CONFIG", "Invalid CORS origin"))?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none_or(|a| a.as_str().contains('@'))
                || uri.path() != "/"
                || uri.query().is_some()
                || origin.ends_with('/')
                || HeaderValue::from_str(origin).is_err()
            {
                return Err(err(
                    "INVALID_CONFIG",
                    "CORS origins must be explicit HTTP(S) origins",
                ));
            }
        }
        Ok(())
    }
}

pub struct ApiState {
    pub store: Store,
    config: ApiConfig,
    _lock: File,
    cleanup_lock: tokio::sync::Mutex<()>,
    requests: Arc<Semaphore>,
    downloads: Arc<Semaphore>,
    readers: Arc<Semaphore>,
    rates: Mutex<HashMap<String, (i64, u32)>>,
}
impl ApiState {
    pub async fn open(config: ApiConfig) -> Result<Arc<Self>> {
        config.validate()?;
        private_dir(&config.storage_root)?;
        for name in ["submissions", "jobs"] {
            private_dir(&config.storage_root.join(name))?;
        }
        let lock = local_lock(&config.storage_root.join("api.lock"), "API_BUSY")?;
        let store = Store::open(&config.storage_root.join("jobs.sqlite")).await?;
        let state = Arc::new(Self {
            store,
            config,
            _lock: lock,
            cleanup_lock: tokio::sync::Mutex::new(()),
            requests: Arc::new(Semaphore::new(64)),
            downloads: Arc::new(Semaphore::new(2)),
            readers: Arc::new(Semaphore::new(2)),
            rates: Mutex::new(HashMap::new()),
        });
        state.cleanup().await?;
        Ok(state)
    }
    pub async fn ready(&self) -> Result<bool> {
        let path = self.config.storage_root.join("worker.lock");
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            return Ok(false);
        }
        let lock = OpenOptions::new().read(true).write(true).open(path)?;
        if !matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
            return Ok(false);
        }
        if !self.store.worker_ready(now()).await? {
            return Ok(false);
        }
        Ok(self.capacity(0).is_ok())
    }
    fn capacity(&self, reserve: u64) -> Result<()> {
        let stat = rustix::fs::statvfs(&self.config.storage_root)
            .map_err(|_| err("STORAGE_ERROR", "Cannot inspect storage"))?;
        if stat.f_bavail.saturating_mul(stat.f_frsize) < 3 * 1024 * 1024 * 1024 + reserve
            || crate::runner::storage_usage(&self.config.storage_root)? + reserve
                > 19 * 1024 * 1024 * 1024
        {
            return Err(err(
                "STORAGE_FULL",
                "Storage reservation cannot be satisfied",
            ));
        }
        Ok(())
    }
    async fn cleanup(&self) -> Result<()> {
        let _guard = self.cleanup_lock.lock().await;
        loop {
            let pending = self.store.pending_deletions().await?;
            if pending.is_empty() {
                return Ok(());
            }
            for (kind, id) in pending {
                let (prefix, parent) = if kind == "job" {
                    ("job_", "jobs")
                } else {
                    ("sub_", "submissions")
                };
                if !opaque(&id, prefix) {
                    return Err(err("STORAGE_ERROR", "Invalid deletion identity"));
                }
                let path = self.config.storage_root.join(parent).join(&id);
                match fs::symlink_metadata(&path) {
                    Ok(_) => remove_private_tree(&path)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                File::open(self.config.storage_root.join(parent))?.sync_all()?;
                self.store.finish_deletion(&kind, &id).await?;
            }
        }
    }
}
fn local_lock(path: &FsPath, code: &str) -> Result<File> {
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) {
        return Err(err("INVALID_STORAGE", "Lock must be a regular file"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.try_lock()
        .map_err(|_| err(code, "Another operation owns this reservation"))?;
    Ok(file)
}
fn remove_private_tree(path: &FsPath) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(err("INVALID_STORAGE", "Deletion target cannot be a link"));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        let info = fs::symlink_metadata(&path)?;
        if info.is_dir() {
            remove_private_tree(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    fs::remove_dir(path)?;
    Ok(())
}

struct ApiError(ServiceError);
impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        Self(error)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code.as_str() {
            "UNAUTHORIZED" => StatusCode::UNAUTHORIZED,
            "NOT_FOUND" => StatusCode::NOT_FOUND,
            "ORIGIN_DENIED" => StatusCode::FORBIDDEN,
            "RESULT_NOT_READY"
            | "JOB_ACTIVE"
            | "SUBMISSION_IN_USE"
            | "JOB_TERMINAL"
            | "IDEMPOTENCY_CONFLICT"
            | "SUBMISSION_EXPIRED" => StatusCode::CONFLICT,
            "INPUT_TOO_LARGE" => StatusCode::PAYLOAD_TOO_LARGE,
            "UNSUPPORTED_INPUT" => StatusCode::UNPROCESSABLE_ENTITY,
            "QUEUE_FULL" | "UPLOAD_BUSY" | "RATE_LIMITED" | "SUBMISSION_QUOTA" | "REQUEST_BUSY" => {
                StatusCode::TOO_MANY_REQUESTS
            }
            "REQUEST_TIMEOUT" => StatusCode::REQUEST_TIMEOUT,
            "METHOD_NOT_ALLOWED" => StatusCode::METHOD_NOT_ALLOWED,
            "WORKER_UNAVAILABLE"
            | "STORAGE_FULL"
            | "STORAGE_ERROR"
            | "INVALID_STORAGE"
            | "ARTIFACT_INTEGRITY_ERROR"
            | "DOWNLOAD_BUSY" => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        };
        let id = format!("req_{}", uuid::Uuid::new_v4().simple());
        let correction = match status {
            StatusCode::UNAUTHORIZED => "Use an operator-provisioned bearer token",
            StatusCode::NOT_FOUND => "Check the identifier and authenticated owner",
            StatusCode::SERVICE_UNAVAILABLE => "Check service readiness or contact the operator",
            StatusCode::TOO_MANY_REQUESTS => {
                "Wait before retrying; remove unused submissions if quota is full"
            }
            StatusCode::CONFLICT => "Poll the job or correct the request before retrying",
            StatusCode::UNPROCESSABLE_ENTITY => {
                "Use prepared schema 1 with explicit V1/V2 artifacts"
            }
            StatusCode::PAYLOAD_TOO_LARGE => "Reduce the upload to the advertised limit",
            _ => "Use the documented request fields and contained relative paths",
        };
        let mut response=(status,Json(json!({"schema_version":1,"error":{"code":self.0.code,"message":self.0.message,"context":null,"suggested_correction":correction,"request_id":id}}))).into_response();
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&id).expect("generated request ID"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response.headers_mut().insert(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        );
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}
type HttpResult<T> = std::result::Result<T, ApiError>;
#[derive(Clone)]
struct Owner(String);

async fn guard(State(state): State<Arc<ApiState>>, mut request: Request, next: Next) -> Response {
    let health = matches!(request.uri().path(), "/health/ready" | "/health/live");
    let download = request.method() == Method::GET
        && (request.uri().path().ends_with("/report")
            || request.uri().path().contains("/artifacts/"));
    let result = (|| -> Result<()> {
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            if !state
                .config
                .cors_origins
                .iter()
                .any(|o| origin.as_bytes() == o.as_bytes())
            {
                return Err(err("ORIGIN_DENIED", "Origin is not allowed"));
            }
        }
        if request.uri().path().starts_with("/v1/") {
            let values = request.headers().get_all(header::AUTHORIZATION);
            if values.iter().count() != 1 {
                return Err(err("UNAUTHORIZED", "Valid bearer authentication required"));
            }
            let token = values
                .iter()
                .next()
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .filter(|v| {
                    v.len() >= 32 && v.len() <= 4096 && v.bytes().all(|b| b.is_ascii_graphic())
                })
                .ok_or_else(|| err("UNAUTHORIZED", "Valid bearer authentication required"))?;
            let received: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            let mut owner = None;
            for credential in &state.config.tokens {
                if bool::from(
                    received.ct_eq(&digest(&credential.sha256).expect("validated digest")),
                ) {
                    owner = Some(credential.owner.clone());
                }
            }
            let owner =
                owner.ok_or_else(|| err("UNAUTHORIZED", "Valid bearer authentication required"))?;
            let mut rates = state
                .rates
                .lock()
                .map_err(|_| err("STORAGE_ERROR", "Rate limiter unavailable"))?;
            let entry = rates.entry(owner.clone()).or_insert((now(), 0));
            if now() - entry.0 >= 60_000 {
                *entry = (now(), 0);
            }
            if entry.1 >= 120 {
                return Err(err("RATE_LIMITED", "Owner request rate exceeded"));
            }
            entry.1 += 1;
            request.extensions_mut().insert(Owner(owner));
            if let Some(length) = request.headers().get(header::CONTENT_LENGTH) {
                let length = length
                    .to_str()
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .ok_or_else(|| err("INVALID_REQUEST", "Invalid Content-Length"))?;
                let max = if request.uri().path() == "/v1/submissions" {
                    384 * 1024 * 1024 + 64 * 1024
                } else {
                    32 * 1024
                };
                if length > max {
                    return Err(err("INPUT_TOO_LARGE", "Request body exceeds route limit"));
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        return ApiError(error).into_response();
    }
    let Ok(_permit) = state.requests.clone().try_acquire_owned() else {
        return ApiError(err("REQUEST_BUSY", "Too many concurrent requests")).into_response();
    };
    let download_permit = if download {
        match state.downloads.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return ApiError(err("DOWNLOAD_BUSY", "Too many concurrent downloads"))
                    .into_response()
            }
        }
    } else {
        None
    };
    let mut response = match tokio::time::timeout(
        Duration::from_secs(state.config.request_timeout_seconds),
        next.run(request),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => ApiError(err(
            "REQUEST_TIMEOUT",
            "Request exceeded the receive/response deadline",
        ))
        .into_response(),
    };
    if !response.headers().contains_key("x-request-id") {
        if !health && (response.status().is_client_error() || response.status().is_server_error()) {
            let code = match response.status() {
                StatusCode::NOT_FOUND => "NOT_FOUND",
                StatusCode::METHOD_NOT_ALLOWED => "METHOD_NOT_ALLOWED",
                StatusCode::PAYLOAD_TOO_LARGE => "INPUT_TOO_LARGE",
                _ => "INVALID_REQUEST",
            };
            response =
                ApiError(err(code, "Request does not match the API contract")).into_response();
        } else {
            response.headers_mut().insert(
                "x-request-id",
                HeaderValue::from_str(&format!("req_{}", uuid::Uuid::new_v4().simple()))
                    .expect("request ID"),
            );
        }
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if let Some(permit) = download_permit {
        let (parts, body) = response.into_parts();
        let stream = futures_util::stream::unfold(
            (body.into_data_stream(), axum::body::Bytes::new(), permit),
            |(mut stream, mut pending, permit)| async move {
                if pending.is_empty() {
                    match stream.next().await {
                        Some(Ok(bytes)) => pending = bytes,
                        Some(Err(error)) => return Some((Err(error), (stream, pending, permit))),
                        None => return None,
                    }
                }
                let chunk = pending.split_to(pending.len().min(64 * 1024));
                Some((Ok(chunk), (stream, pending, permit)))
            },
        );
        response = Response::from_parts(parts, Body::from_stream(stream));
    }
    response
}
pub fn router(state: Arc<ApiState>) -> Router {
    let origins = state
        .config
        .cors_origins
        .iter()
        .map(|o| HeaderValue::from_str(o).expect("validated origin"))
        .collect::<Vec<_>>();
    Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"ok"})) }),
        )
        .route("/health/ready", get(ready))
        .route("/v1/capabilities", get(capabilities))
        .route(
            "/v1/submissions",
            post(upload).layer(DefaultBodyLimit::max(384 * 1024 * 1024 + 64 * 1024)),
        )
        .route(
            "/v1/submissions/{id}",
            get(submission).delete(delete_submission),
        )
        .route("/v1/jobs", post(enqueue).get(jobs))
        .route("/v1/jobs/{id}", get(job).delete(delete_job))
        .route("/v1/jobs/{id}/cancel", post(cancel))
        .route("/v1/jobs/{id}/events", get(events))
        .route("/v1/jobs/{id}/report", get(report))
        .route("/v1/jobs/{id}/artifacts/{kind}", get(download))
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([Method::GET, Method::POST, Method::DELETE])
                .allow_headers([
                    header::AUTHORIZATION,
                    header::CONTENT_TYPE,
                    axum::http::HeaderName::from_static("idempotency-key"),
                ])
                .expose_headers([
                    axum::http::HeaderName::from_static("x-request-id"),
                    header::ETAG,
                    header::RETRY_AFTER,
                ]),
        )
        .with_state(state)
}
async fn ready(State(state): State<Arc<ApiState>>) -> Response {
    if state.ready().await.unwrap_or(false) {
        (StatusCode::OK, Json(json!({"status":"ok"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"unavailable"})),
        )
            .into_response()
    }
}
async fn capabilities() -> Json<Value> {
    Json(
        json!({"schema_version":1,"modes":["prepared"],"engine_version":ENGINE_VERSION,"toolchains":[],"limits":{"upload_bytes":384*1024*1024,"extracted_bytes":512*1024*1024,"entry_bytes":128*1024*1024,"entries":10000,"queued_per_owner":4,"queued_global":16,"submissions_per_owner":64,"requests_per_owner_per_minute":120,"uploads_active_global":1,"output_bytes":128*1024*1024}}),
    )
}
async fn upload(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    request: Request,
) -> HttpResult<(StatusCode, Json<Value>)> {
    let _reservation = local_lock(
        &state.config.storage_root.join("upload.lock"),
        "UPLOAD_BUSY",
    )?;
    state.capacity(Limits::default().upload_bytes)?;
    if !state.store.upload_quota(&owner.0).await? {
        return Err(err("SUBMISSION_QUOTA", "Owner submission quota reached").into());
    }
    let mut multipart = Multipart::from_request(request, &state)
        .await
        .map_err(|_| {
            err(
                "INVALID_MULTIPART",
                "Expected metadata and archive multipart fields",
            )
        })?;
    let mut writer = UploadWriter::new(
        &state.config.storage_root.join("submissions"),
        Limits::default(),
    )?;
    let mut metadata = None;
    let mut archive = false;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("metadata") if metadata.is_none() => {
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    if bytes.len() + chunk.len() > 32 * 1024 {
                        return Err(err("INPUT_TOO_LARGE", "Metadata exceeds 32 KiB").into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| err("INVALID_METADATA", "Malformed metadata JSON"))?;
                if value["mode"] != "prepared" || value["schema_version"] != 1 {
                    return Err(err(
                        "UNSUPPORTED_INPUT",
                        "Only prepared upload schema 1 is enabled",
                    )
                    .into());
                }
                let prepared: PreparedUpload = serde_json::from_slice(&bytes).map_err(|_| {
                    err(
                        "INVALID_METADATA",
                        "Unknown, duplicate or missing metadata fields",
                    )
                })?;
                prepared.validate()?;
                metadata = Some(prepared);
            }
            Some("archive") if !archive => {
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    writer.write(&chunk)?;
                }
                archive = true;
            }
            _ => {
                return Err(err(
                    "INVALID_MULTIPART",
                    "Require exactly one metadata field and one archive field",
                )
                .into())
            }
        }
    }
    let metadata = metadata
        .filter(|_| archive)
        .ok_or_else(|| err("INVALID_MULTIPART", "Metadata or archive is missing"))?;
    let receipt = writer.finish()?;
    let id = state
        .store
        .add_submission(&owner.0, &metadata, &receipt, now())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .store
                .submission_info(&owner.0, &id)
                .await?
                .ok_or_else(|| err("STORAGE_ERROR", "Registered submission missing"))?,
        ),
    ))
}
fn multipart_error(error: axum::extract::multipart::MultipartError) -> ServiceError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        err("INPUT_TOO_LARGE", "Multipart body exceeds limit")
    } else {
        err(
            "INVALID_MULTIPART",
            "Multipart body is malformed or incomplete",
        )
    }
}
fn check_id(id: &str, prefix: &str) -> Result<()> {
    if opaque(id, prefix) {
        Ok(())
    } else {
        Err(err("NOT_FOUND", "Resource not found"))
    }
}
async fn submission(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> HttpResult<Json<Value>> {
    check_id(&id, "sub_")?;
    Ok(Json(
        state
            .store
            .submission_info(&owner.0, &id)
            .await?
            .ok_or_else(|| err("NOT_FOUND", "Submission not found"))?,
    ))
}
async fn delete_submission(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> HttpResult<StatusCode> {
    check_id(&id, "sub_")?;
    state.store.delete_submission(&owner.0, &id).await?;
    state.cleanup().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_job(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> HttpResult<StatusCode> {
    check_id(&id, "job_")?;
    state.store.delete_job(&owner.0, &id).await?;
    state.cleanup().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn job_view(state: &ApiState, owner: &str, job: Job) -> Result<Value> {
    let artifacts = state
        .store
        .artifacts(owner, &job.job_id)
        .await?
        .into_iter()
        .map(|a| json!({"kind":a.kind,"bytes":a.bytes,"sha256":a.sha256}))
        .collect::<Vec<_>>();
    let mut value =
        serde_json::to_value(job).map_err(|_| err("STORAGE_ERROR", "Cannot encode job"))?;
    value["artifacts"] = json!(artifacts);
    Ok(value)
}
async fn owned_job(state: &ApiState, owner: &str, id: &str) -> Result<Job> {
    check_id(id, "job_")?;
    state
        .store
        .job(owner, id)
        .await?
        .ok_or_else(|| err("NOT_FOUND", "Job not found"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobRequest {
    submission_id: String,
}
async fn enqueue(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    request: Request,
) -> HttpResult<(StatusCode, Json<Value>)> {
    let values = request.headers().get_all("idempotency-key");
    if values.iter().count() > 1 {
        return Err(err("INVALID_IDEMPOTENCY_KEY", "Use one idempotency key").into());
    }
    let key = values
        .iter()
        .next()
        .map(|v| {
            v.to_str()
                .map(str::to_owned)
                .map_err(|_| err("INVALID_IDEMPOTENCY_KEY", "Invalid idempotency key"))
        })
        .transpose()?;
    let Json(body) = Json::<JobRequest>::from_request(request, &state)
        .await
        .map_err(|_| err("INVALID_REQUEST", "Expected submission_id JSON"))?;
    check_id(&body.submission_id, "sub_")?;
    if state
        .store
        .submission(&owner.0, &body.submission_id)
        .await?
        .is_none()
    {
        return Err(err("NOT_FOUND", "Submission not found").into());
    }
    if let Some(job) = state
        .store
        .idempotent_job(&owner.0, &body.submission_id, key.as_deref())
        .await?
    {
        return Ok((
            StatusCode::ACCEPTED,
            Json(job_view(&state, &owner.0, job).await?),
        ));
    }
    if !state.ready().await? {
        return Err(err("WORKER_UNAVAILABLE", "Execution worker is not ready").into());
    }
    let job = state
        .store
        .enqueue(&owner.0, &body.submission_id, key.as_deref(), now())
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(job_view(&state, &owner.0, job).await?),
    ))
}
async fn job(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> HttpResult<Json<Value>> {
    Ok(Json(
        job_view(&state, &owner.0, owned_job(&state, &owner.0, &id).await?).await?,
    ))
}
async fn cancel(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> HttpResult<(StatusCode, Json<Value>)> {
    check_id(&id, "job_")?;
    let job = state.store.cancel(&owner.0, &id, now()).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(job_view(&state, &owner.0, job).await?),
    ))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<String>,
}
async fn jobs(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Query(cursor): Query<Cursor>,
) -> HttpResult<Json<Value>> {
    if cursor.after.as_ref().is_some_and(|id| !opaque(id, "job_")) {
        return Err(err("INVALID_CURSOR", "Invalid job cursor").into());
    }
    let (rows, next) = state.store.jobs(&owner.0, cursor.after.as_deref()).await?;
    let mut views = vec![];
    for row in rows {
        views.push(job_view(&state, &owner.0, row).await?);
    }
    Ok(Json(
        json!({"schema_version":1,"jobs":views,"next_cursor":next}),
    ))
}
async fn events(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> HttpResult<Json<Value>> {
    check_id(&id, "job_")?;
    let after = cursor
        .after
        .map(|c| c.parse::<i64>())
        .transpose()
        .map_err(|_| err("INVALID_CURSOR", "Event cursor must be an integer"))?
        .unwrap_or(0);
    let events = state.store.events(&owner.0, &id, after).await?;
    let next = events.last().map(|e| e.cursor).unwrap_or(after);
    Ok(Json(
        json!({"schema_version":1,"events":events,"next_cursor":next}),
    ))
}
async fn artifact_bytes(
    state: &ApiState,
    owner: &str,
    id: &str,
    kind: &str,
) -> Result<(Vec<u8>, Artifact)> {
    let job = owned_job(state, owner, id).await?;
    if job.state != JobState::Completed {
        return Err(err(
            "RESULT_NOT_READY",
            "Job has no published completed result",
        ));
    }
    let artifact = state
        .store
        .artifacts(owner, id)
        .await?
        .into_iter()
        .find(|a| a.kind == kind)
        .ok_or_else(|| err("NOT_FOUND", "Artifact not available"))?;
    let permit = state
        .readers
        .clone()
        .try_acquire_owned()
        .map_err(|_| err("DOWNLOAD_BUSY", "Too many concurrent downloads"))?;
    let root = state.config.storage_root.clone();
    let copy = artifact.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let _permit = permit;
        relative_path(&copy.storage_key)?;
        let mut path = root;
        for part in FsPath::new(&copy.storage_key).components() {
            path.push(part);
            let info = fs::symlink_metadata(&path)?;
            if !info.is_dir() && !info.is_file() {
                return Err(err(
                    "ARTIFACT_INTEGRITY_ERROR",
                    "Published file is not regular",
                ));
            }
        }
        let mut file = File::open(&path)?;
        let size = file.metadata()?.len();
        if size != copy.bytes as u64 || size > 128 * 1024 * 1024 {
            return Err(err(
                "ARTIFACT_INTEGRITY_ERROR",
                "Published evidence size changed",
            ));
        }
        let mut bytes = Vec::new();
        file.by_ref().take(size + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(&bytes)) != copy.sha256 {
            return Err(err(
                "ARTIFACT_INTEGRITY_ERROR",
                "Published evidence hash changed",
            ));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| err("STORAGE_ERROR", "Artifact reader failed"))??;
    Ok((bytes, artifact))
}
async fn report(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> HttpResult<Json<Value>> {
    let (bytes, _) = artifact_bytes(&state, &owner.0, &id, "report_json").await?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| err("ARTIFACT_INTEGRITY_ERROR", "Stored report is invalid"))?;
    if value["schema_version"] != 1 || value["derived_view"] != true {
        return Err(err(
            "ARTIFACT_INTEGRITY_ERROR",
            "Stored report is not a client view",
        )
        .into());
    }
    Ok(Json(value))
}
async fn download(
    State(state): State<Arc<ApiState>>,
    Extension(owner): Extension<Owner>,
    Path((id, kind)): Path<(String, String)>,
) -> HttpResult<Response> {
    let (file, mime) = match kind.as_str() {
        "report_json" => ("report.json", "application/json"),
        "report_html" => ("report.html", "text/html; charset=utf-8"),
        "summary" => ("summary.md", "text/markdown; charset=utf-8"),
        "portable_case" => ("portable_case.zip", "application/zip"),
        _ => return Err(err("NOT_FOUND", "Artifact kind not available").into()),
    };
    let (bytes, artifact) = artifact_bytes(&state, &owner.0, &id, &kind).await?;
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{file}\"")).expect("fixed filename"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'; style-src 'unsafe-inline'"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", artifact.sha256)).expect("artifact digest"),
    );
    Ok(response)
}
