use std::{fs, path::PathBuf, process::Command};
fn check(broken: bool) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output =
        std::env::temp_dir().join(format!("dg-r8-cli-{}-{broken}.json", std::process::id()));
    let project = if broken {
        "examples/vault-runtime-r8-broken"
    } else {
        "examples/vault-runtime"
    };
    let result = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(root)
        .args(["runtime", "lifecycle", project, "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains(if broken { "Passed: 6" } else { "Passed: 7" }),
        "{stdout}"
    );
    assert!(stdout.contains(if broken { "Failed: 1" } else { "Failed: 0" }));
    assert!(stdout.contains("Errors: 0"));
    assert!(stdout.contains("Policy: not evaluated"));
    let text = fs::read_to_string(&output).unwrap();
    let r: serde_json::Value = serde_json::from_str(&text).unwrap();
    fs::remove_file(output).unwrap();
    assert_eq!(r["lifecycle_evaluated"], true);
    assert_eq!(r["preservation_evaluated"], false);
    assert_eq!(r["policy_evaluated"], false);
    assert_eq!(r["scenarios"][2]["steps"][2]["expected"], "error");
    assert_eq!(
        r["scenarios"][2]["steps"][2]["observed"],
        if broken { "success" } else { "error" }
    );
    for secret in ["test_seed", "secret_key", "private_key", "keypair"] {
        assert!(!text.contains(secret));
    }
}
#[test]
fn good_lifecycle_report() {
    check(false);
}
#[test]
fn broken_lifecycle_is_reported_not_exit_policy() {
    check(true);
}
