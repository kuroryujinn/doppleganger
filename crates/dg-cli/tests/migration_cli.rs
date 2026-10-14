use serde_json::json;
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const ID: &str = "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS";

#[test]
fn non_vault_project_registers_and_resolves_migration_binding() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work =
        std::env::temp_dir().join(format!("dg-migration-cli-{}-{unique}", std::process::id()));
    let project = work.join("ledger");
    fs::create_dir_all(&project).unwrap();
    let v1 = json!({"address":ID,"metadata":{"name":"ledger","version":"1"},"instructions":[]});
    let v2 = json!({"address":ID,"metadata":{"name":"ledger","version":"2"},
        "instructions":[{"name":"migrate_position","discriminator":[11,12],
            "accounts":[{"name":"position","writable":true},{"name":"operator","signer":true}],
            "args":[{"name":"version","type":"u16"}]}]});
    fs::write(project.join("v1.json"), v1.to_string()).unwrap();
    fs::write(project.join("v2.json"), v2.to_string()).unwrap();
    fs::write(project.join("v1.so"), []).unwrap();
    fs::write(project.join("v2.so"), []).unwrap();
    let manifest = format!("[project]\nname='ledger'\nframework='anchor'\n[versions.v1]\nprogram_path='v1.so'\nidl='v1.json'\nprogram_id='{ID}'\n[versions.v2]\nprogram_path='v2.so'\nidl='v2.json'\nprogram_id='{ID}'\n[migration]\ninstruction='migrate_position'\n[migration.account_mapping]\nposition='state'\noperator='payer'\n[migration.arguments]\nversion=2\n");
    fs::write(project.join("doppelganger.toml"), manifest).unwrap();
    let cli = env!("CARGO_BIN_EXE_doppelganger");
    let added = Command::new(cli)
        .current_dir(&work)
        .args(["project", "add", "ledger"])
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert!(String::from_utf8_lossy(&added.stdout).contains("Added project: ledger"));
    let inspect = Command::new(cli)
        .current_dir(&work)
        .args(["migration", "inspect", "ledger"])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(stdout.contains("Instruction: migrate_position"));
    assert!(stdout.contains("Discriminator: [11, 12]"));
    assert!(stdout.contains("Account position <- state signer=false writable=true"));
    assert!(stdout.contains("Account operator <- payer signer=true writable=false"));
    assert!(stdout.contains("Argument version"));
    let check = Command::new(cli)
        .current_dir(&work)
        .args(["migration", "check", "ledger"])
        .output()
        .unwrap();
    assert!(!check.status.success());
    assert!(String::from_utf8_lossy(&check.stderr).contains("runtime fixture is not supported"));
    fs::remove_dir_all(work).unwrap();
}
