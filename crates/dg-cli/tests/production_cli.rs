use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("dg-production-{}-{name}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(args: &[&str], out: &Path) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .args(args)
        .args(["--json", "--output"])
        .arg(out)
        .output()
        .unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().unwrap(), json)
}
fn custom_manifest(s: &Scratch, name: &str, edit: impl FnOnce(&mut toml::Value)) -> PathBuf {
    let base = root().join(format!("examples/{name}"));
    let mut m: toml::Value =
        toml::from_str(&fs::read_to_string(base.join("doppelganger.toml")).unwrap()).unwrap();
    for version in ["v1", "v2"] {
        for field in ["program_path", "idl"] {
            let path = base
                .join(m["versions"][version][field].as_str().unwrap())
                .canonicalize()
                .unwrap();
            m["versions"][version][field] = toml::Value::String(path.to_str().unwrap().into());
        }
    }
    edit(&mut m);
    let path = s.0.join("doppelganger.toml");
    fs::write(&path, toml::to_string(&m).unwrap()).unwrap();
    path
}
fn policy(s: &Scratch, name: &str, edit: impl FnOnce(String) -> String) -> PathBuf {
    let path = s.0.join("policy.toml");
    fs::write(
        &path,
        edit(
            fs::read_to_string(root().join(format!("examples/{name}/doppelganger-policy.toml")))
                .unwrap(),
        ),
    )
    .unwrap();
    path
}
#[test]
fn both_projects_pass_complete_pipeline_and_generate_reports() {
    let s = Scratch::new("good");
    for name in ["vault-runtime", "escrow-runtime"] {
        let out = s.0.join(name);
        let (code, r) = run(&["test", &format!("examples/{name}"), "--ci"], &out);
        assert_eq!(code, 0, "{r}");
        assert_eq!(r["result"], "PASS");
        assert_eq!(r["execution"]["transaction"]["success"], true);
        assert_eq!(r["relationships"]["errors"], 0);
        assert!(r["relationships"]["passed"].as_u64().unwrap() > 0);
        assert_eq!(r["policy"]["blocking"], 0);
        assert!(r["policy"]["expected"].as_u64().unwrap() > 0);
        for file in ["report.json", "report.html", "summary.md"] {
            assert!(out.join(file).is_file());
        }
        for role in [
            "manifest",
            "policy",
            "v1_binary",
            "v2_binary",
            "v1_idl",
            "v2_idl",
        ] {
            assert_eq!(r["artifacts"][role]["sha256"].as_str().unwrap().len(), 64);
        }
        let html = fs::read_to_string(out.join("report.html")).unwrap();
        assert!(html.contains("before/after"));
    }
}
#[test]
fn preservation_violation_returns_policy_exit_one() {
    let s = Scratch::new("broken");
    let (code, r) = run(
        &[
            "migration",
            "test",
            "examples/escrow-runtime-broken",
            "--policy",
            "examples/escrow-runtime/doppelganger-policy.toml",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 1, "{r}");
    assert_eq!(r["result"], "FAIL");
    assert_eq!(r["preservation"]["failed"], 1);
    assert_eq!(r["lifecycle"]["failed"], 0);
    assert_eq!(r["policy"]["blocking"], 1);
}
#[test]
fn repeat_migration_violation_blocks() {
    let s = Scratch::new("repeat");
    let (code, r) = run(
        &[
            "test",
            "examples/vault-runtime-r8-broken",
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 1, "{r}");
    assert_eq!(r["preservation"]["failed"], 0);
    assert_eq!(r["lifecycle"]["failed"], 1);
}
#[test]
fn wrong_relationship_target_blocks_even_when_preservation_passes() {
    let s = Scratch::new("wrong-authority");
    let p = policy(&s, "vault-runtime", |text| {
        text.replace("target = \"$owner\"", "target = \"$stranger\"")
    });
    let (code, r) = run(
        &[
            "test",
            "examples/vault-runtime",
            "--policy",
            p.to_str().unwrap(),
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 1);
    assert_eq!(r["preservation"]["failed"], 0);
    assert_eq!(r["relationships"]["failed"], 1);
}
#[test]
fn missing_relationship_evidence_returns_runtime_exit_three() {
    let s = Scratch::new("missing-evidence");
    let p = policy(&s, "vault-runtime", |text| {
        text.replace("account = \"vault\"", "account = \"unobserved\"")
    });
    let (code, r) = run(
        &[
            "test",
            "examples/vault-runtime",
            "--policy",
            p.to_str().unwrap(),
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 3);
    assert_eq!(r["result"], "ERROR");
    assert!(r["relationships"]["errors"].as_u64().unwrap() > 0);
}
#[test]
fn invalid_or_unsupported_policy_is_a_configuration_error_with_saved_diagnostics() {
    let s = Scratch::new("invalid");
    let p = s.0.join("policy.toml");
    fs::write(&p, "schema_version=9").unwrap();
    let (code, r) = run(
        &[
            "test",
            "examples/vault-runtime",
            "--policy",
            p.to_str().unwrap(),
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 2);
    assert_eq!(r["diagnostics"][0]["category"], "CONFIG");
    assert!(s.0.join("run/report.json").is_file());
}
#[test]
fn missing_policy_is_not_an_implicit_safety_pass() {
    let s = Scratch::new("no-policy");
    let (code, r) = run(
        &[
            "test",
            "examples/vault-runtime",
            "--policy",
            s.0.join("missing.toml").to_str().unwrap(),
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 2);
    assert_eq!(r["result"], "ERROR");
}
#[test]
fn incomplete_required_coverage_blocks() {
    let s = Scratch::new("coverage");
    let m = custom_manifest(&s, "vault-runtime", |m| {
        m.as_table_mut().unwrap().remove("preservation");
        m.as_table_mut().unwrap().remove("scenario");
    });
    let (code, r) = run(
        &[
            "test",
            m.to_str().unwrap(),
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 1, "{r}");
    assert_eq!(r["policy"]["coverage"]["preservation"], 0);
    assert_eq!(r["policy"]["coverage"]["lifecycle"], 0);
}
#[test]
fn runtime_setup_failure_retains_execution_evidence() {
    let s = Scratch::new("setup-error");
    let m = custom_manifest(&s, "vault-runtime", |m| {
        m["fixture"]["setup"][0]["args"]["amount"] =
            toml::Value::String("u64:18446744073709551615".into());
    });
    let (code, r) = run(
        &[
            "test",
            m.to_str().unwrap(),
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 3, "{r}");
    assert_eq!(r["diagnostics"][0]["category"], "RUNTIME");
    assert!(!r["diagnostics"][0]["evidence"].is_null());
}
#[test]
fn nondeterministic_fixture_identities_are_rejected_before_execution() {
    let s = Scratch::new("random-signer");
    let m = custom_manifest(&s, "vault-runtime", |m| {
        m["addresses"]["owner"]
            .as_table_mut()
            .unwrap()
            .remove("test_seed");
    });
    let (code, r) = run(
        &[
            "test",
            m.to_str().unwrap(),
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 2);
    assert!(r["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains("deterministic"));
}
#[test]
fn repeated_runs_have_identical_report_and_identity() {
    let s = Scratch::new("deterministic");
    let (_, a) = run(&["test", "examples/vault-runtime"], &s.0.join("a"));
    let (_, b) = run(&["test", "examples/vault-runtime"], &s.0.join("b"));
    assert_eq!(a, b);
    assert!(a["run_id"].as_str().unwrap().starts_with("RUN-"));
}
#[test]
fn existing_output_is_never_overwritten_and_returns_internal_exit_four() {
    let s = Scratch::new("overwrite");
    let out = s.0.join("run");
    let (code, _) = run(&["test", "examples/vault-runtime"], &out);
    assert_eq!(code, 0);
    let original = fs::read(out.join("report.json")).unwrap();
    let (code, r) = run(&["test", "examples/vault-runtime"], &out);
    assert_eq!(code, 4);
    assert_eq!(r["result"], "ERROR");
    assert_eq!(fs::read(out.join("report.json")).unwrap(), original);
}
#[test]
fn genuine_snapshot_enters_policy_workflow_without_network() {
    let s = Scratch::new("snapshot");
    let (code, r) = run(
        &[
            "test",
            "examples/vault-runtime-devnet",
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
            "--snapshot",
            "fixtures/devnet-snapshots/vault-devnet",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 0, "{r}");
    assert_eq!(r["source"]["kind"], "snapshot");
    assert_eq!(r["snapshot"]["provenance"]["slot"], 507756125);
    assert_eq!(r["remote_transactions_sent"], false);
    assert_eq!(r["rpc_write_calls"], 0);
}
#[test]
fn html_escapes_profile_text() {
    let s = Scratch::new("html");
    let p = policy(&s, "vault-runtime", |text| {
        text.replace("Reviewed fixture upgrade:", "<script>alert(1)</script>")
    });
    let (code, _) = run(
        &[
            "test",
            "examples/vault-runtime",
            "--policy",
            p.to_str().unwrap(),
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 0);
    let html = fs::read_to_string(s.0.join("run/report.html")).unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn combined_report_preserves_native_regression_creation_and_candidate_replay() {
    let s = Scratch::new("regression");
    let out = s.0.join("run");
    let (code, _) = run(
        &[
            "test",
            "examples/escrow-runtime-broken",
            "--policy",
            "examples/escrow-runtime/doppelganger-policy.toml",
        ],
        &out,
    );
    assert_eq!(code, 1);
    let created = dg_regression::create(
        &out.join("report.json"),
        &root().join("examples/escrow-runtime-broken"),
        dg_regression::Source::Preservation("PRESERVE-amount-retained".into()),
        &s.0.join("regressions"),
    )
    .unwrap();
    let broken = dg_regression::reproduce(&created.directory, &Default::default()).unwrap();
    assert_eq!(broken.result, dg_regression::ReplayStatus::Reproduced);
    let good = dg_regression::reproduce(
        &created.directory,
        &dg_regression::CandidateOverride {
            binary: Some(root().join("target/deploy/escrow-v2/dg_escrow_v2.so")),
            idl: None,
        },
    )
    .unwrap();
    assert_eq!(good.result, dg_regression::ReplayStatus::NotReproduced);
}

#[test]
fn combined_snapshot_report_reproduces_from_frozen_state() {
    let s = Scratch::new("snapshot-regression");
    let m = custom_manifest(&s, "vault-runtime-devnet", |m| {
        m["versions"]["v2"]["program_path"] = toml::Value::String(
            root()
                .join("fixtures/devnet-snapshots/vault-devnet/artifacts/v2-broken.bin")
                .to_str()
                .unwrap()
                .into(),
        );
    });
    let out = s.0.join("run");
    let (code, r) = run(
        &[
            "test",
            m.to_str().unwrap(),
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
            "--snapshot",
            "fixtures/devnet-snapshots/vault-devnet",
        ],
        &out,
    );
    assert_eq!(code, 1, "{r}");
    let created = dg_regression::create(
        &out.join("report.json"),
        &m,
        dg_regression::Source::Preservation("PRESERVE-balance-field".into()),
        &s.0.join("regressions"),
    )
    .unwrap();
    assert_eq!(
        created.case.reconstruction,
        dg_regression::Reconstruction::SnapshotReplay
    );
    let broken = dg_regression::reproduce(&created.directory, &Default::default()).unwrap();
    assert_eq!(broken.result, dg_regression::ReplayStatus::Reproduced);
    let good = dg_regression::reproduce(
        &created.directory,
        &dg_regression::CandidateOverride {
            binary: Some(
                root().join("fixtures/devnet-snapshots/vault-devnet/artifacts/v2-good.bin"),
            ),
            idl: None,
        },
    )
    .unwrap();
    assert_eq!(good.result, dg_regression::ReplayStatus::NotReproduced);
    let portable = s.0.join("portable-snapshot");
    let capture = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .args(["regression", "capture", "--from-report"])
        .arg(out.join("report.json"))
        .arg("--store")
        .arg(&portable)
        .output()
        .unwrap();
    assert!(
        capture.status.success(),
        "{}",
        String::from_utf8_lossy(&capture.stdout)
    );
    fs::remove_file(m).unwrap();
    let replay = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .args(["regression", "replay"])
        .arg(portable)
        .arg("--output")
        .arg(s.0.join("portable-replay"))
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stdout)
    );
}

#[test]
fn missing_fixture_is_configuration_error_instead_of_runtime_error() {
    let s = Scratch::new("missing-fixture");
    let m = custom_manifest(&s, "vault-runtime", |m| {
        m.as_table_mut().unwrap().remove("fixture");
    });
    let (code, r) = run(
        &[
            "test",
            m.to_str().unwrap(),
            "--policy",
            "examples/vault-runtime/doppelganger-policy.toml",
        ],
        &s.0.join("run"),
    );
    assert_eq!(code, 2);
    assert_eq!(r["diagnostics"][0]["category"], "CONFIG");
}
#[test]
fn portable_full_workflow_cases_reproduce_preservation_relationship_and_static_failures() {
    let s = Scratch::new("portable-cases");
    for kind in ["preservation", "relationship", "static"] {
        let p = policy(&s, "vault-runtime", |text| match kind {
            "relationship" => text.replace("target = \"$owner\"", "target = \"$stranger\""),
            "static" => {
                let mut value: toml::Value = toml::from_str(&text).unwrap();
                value["expected_change"] = toml::Value::Array(vec![]);
                toml::to_string(&value).unwrap()
            }
            _ => text,
        });
        let project = if kind == "preservation" {
            "examples/vault-runtime-r7-broken"
        } else {
            "examples/vault-runtime"
        };
        let original = s.0.join(format!("original-{kind}"));
        let (code, r) = run(
            &["test", project, "--policy", p.to_str().unwrap()],
            &original,
        );
        assert_eq!(code, 1, "{r}");
        let case = s.0.join(format!("case-{kind}"));
        let capture = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
            .current_dir(root())
            .args(["regression", "capture", "--from-report"])
            .arg(original.join("report.json"))
            .arg("--store")
            .arg(&case)
            .output()
            .unwrap();
        assert!(
            capture.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&capture.stdout),
            String::from_utf8_lossy(&capture.stderr)
        );
        // Relocate the whole case and remove the source policy/report. Replay needs only the case.
        let moved = s.0.join(format!("moved-{kind}"));
        fs::rename(&case, &moved).unwrap();
        fs::remove_file(&p).unwrap();
        fs::remove_dir_all(&original).unwrap();
        let replay = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
            .current_dir(&s.0)
            .args(["regression", "replay"])
            .arg(&moved)
            .arg("--output")
            .arg(s.0.join(format!("replay-{kind}")))
            .output()
            .unwrap();
        assert!(
            replay.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&replay.stdout),
            String::from_utf8_lossy(&replay.stderr)
        );
        let report: Value = serde_json::from_slice(&replay.stdout).unwrap();
        assert_eq!(report["matched"], true);
        fs::write(moved.join("policy.toml"), "tampered").unwrap();
        let bad = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
            .args(["regression", "replay"])
            .arg(&moved)
            .arg("--output")
            .arg(s.0.join(format!("tampered-{kind}")))
            .output()
            .unwrap();
        assert_eq!(bad.status.code(), Some(2));
        assert!(!s.0.join(format!("tampered-{kind}")).exists());
    }
}
#[test]
fn doctor_version_help_and_quiet_modes_work() {
    let s = Scratch::new("diagnostics");
    for flag in ["--version", "--help"] {
        let out = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("0.1.0"));
    }
    let doctor = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .args(["doctor", "examples/vault-runtime", "--json"])
        .output()
        .unwrap();
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stdout)
    );
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["migration_executed"], false);
    let quiet = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .args(["test", "examples/vault-runtime", "--quiet", "--output"])
        .arg(s.0.join("quiet"))
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    assert!(s.0.join("quiet/report.json").exists());
    let invalid = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .args(["doctor", "missing-directory", "--json"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}
#[test]
fn warning_and_unexpected_static_changes_have_correct_ci_exits() {
    let s = Scratch::new("static-policy");
    for (action, expected) in [("warn", 0), ("fail", 1)] {
        let p = policy(&s, "vault-runtime", |text| {
            let mut value: toml::Value = toml::from_str(&text).unwrap();
            value["expected_change"] = toml::Value::Array(vec![]);
            value
                .as_table_mut()
                .unwrap()
                .entry("policy".to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            value["policy"].as_table_mut().unwrap().insert(
                "unexpected_static".into(),
                toml::Value::String(action.into()),
            );
            toml::to_string(&value).unwrap()
        });
        let (code, r) = run(
            &[
                "test",
                "examples/vault-runtime",
                "--ci",
                "--policy",
                p.to_str().unwrap(),
            ],
            &s.0.join(action),
        );
        assert_eq!(code, expected);
        if action == "warn" {
            assert!(r["policy"]["warnings"].as_u64().unwrap() > 0);
        } else {
            assert!(r["policy"]["blocking"].as_u64().unwrap() > 0);
        }
    }
}
#[test]
fn malformed_full_workflow_report_fails_without_panicking_or_leaving_case() {
    let s = Scratch::new("malformed-case");
    let p = policy(&s, "vault-runtime", |text| {
        text.replace("target = \"$owner\"", "target = \"$stranger\"")
    });
    let (code, mut report) = run(
        &[
            "test",
            "examples/vault-runtime",
            "--policy",
            p.to_str().unwrap(),
        ],
        &s.0.join("original"),
    );
    assert_eq!(code, 1);
    report["lifecycle"]["scenarios"][0]["steps"] = serde_json::json!([1]);
    let source = s.0.join("malformed.json");
    fs::write(&source, serde_json::to_vec(&report).unwrap()).unwrap();
    let target = s.0.join("case");
    let output = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .args(["regression", "capture", "--from-report"])
        .arg(source)
        .arg("--store")
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    assert!(!target.exists());
}
