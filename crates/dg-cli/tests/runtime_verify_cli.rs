use std::{fs, path::PathBuf, process::Command};

#[test]
fn broken_candidate_is_execution_success_and_preservation_failure_not_command_failure() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::temp_dir().join(format!("dg-r7-cli-broken-{}.json", std::process::id()));
    let result = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(&root)
        .args([
            "runtime",
            "verify",
            "examples/vault-runtime-r7-broken",
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Migration: success"));
    assert!(stdout.contains("Passed: 6"));
    assert!(stdout.contains("Failed: 1"));
    assert!(stdout.contains("Errors: 0"));
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    fs::remove_file(output).unwrap();
    assert_eq!(report["execution"]["transaction"]["success"], true);
    assert_eq!(report["preservation_evaluated"], true);
    assert_eq!(report["preservation"]["failed"], 1);
    assert_eq!(report["preservation"]["errors"], 0);
    assert_eq!(
        report["preservation"]["findings"].as_array().unwrap().len(),
        1
    );
    let finding = &report["preservation"]["findings"][0];
    assert_eq!(finding["rule_id"], "balance-field");
    assert_eq!(finding["account"], "vault");
    assert_eq!(finding["field"], "balance");
    assert_eq!(finding["rule"], "equal");
    assert_eq!(finding["status"], "FAIL");
    assert_eq!(finding["category"], "FIELD_NOT_PRESERVED");
    assert_eq!(finding["before"]["value"]["value"], "1000000");
    assert_eq!(finding["after"]["value"]["value"], "950000");
    assert_eq!(report["lifecycle_evaluated"], false);
    assert_eq!(report["policy_evaluated"], false);
}

#[test]
fn verify_reports_rules_and_keeps_execution_and_policy_separate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::temp_dir().join(format!("dg-r7-cli-{}.json", std::process::id()));
    let result = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(&root)
        .args(["runtime", "verify", "examples/vault-runtime", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("Passed: 7"));
    assert!(stdout.contains("Lifecycle testing: not executed"));
    assert!(stdout.contains("Policy evaluation: not executed"));
    let text = fs::read_to_string(&output).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    fs::remove_file(output).unwrap();
    assert_eq!(report["preservation_evaluated"], true);
    assert_eq!(report["lifecycle_evaluated"], false);
    assert_eq!(report["policy_evaluated"], false);
    assert_eq!(report["execution"]["preservation_evaluated"], false);
    assert_eq!(report["preservation"]["passed"], 7);
    assert_eq!(report["preservation"]["rules"].as_array().unwrap().len(), 7);
    assert!(report["execution"]["before_accounts"][0]["snapshot"]["raw_data"].is_array());
    for secret in ["test_seed", "secret_key", "private_key", "keypair"] {
        assert!(!text.contains(secret));
    }
}
