use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn run(action: &str, broken: bool) -> Value {
    let project = if broken {
        "examples/escrow-runtime-broken"
    } else {
        "examples/escrow-runtime"
    };
    let output = std::env::temp_dir().join(format!(
        "dg-r9-{}-{action}-{broken}.json",
        std::process::id()
    ));
    let result = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .args(["runtime", action, project, "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    let text = fs::read_to_string(&output).unwrap();
    let report = serde_json::from_str(&text).unwrap();
    fs::remove_file(output).unwrap();
    for secret in ["test_seed", "secret_key", "private_key", "keypair"] {
        assert!(!text.contains(secret));
    }
    report
}

#[test]
fn static_analysis_accepts_escrow_account_mapping() {
    let output = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root())
        .args([
            "analyze",
            "--from",
            "fixtures/escrow/idl/v1.json",
            "--to",
            "fixtures/escrow/idl/v2.json",
            "--account",
            "EscrowV1:EscrowV2",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["runtime_executed"], false);
    assert_eq!(
        report["account_sizes"]["EscrowV1 -> EscrowV2"][0]["bytes"],
        81
    );
    assert_eq!(
        report["account_sizes"]["EscrowV1 -> EscrowV2"][1]["bytes"],
        121
    );
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"] == "EscrowStatus.Disputed"));
}

#[test]
fn build_reports_real_v1_state_without_migration() {
    let r = run("build", false);
    assert_eq!(r["migration_executed"], false);
    assert_eq!(r["executed_actions"].as_array().unwrap().len(), 2);
    let state = &r["observed_accounts"][0];
    assert_eq!(state["snapshot"]["data_length"], 81);
    assert_eq!(state["decoded"]["fields"]["amount"]["value"], "1000000");
    assert_eq!(
        state["decoded"]["fields"]["status"]["value"]["variant"],
        "Funded"
    );
}

#[test]
fn both_candidates_migrate_without_implicit_preservation_verdict() {
    for broken in [false, true] {
        let r = run("migrate", broken);
        assert_eq!(r["transaction"]["success"], true);
        assert_eq!(r["preservation_evaluated"], false);
        assert_eq!(r["after_accounts"][0]["snapshot"]["data_length"], 121);
        assert_eq!(
            r["after_accounts"][0]["decoded"]["fields"]["amount"]["value"],
            if broken { "900000" } else { "1000000" }
        );
    }
}

#[test]
fn good_preservation_has_no_findings() {
    let r = run("verify", false);
    assert_eq!(r["execution"]["transaction"]["success"], true);
    assert_eq!(r["preservation"]["passed"], 11);
    assert_eq!(r["preservation"]["failed"], 0);
    assert_eq!(r["preservation"]["errors"], 0);
    assert!(r["preservation"]["findings"].as_array().unwrap().is_empty());
    assert_eq!(r["preservation_evaluated"], true);
    assert_eq!(r["lifecycle_evaluated"], false);
    assert_eq!(r["policy_evaluated"], false);
}

#[test]
fn broken_preservation_finding_is_not_a_command_failure() {
    let r = run("verify", true);
    assert_eq!(r["execution"]["transaction"]["success"], true);
    assert_eq!(r["preservation"]["passed"], 10);
    assert_eq!(r["preservation"]["failed"], 1);
    assert_eq!(r["preservation"]["errors"], 0);
    let findings = r["preservation"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    let f = &findings[0];
    assert_eq!(f["id"], "PRESERVE-amount-retained");
    assert_eq!(f["category"], "FIELD_NOT_PRESERVED");
    assert_eq!(f["account"], "escrow");
    assert_eq!(f["field"], "amount");
    assert_eq!(f["before"]["value"]["value"], "1000000");
    assert_eq!(f["after"]["value"]["value"], "900000");
    assert_eq!(r["policy_evaluated"], false);
}

#[test]
fn both_candidates_pass_the_same_six_lifecycle_scenarios() {
    let good = run("lifecycle", false);
    let broken = run("lifecycle", true);
    for r in [&good, &broken] {
        assert_eq!(r["total"], 6);
        assert_eq!(r["passed"], 6);
        assert_eq!(r["failed"], 0);
        assert_eq!(r["errors"], 0);
        assert_eq!(r["lifecycle_evaluated"], true);
        assert_eq!(r["preservation_evaluated"], false);
        assert_eq!(r["policy_evaluated"], false);
    }
    for (a, b) in good["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .zip(broken["scenarios"].as_array().unwrap())
    {
        assert_eq!(a["definition"], b["definition"]);
        assert_eq!(a["status"], "PASS");
        assert_eq!(b["status"], "PASS");
    }
}
