use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use dg_service::{
    api::{router, ApiConfig, ApiState, Credential},
    model::PreparedUpload,
    protocol::*,
    runner::{now, Worker, WorkerConfig},
    sandbox,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Cursor,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::Arc,
};
use tempfile::TempDir;
use tower::ServiceExt;

const ALICE: &str = "alice-test-token-012345678901234567890123456789";
const BOB: &str = "bob-test-token-01234567890123456789012345678901";
struct App {
    _temp: TempDir,
    state: Arc<ApiState>,
    app: Router,
}
impl App {
    async fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let config = ApiConfig {
            storage_root: temp.path().join("storage"),
            bind: "127.0.0.1:0".parse().unwrap(),
            tokens: vec![
                Credential {
                    owner: "alice".into(),
                    sha256: format!("{:x}", Sha256::digest(ALICE)),
                },
                Credential {
                    owner: "bob".into(),
                    sha256: format!("{:x}", Sha256::digest(BOB)),
                },
            ],
            cors_origins: vec!["http://localhost:5173".into()],
            request_timeout_seconds: 1,
        };
        let state = ApiState::open(config).await.unwrap();
        let app = router(state.clone());
        Self {
            _temp: temp,
            state,
            app,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        content: Option<&str>,
        body: impl Into<Body>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        if let Some(content) = content {
            builder = builder.header("content-type", content);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body.into()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 129 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec();
        (status, headers, bytes)
    }
    async fn json(
        &self,
        method: &str,
        path: &str,
        token: &str,
        value: Value,
    ) -> (StatusCode, Value) {
        let (status, _, body) = self
            .call(
                method,
                path,
                Some(token),
                Some("application/json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .await;
        (status, serde_json::from_slice(&body).unwrap())
    }
    async fn upload(&self, token: &str, data: &[u8]) -> Value {
        let body = multipart(&[
            ("metadata", serde_json::to_vec(&metadata()).unwrap()),
            ("archive", data.to_vec()),
        ]);
        let (status, _, body) = self
            .call(
                "POST",
                "/v1/submissions",
                Some(token),
                Some("multipart/form-data; boundary=boundary"),
                body,
            )
            .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&body)
        );
        let value: Value = serde_json::from_slice(&body).unwrap();
        contract(&value, "Submission");
        value
    }
    async fn worker(&self) -> (UnixListener, Worker) {
        let root = self._temp.path();
        let socket = UnixListener::bind(root.join("docker.sock")).unwrap();
        fs::write(root.join("docker.py"), include_str!("fixtures/docker.py")).unwrap();
        fs::set_permissions(root.join("docker.py"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("identity.json"), sandbox::ENGINE_LOCK).unwrap();
        fs::write(root.join("stream.bin"), stream()).unwrap();
        fs::write(root.join("mode"), "normal").unwrap();
        let cfg = WorkerConfig {
            storage_root: root.join("storage"),
            docker: root.join("docker.py"),
            socket: root.join("docker.sock"),
            image: format!("sha256:{}", "b".repeat(64)),
            timeout_seconds: 5,
        };
        let worker = Worker::open(cfg).await.unwrap();
        worker.check().await.unwrap();
        (socket, worker)
    }
}
fn metadata() -> Value {
    json!({"schema_version":1,"mode":"prepared","manifest_path":"project/manifest.toml","policy_path":"project/policy.toml","snapshot_path":null})
}
fn multipart(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut body = vec![];
    for (name, bytes) in parts {
        body.extend_from_slice(
            format!("--boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--boundary--\r\n");
    body
}
fn stream() -> Vec<u8> {
    let report = json!({"schema_version":1,"engine_version":"0.1.0","workflow_version":1,"manifest_schema_version":1,"legacy_unversioned_manifest":false,"run_id":format!("RUN-{}","a".repeat(64)),"result":"PASS","exit_code":0,"remote_transactions_sent":false,"rpc_write_calls":0,"migration_executed_locally":true,"preservation_evaluated":true,"lifecycle_evaluated":true,"policy_evaluated":true,"analysis":{},"execution":{},"preservation":{},"relationships":{},"lifecycle":{},"artifacts":{"manifest":{"path":"/work/input/project/manifest.toml"}},"policy":{"schema_version":1,"policy_evaluated":true,"passed":true,"blocking":0,"evaluation_errors":0}});
    let mut bytes = MAGIC.to_vec();
    frame(&mut bytes, RUNNING, &[]).unwrap();
    frame(&mut bytes, PUBLISHING, &[]).unwrap();
    frame(&mut bytes, ENGINE_EXIT, &[0]).unwrap();
    frame(&mut bytes, ORIGINAL, &serde_json::to_vec(&report).unwrap()).unwrap();
    frame(&mut bytes, HTML, b"<html>fixture</html>").unwrap();
    frame(&mut bytes, SUMMARY, b"fixture").unwrap();
    frame(&mut bytes, DONE, &[]).unwrap();
    bytes
}
fn contract(value: &Value, name: &str) {
    let spec: Value = serde_json::from_str(include_str!("../../../docs/backend-api.yaml")).unwrap();
    fn validate(value: &Value, schema: &Value, spec: &Value) {
        if let Some(reference) = schema["$ref"].as_str() {
            return validate(
                value,
                spec.pointer(reference.strip_prefix('#').unwrap()).unwrap(),
                spec,
            );
        }
        if let Some(constant) = schema.get("const") {
            assert_eq!(value, constant);
        }
        if let Some(choices) = schema["enum"].as_array() {
            assert!(choices.contains(value), "{value} not in {choices:?}");
        }
        if let Some(kind) = schema.get("type") {
            let actual = if value.is_object() {
                "object"
            } else if value.is_array() {
                "array"
            } else if value.is_string() {
                "string"
            } else if value.is_null() {
                "null"
            } else if value.is_boolean() {
                "boolean"
            } else {
                "integer"
            };
            assert!(
                kind == actual
                    || kind
                        .as_array()
                        .is_some_and(|a| a.iter().any(|k| k == actual)),
                "{value}: expected {kind}, got {actual}"
            );
        }
        if let Some(object) = value.as_object() {
            if let Some(required) = schema["required"].as_array() {
                for key in required {
                    assert!(object.contains_key(key.as_str().unwrap()), "missing {key}");
                }
            }
            for (key, value) in object {
                if let Some(property) = schema["properties"].get(key) {
                    validate(value, property, spec);
                } else {
                    assert_ne!(schema["additionalProperties"], false, "unexpected {key}");
                }
            }
        }
        if let Some(array) = value.as_array() {
            if let Some(limit) = schema["maxItems"].as_u64() {
                assert!(array.len() <= limit as usize);
            }
            if let Some(item) = schema.get("items") {
                for value in array {
                    validate(value, item, spec);
                }
            }
        }
    }
    validate(value, &spec["components"]["schemas"][name], &spec)
}

#[tokio::test]
async fn authentication_cors_request_ids_and_readiness_are_honest() {
    let app = App::new().await;
    let (status, _, body) = app
        .call("GET", "/health/ready", None, None, Body::empty())
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let health: Value = serde_json::from_slice(&body).unwrap();
    contract(&health, "Health");
    assert_eq!(health["status"], "unavailable");
    for token in [
        None,
        Some("bad"),
        Some("invalid-secret-012345678901234567890123456789"),
    ] {
        let (status, headers, body) = app
            .call("GET", "/v1/capabilities", token, None, Body::empty())
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let error: Value = serde_json::from_slice(&body).unwrap();
        contract(&error, "Error");
        assert_eq!(
            headers["x-request-id"].to_str().unwrap(),
            error["error"]["request_id"]
        );
        assert!(!String::from_utf8(body).unwrap().contains("invalid-secret"));
    }
    let (status, _, body) = app
        .call("GET", "/v1/capabilities", Some(ALICE), None, Body::empty())
        .await;
    assert_eq!(status, StatusCode::OK);
    contract(&serde_json::from_slice(&body).unwrap(), "Capabilities");
    for (origin, expected) in [
        ("http://localhost:5173", StatusCode::OK),
        ("https://bad.example", StatusCode::FORBIDDEN),
    ] {
        let response = app
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/capabilities")
                    .header("authorization", format!("Bearer {ALICE}"))
                    .header("origin", origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            assert_eq!(response.headers()["access-control-allow-origin"], origin);
        }
    }
    let request = Request::builder()
        .method("OPTIONS")
        .uri("/v1/jobs")
        .header("origin", "http://localhost:5173")
        .header("access-control-request-method", "POST")
        .header(
            "access-control-request-headers",
            "authorization,content-type,idempotency-key",
        )
        .body(Body::empty())
        .unwrap();
    let response = app.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );
    let (_socket, worker) = app.worker().await;
    assert!(app.state.ready().await.unwrap());
    worker
        .store
        .record_worker_ready(true, now() - 16_000)
        .await
        .unwrap();
    assert!(!app.state.ready().await.unwrap());
    worker.check().await.unwrap();
    drop(worker);
    assert!(!app.state.ready().await.unwrap());
}

#[tokio::test]
async fn upload_streaming_limits_invalid_fields_and_partial_cleanup() {
    let app = App::new().await;
    let value = app.upload(ALICE, &vec![7; 3 * 1024 * 1024]).await;
    assert_eq!(value["bytes"], 3 * 1024 * 1024);
    let cases = [
        (
            vec![
                (
                    "metadata",
                    serde_json::to_vec(&json!({"schema_version":1,"mode":"source"})).unwrap(),
                ),
                ("archive", b"data".to_vec()),
            ],
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            vec![("metadata", b"{}".to_vec()), ("archive", b"data".to_vec())],
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            vec![
                ("metadata", serde_json::to_vec(&metadata()).unwrap()),
                ("metadata", b"{}".to_vec()),
                ("archive", b"data".to_vec()),
            ],
            StatusCode::BAD_REQUEST,
        ),
        (
            vec![
                ("metadata", vec![b'x'; 32 * 1024 + 1]),
                ("archive", b"data".to_vec()),
            ],
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (
            vec![
                ("metadata", serde_json::to_vec(&metadata()).unwrap()),
                ("archive", vec![]),
            ],
            StatusCode::BAD_REQUEST,
        ),
        (
            vec![
                ("metadata", serde_json::to_vec(&metadata()).unwrap()),
                ("unexpected", b"data".to_vec()),
            ],
            StatusCode::BAD_REQUEST,
        ),
    ];
    for (parts, expected) in cases {
        let (status, _, body) = app
            .call(
                "POST",
                "/v1/submissions",
                Some(ALICE),
                Some("multipart/form-data; boundary=boundary"),
                multipart(&parts),
            )
            .await;
        assert_eq!(status, expected, "{}", String::from_utf8_lossy(&body));
        contract(&serde_json::from_slice(&body).unwrap(), "Error");
        assert_eq!(
            fs::read_dir(app._temp.path().join("storage/submissions"))
                .unwrap()
                .count(),
            1
        );
    }
    let partial =
        b"--boundary\r\nContent-Disposition: form-data; name=\"archive\"\r\n\r\npartial upload";
    let (status, _, _) = app
        .call(
            "POST",
            "/v1/submissions",
            Some(ALICE),
            Some("multipart/form-data; boundary=boundary"),
            partial.to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        fs::read_dir(app._temp.path().join("storage/submissions"))
            .unwrap()
            .count(),
        1
    );
    // Drop an in-flight receive on the server deadline, then release its reservation.
    let pending = Body::from_stream(futures_util::stream::pending::<
        std::result::Result<axum::body::Bytes, std::io::Error>,
    >());
    let (status, _, _) = app
        .call(
            "POST",
            "/v1/submissions",
            Some(ALICE),
            Some("multipart/form-data; boundary=boundary"),
            pending,
        )
        .await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    app.upload(ALICE, b"after timeout").await;
}

#[tokio::test]
async fn queue_idempotency_cancellation_ownership_and_delete_boundaries() {
    let app = App::new().await;
    let sub = app.upload(ALICE, b"opaque upload").await;
    let id = sub["submission_id"].as_str().unwrap();
    let (status, _) = app
        .json("POST", "/v1/jobs", ALICE, json!({"submission_id":id}))
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (_socket, _worker) = app.worker().await;
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/v1/jobs")
            .header("authorization", format!("Bearer {ALICE}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "retry-key")
            .body(Body::from(
                serde_json::to_vec(&json!({"submission_id":id})).unwrap(),
            ))
            .unwrap()
    };
    let response = app.app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap();
    contract(&job, "Job");
    let jid = job["job_id"].as_str().unwrap();
    let response = app.app.clone().oneshot(request()).await.unwrap();
    let retry: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap();
    assert_eq!(retry["job_id"], jid);
    _worker
        .store
        .record_worker_ready(false, now())
        .await
        .unwrap();
    assert_eq!(
        app.app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    _worker.check().await.unwrap();

    for path in [
        format!("/v1/submissions/{id}"),
        format!("/v1/jobs/{jid}"),
        format!("/v1/jobs/{jid}/events"),
        format!("/v1/jobs/{jid}/report"),
        format!("/v1/jobs/{jid}/artifacts/report_json"),
    ] {
        assert_eq!(
            app.call("GET", &path, Some(BOB), None, Body::empty())
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        app.json("POST", "/v1/jobs", BOB, json!({"submission_id":id}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.call(
            "POST",
            &format!("/v1/jobs/{jid}/cancel"),
            Some(BOB),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/v1/jobs/{jid}"),
            Some(BOB),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/v1/submissions/{id}"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/v1/jobs/{jid}"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.call(
            "GET",
            &format!("/v1/jobs/{jid}/report"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, _, body) = app
        .call(
            "POST",
            &format!("/v1/jobs/{jid}/cancel"),
            Some(ALICE),
            None,
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let cancelled: Value = serde_json::from_slice(&body).unwrap();
    contract(&cancelled, "Job");
    assert_eq!(cancelled["state"], "cancelled");
    let (_, _, body) = app
        .call("GET", "/v1/jobs", Some(BOB), None, Body::empty())
        .await;
    let list: Value = serde_json::from_slice(&body).unwrap();
    contract(&list, "Jobs");
    assert_eq!(list["jobs"], json!([]));
    let (_, _, body) = app
        .call(
            "GET",
            &format!("/v1/jobs/{jid}/events"),
            Some(ALICE),
            None,
            Body::empty(),
        )
        .await;
    let events: Value = serde_json::from_slice(&body).unwrap();
    contract(&events, "Events");
    assert_eq!(events["events"].as_array().unwrap().len(), 2);
    assert_eq!(
        app.call(
            "GET",
            &format!("/v1/jobs/{jid}/events?after=-1"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/v1/jobs/{jid}"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/v1/submissions/{id}"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        fs::read_dir(app._temp.path().join("storage/submissions"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn http_queue_to_worker_report_download_and_integrity_check() {
    let app = App::new().await;
    let (_socket, worker) = app.worker().await;
    let sub = app.upload(ALICE, b"opaque runtime-double fixture").await;
    let (status, job) = app
        .json(
            "POST",
            "/v1/jobs",
            ALICE,
            json!({"submission_id":sub["submission_id"]}),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = job["job_id"].as_str().unwrap();
    worker.run_once().await.unwrap();
    let (_, _, body) = app
        .call(
            "GET",
            &format!("/v1/jobs/{id}"),
            Some(ALICE),
            None,
            Body::empty(),
        )
        .await;
    let completed: Value = serde_json::from_slice(&body).unwrap();
    contract(&completed, "Job");
    assert_eq!(completed["verdict"], "PASS");
    assert_eq!(completed["artifacts"].as_array().unwrap().len(), 3);
    assert!(!String::from_utf8(body).unwrap().contains("storage_key"));
    let (status, _, body) = app
        .call(
            "GET",
            &format!("/v1/jobs/{id}/report"),
            Some(ALICE),
            None,
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let report: Value = serde_json::from_slice(&body).unwrap();
    contract(&report, "Report");
    assert!(!String::from_utf8(body).unwrap().contains("/work/"));
    let (status, headers, body) = app
        .call(
            "GET",
            &format!("/v1/jobs/{id}/artifacts/report_html"),
            Some(ALICE),
            None,
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment;"));
    assert!(headers["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("sandbox"));
    assert_eq!(
        headers["etag"].to_str().unwrap(),
        format!("\"{:x}\"", Sha256::digest(&body))
    );
    assert_eq!(
        app.call(
            "GET",
            &format!("/v1/jobs/{id}/artifacts/private-stderr.log"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let artifact = worker
        .store
        .artifacts("alice", id)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.kind == "report_json")
        .unwrap();
    fs::write(
        app._temp.path().join("storage").join(&artifact.storage_key),
        b"tampered",
    )
    .unwrap();
    let (status, _, body) = app
        .call(
            "GET",
            &format!("/v1/jobs/{id}/report"),
            Some(ALICE),
            None,
            Body::empty(),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
        "ARTIFACT_INTEGRITY_ERROR"
    );
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/v1/jobs/{id}"),
            Some(ALICE),
            None,
            Body::empty()
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(worker.store.pending_deletions().await.unwrap().is_empty());
    assert!(!app._temp.path().join("storage/jobs").join(id).exists());
}

#[tokio::test]
async fn quota_reservations_rate_limit_and_error_envelopes() {
    let app = App::new().await;
    let root = app._temp.path().join("storage");
    let file = fs::File::create(root.join("oversize")).unwrap();
    file.set_len(19 * 1024 * 1024 * 1024).unwrap();
    let body = multipart(&[
        ("metadata", serde_json::to_vec(&metadata()).unwrap()),
        ("archive", b"data".to_vec()),
    ]);
    let (status, _, _) = app
        .call(
            "POST",
            "/v1/submissions",
            Some(ALICE),
            Some("multipart/form-data; boundary=boundary"),
            body,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    fs::remove_file(root.join("oversize")).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("upload.lock"))
        .unwrap();
    // Parallel fork/exec can briefly retain the previous request's lock descriptor.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => panic!("Cannot acquire test upload reservation: {error}"),
        }
    }
    let (status, _, _) = app
        .call("POST", "/v1/submissions", Some(ALICE), None, Body::empty())
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    drop(lock);
    for _ in 0..119 {
        let _ = app
            .call("GET", "/v1/capabilities", Some(BOB), None, Body::empty())
            .await;
    }
    assert_eq!(
        app.call("GET", "/v1/capabilities", Some(BOB), None, Body::empty())
            .await
            .0,
        StatusCode::OK
    );
    let (status, headers, body) = app
        .call("GET", "/v1/capabilities", Some(BOB), None, Body::empty())
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers["retry-after"], "60");
    contract(&serde_json::from_slice(&body).unwrap(), "Error");
    let (status, _, body) = app
        .call("PUT", "/v1/jobs", Some(ALICE), None, Body::empty())
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    contract(&serde_json::from_slice(&body).unwrap(), "Error");
}

#[tokio::test]
async fn durable_delete_intents_and_pagination_survive_reopening() {
    let app = App::new().await;
    let sub = app.upload(ALICE, b"fixture").await;
    let id = sub["submission_id"].as_str().unwrap();
    let one = app
        .state
        .store
        .enqueue("alice", id, None, now())
        .await
        .unwrap();
    app.state
        .store
        .cancel("alice", &one.job_id, now())
        .await
        .unwrap();
    let two = app
        .state
        .store
        .enqueue("alice", id, None, now() + 1)
        .await
        .unwrap();
    let (jobs, _) = app
        .state
        .store
        .jobs("alice", Some(&one.job_id))
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, two.job_id);
    assert!(app
        .state
        .store
        .jobs("bob", Some(&one.job_id))
        .await
        .is_err());
    app.state
        .store
        .cancel("alice", &two.job_id, now())
        .await
        .unwrap();
    app.state
        .store
        .delete_job("alice", &one.job_id)
        .await
        .unwrap();
    app.state
        .store
        .delete_job("alice", &two.job_id)
        .await
        .unwrap();
    app.state
        .store
        .delete_submission("alice", id)
        .await
        .unwrap();
    let root = app._temp.path().join("storage");
    // Recovery must drain more than one bounded database page.
    for _ in 0..98 {
        let receipt = dg_service::intake::receive(
            &root.join("submissions"),
            Cursor::new(b"fixture"),
            dg_service::intake::Limits::default(),
        )
        .unwrap();
        let id = app
            .state
            .store
            .add_submission(
                "alice",
                &serde_json::from_value(metadata()).unwrap(),
                &receipt,
                now(),
            )
            .await
            .unwrap();
        app.state
            .store
            .delete_submission("alice", &id)
            .await
            .unwrap();
    }
    let temp = app._temp;
    drop(app.app);
    drop(app.state);
    let config = ApiConfig {
        storage_root: root.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        tokens: vec![Credential {
            owner: "alice".into(),
            sha256: format!("{:x}", Sha256::digest(ALICE)),
        }],
        cors_origins: vec![],
        request_timeout_seconds: 1,
    };
    let receipt_dir = root.join("submissions").join(id);
    fs::remove_dir_all(&receipt_dir).unwrap();
    std::os::unix::fs::symlink(temp.path().join("missing"), &receipt_dir).unwrap();
    assert_eq!(
        ApiState::open(config.clone()).await.err().unwrap().code,
        "INVALID_STORAGE"
    );
    fs::remove_file(receipt_dir).unwrap();
    let state = ApiState::open(config).await.unwrap();
    assert!(state.store.pending_deletions().await.unwrap().is_empty());
    assert_eq!(
        fs::read_dir(temp.path().join("storage/submissions"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn incremental_receive_is_cleaned_when_dropped() {
    let temp = TempDir::new().unwrap();
    let mut writer =
        dg_service::intake::UploadWriter::new(temp.path(), dg_service::intake::Limits::default())
            .unwrap();
    writer.write(b"partial").unwrap();
    drop(writer);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    let receipt = dg_service::intake::receive(
        temp.path(),
        Cursor::new(b"finished"),
        dg_service::intake::Limits::default(),
    )
    .unwrap();
    assert_eq!(receipt.bytes(), 8);
}

#[tokio::test]
async fn idempotency_conflicts_expiry_queue_and_upload_quotas_and_body_limits() {
    let app = App::new().await;
    let (_socket, worker) = app.worker().await;
    let one = app.upload(ALICE, b"first").await;
    let two = app.upload(ALICE, b"second").await;
    let request = |sub: &Value| {
        Request::builder()
            .method("POST")
            .uri("/v1/jobs")
            .header("authorization", format!("Bearer {ALICE}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "conflict-key")
            .body(Body::from(
                serde_json::to_vec(&json!({"submission_id":sub["submission_id"]})).unwrap(),
            ))
            .unwrap()
    };
    assert_eq!(
        app.app
            .clone()
            .oneshot(request(&one))
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        app.app
            .clone()
            .oneshot(request(&two))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    for _ in 0..3 {
        assert_eq!(
            app.json(
                "POST",
                "/v1/jobs",
                ALICE,
                json!({"submission_id":one["submission_id"]})
            )
            .await
            .0,
            StatusCode::ACCEPTED
        );
    }
    assert_eq!(
        app.json(
            "POST",
            "/v1/jobs",
            ALICE,
            json!({"submission_id":one["submission_id"]})
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let root = app._temp.path().join("storage/submissions");
    let receipt = dg_service::intake::receive(
        &root,
        Cursor::new(b"expired"),
        dg_service::intake::Limits::default(),
    )
    .unwrap();
    let prepared: PreparedUpload = serde_json::from_value(metadata()).unwrap();
    let expired = worker
        .store
        .add_submission("bob", &prepared, &receipt, now() - 604_800_001)
        .await
        .unwrap();
    assert_eq!(
        app.json("POST", "/v1/jobs", BOB, json!({"submission_id":expired}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    for _ in 0..63 {
        let receipt = dg_service::intake::receive(
            &root,
            Cursor::new(b"unused"),
            dg_service::intake::Limits::default(),
        )
        .unwrap();
        worker
            .store
            .add_submission("bob", &prepared, &receipt, now())
            .await
            .unwrap();
    }
    let body = multipart(&[
        ("metadata", serde_json::to_vec(&metadata()).unwrap()),
        ("archive", b"data".to_vec()),
    ]);
    assert_eq!(
        app.call(
            "POST",
            "/v1/submissions",
            Some(BOB),
            Some("multipart/form-data; boundary=boundary"),
            body
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/submissions")
        .header("authorization", format!("Bearer {ALICE}"))
        .header(
            "content-length",
            (384u64 * 1024 * 1024 + 64 * 1024 + 1).to_string(),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let request = Request::builder()
        .uri("/v1/capabilities")
        .header("authorization", format!("Bearer {ALICE}"))
        .header("authorization", format!("Bearer {BOB}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn configuration_requires_explicit_origins_private_bind_and_unique_credentials() {
    let mut config = ApiConfig {
        storage_root: "/private/storage".into(),
        bind: "127.0.0.1:8787".parse().unwrap(),
        tokens: vec![Credential {
            owner: "owner".into(),
            sha256: format!("{:x}", Sha256::digest(ALICE)),
        }],
        cors_origins: vec![],
        request_timeout_seconds: 120,
    };
    assert!(config.validate().is_ok());
    config.bind = "0.0.0.0:8787".parse().unwrap();
    assert!(config.validate().is_err());
    config.bind = "127.0.0.1:8787".parse().unwrap();
    config.cors_origins = vec!["*".into()];
    assert!(config.validate().is_err());
    config.cors_origins = vec![];
    config.tokens.push(config.tokens[0].clone());
    assert!(config.validate().is_err());
}

#[tokio::test]
async fn smoke_client_runs_the_workflow_over_a_real_http_socket() {
    let app = App::new().await;
    let (_socket, worker) = app.worker().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let routes = app.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, routes)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let root = app._temp.path();
    fs::write(
        root.join("metadata.json"),
        serde_json::to_vec(&metadata()).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("fixture.zip"),
        b"opaque controlled runtime fixture",
    )
    .unwrap();
    let worker = Arc::new(worker);
    let runner = worker.clone();
    let work = tokio::spawn(async move {
        for _ in 0..30 {
            if runner.run_once().await.unwrap() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("Smoke client did not submit a job");
    });
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/smoke-backend-api.py");
    let client = tokio::process::Command::new("python3")
        .arg(script)
        .arg("--url")
        .arg(format!("http://{address}"))
        .arg("--metadata")
        .arg(root.join("metadata.json"))
        .arg("--archive")
        .arg(root.join("fixture.zip"))
        .arg("--output")
        .arg(root.join("smoke"))
        .env("DG_API_TOKEN", ALICE)
        .output()
        .await
        .unwrap();
    assert!(
        client.status.success(),
        "{}",
        String::from_utf8_lossy(&client.stderr)
    );
    let summary: Value = serde_json::from_slice(&client.stdout).unwrap();
    assert_eq!(summary["verdict"], "PASS");
    assert_eq!(summary["artifacts_verified"], 3);
    work.await.unwrap();
    assert!(root.join("smoke/artifacts/report.html").is_file());
    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn download_reservations_are_held_until_bodies_are_consumed() {
    let app = App::new().await;
    let (_socket, worker) = app.worker().await;
    let sub = app.upload(ALICE, b"fixture").await;
    let (_, job) = app
        .json(
            "POST",
            "/v1/jobs",
            ALICE,
            json!({"submission_id":sub["submission_id"]}),
        )
        .await;
    worker.run_once().await.unwrap();
    let id = job["job_id"].as_str().unwrap();
    let request = || {
        Request::builder()
            .uri(format!("/v1/jobs/{id}/artifacts/report_html"))
            .header("authorization", format!("Bearer {ALICE}"))
            .body(Body::empty())
            .unwrap()
    };
    let first = app.app.clone().oneshot(request()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let second = app.app.clone().oneshot(request()).await.unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        app.app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(first);
    drop(second);
    assert_eq!(
        app.app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::OK
    );
}
