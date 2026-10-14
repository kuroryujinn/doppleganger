use serde_json::{json, Value};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn direct_account_mapping_accepts_non_vault_program() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work = std::env::temp_dir().join(format!("dg-preflight-{}-{unique}", std::process::id()));
    fs::create_dir(&work).unwrap();
    let idl = |name: &str, extra: bool| {
        json!({
            "address":"Program111",
            "metadata":{"name":"ledger","version":"1.0"},
            "instructions":[],
            "accounts":[{"name":name,"discriminator":[1]}],
            "types":[{"name":name,"type":{"kind":"struct","fields":
                if extra { vec![json!({"name":"balance","type":"u64"}),json!({"name":"sequence","type":"u64"})] }
                else { vec![json!({"name":"balance","type":"u64"})] }
            }}]
        })
    };
    fs::write(work.join("v1.json"), idl("LedgerV1", false).to_string()).unwrap();
    fs::write(work.join("v2.json"), idl("LedgerV2", true).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(&work)
        .args([
            "analyze",
            "--from",
            "v1.json",
            "--to",
            "v2.json",
            "--account",
            "LedgerV1:LedgerV2",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let analysis_dir = work.join(".doppelganger/analysis");
    let run = fs::read_dir(analysis_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let report: Value =
        serde_json::from_slice(&fs::read(run.join("analysis.json")).unwrap()).unwrap();
    assert_eq!(
        report["account_sizes"]["LedgerV1 -> LedgerV2"][0]["bytes"],
        9
    );
    assert_eq!(
        report["account_sizes"]["LedgerV1 -> LedgerV2"][1]["bytes"],
        17
    );
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"] == "account.LedgerV1 -> LedgerV2.sequence"));
    fs::remove_dir_all(work).unwrap();
}
