//! Required deployment gate. Uses the actual rootless runtime; never substitutes a host runner.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use dg_service::{
    api::{router, ApiConfig, ApiState, Credential},
    intake::Limits,
    model::{Job, JobState, PreparedUpload, Verdict},
    runner::{Worker, WorkerConfig},
    sandbox,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
const TOKEN: &str = "live-container-test-token-01234567890123456789";
async fn call(
    api: &Router,
    method: &str,
    path: &str,
    content: Option<&str>,
    bytes: Vec<u8>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"));
    if let Some(content) = content {
        request = request.header("content-type", content);
    }
    let response = api
        .clone()
        .oneshot(request.body(Body::from(bytes)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        to_bytes(response.into_body(), 128 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
}
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, ZipWriter};

fn archive(example: &Path, broken: Option<&Path>, missing: bool) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for name in [
        "doppelganger.toml",
        "doppelganger-policy.toml",
        "v1.bin",
        "v2.bin",
        "v1.json",
        "v2.json",
    ] {
        if missing && name == "v2.bin" {
            continue;
        }
        let path = if name == "v2.bin" {
            broken
                .map(Path::to_owned)
                .unwrap_or_else(|| example.join(name))
        } else {
            example.join(name)
        };
        zip.start_file(format!("project/{name}"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&fs::read(path).unwrap()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
#[ignore = "Required live rootless Docker qualification; configure DG_WORKER_CONFIG, DG_PREPARED_EXAMPLES, DG_BROKEN_ESCROW and DG_QUALIFIED_ENGINE"]
async fn isolated_prepared_results_and_downloaded_failure_case_replay() {
    let cfg = WorkerConfig::load(Path::new(
        &std::env::var("DG_WORKER_CONFIG").expect("DG_WORKER_CONFIG"),
    ))
    .unwrap();
    // Dedicated fresh private test storage; preserves deployment config/runtime identity.
    let temporary = TempDir::new_in(cfg.storage_root.parent().unwrap()).unwrap();
    let cfg = WorkerConfig {
        storage_root: temporary.path().join("acceptance"),
        ..cfg
    };
    let worker = Worker::open(cfg.clone()).await.unwrap();
    worker.check().await.unwrap();
    worker.recover().await.unwrap();
    let api = router(
        ApiState::open(ApiConfig {
            storage_root: cfg.storage_root.clone(),
            bind: "127.0.0.1:0".parse().unwrap(),
            tokens: vec![Credential {
                owner: "qualification".into(),
                sha256: format!("{:x}", Sha256::digest(TOKEN)),
            }],
            cors_origins: vec![],
            request_timeout_seconds: 120,
        })
        .await
        .unwrap(),
    );

    let examples =
        PathBuf::from(std::env::var("DG_PREPARED_EXAMPLES").expect("DG_PREPARED_EXAMPLES"));
    let broken = PathBuf::from(std::env::var("DG_BROKEN_ESCROW").expect("DG_BROKEN_ESCROW"));
    let engine = PathBuf::from(std::env::var("DG_QUALIFIED_ENGINE").expect("DG_QUALIFIED_ENGINE"));
    sandbox::verify_engine(&engine).unwrap();
    for (name, is_broken, missing, verdict) in [
        ("vault", false, false, Verdict::Pass),
        ("escrow", false, false, Verdict::Pass),
        ("record", false, false, Verdict::Pass),
        ("escrow", true, false, Verdict::Fail),
        ("vault", false, true, Verdict::Error),
    ] {
        let bytes = archive(
            &examples.join(name),
            is_broken.then_some(broken.as_path()),
            missing,
        );
        let metadata = PreparedUpload {
            schema_version: 1,
            mode: "prepared".into(),
            manifest_path: "project/doppelganger.toml".into(),
            policy_path: "project/doppelganger-policy.toml".into(),
            snapshot_path: None,
        };
        let mut multipart =
            b"--fixture\r\nContent-Disposition: form-data; name=\"metadata\"\r\n\r\n".to_vec();
        multipart.extend_from_slice(&serde_json::to_vec(&metadata).unwrap());
        multipart.extend_from_slice(
            b"\r\n--fixture\r\nContent-Disposition: form-data; name=\"archive\"\r\n\r\n",
        );
        multipart.extend_from_slice(&bytes);
        multipart.extend_from_slice(b"\r\n--fixture--\r\n");
        let (status, body) = call(
            &api,
            "POST",
            "/v1/submissions",
            Some("multipart/form-data; boundary=fixture"),
            multipart,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let sub: Value = serde_json::from_slice(&body).unwrap();
        // This gate drives individual attempts rather than the continuous worker loop.
        worker.check().await.unwrap();
        let (status, body) = call(
            &api,
            "POST",
            "/v1/jobs",
            Some("application/json"),
            serde_json::to_vec(&json!({"submission_id":sub["submission_id"]})).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let job: Job = serde_json::from_slice(&body).unwrap();
        worker.run_once().await.unwrap();
        let (status, body) = call(
            &api,
            "GET",
            &format!("/v1/jobs/{}", job.job_id),
            None,
            vec![],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let result: Job = serde_json::from_slice(&body).unwrap();
        let (status, report) = call(
            &api,
            "GET",
            &format!("/v1/jobs/{}/report", job.job_id),
            None,
            vec![],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!String::from_utf8(report).unwrap().contains("/work/input"));
        assert_eq!(
            result.state,
            JobState::Completed,
            "{name}: {:?}",
            result.error_code
        );
        assert_eq!(result.verdict, Some(verdict));
        if missing {
            assert_eq!(result.engine_exit_code, None);
        }
        if is_broken {
            let (status, download) = call(
                &api,
                "GET",
                &format!("/v1/jobs/{}/artifacts/portable_case", job.job_id),
                None,
                vec![],
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "eligible FAIL must capture a downloadable case"
            );
            // Only this committed developer fixture is extracted/replayed on the test host.
            let case_root = temporary.path().join("downloaded-case");
            fs::create_dir(&case_root).unwrap();
            dg_service::intake::extract(
                Cursor::new(download),
                &case_root.join("bundle"),
                Limits::default(),
            )
            .unwrap();
            let replay = Command::new(&engine)
                .args(["regression", "replay"])
                .arg(case_root.join("bundle"))
                .arg("--output")
                .arg(temporary.path().join("replayed"))
                .output()
                .unwrap();
            assert!(
                replay.status.success(),
                "{}",
                String::from_utf8_lossy(&replay.stdout)
            );
        }
    }
}
