use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("dg-registry-{}-{name}", std::process::id()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_doppelganger"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}
fn manifest(cwd: &Path, name: &str) -> PathBuf {
    let p = cwd.join(format!("{name}.toml"));
    fs::write(&p,format!("name='{name}'\nprogram_id='11111111111111111111111111111111'\n[v1]\nidl='v1.json'\n[v2]\nidl='v2.json'\n[[accounts]]\nv1='Before'\nv2='After'\n")).unwrap();
    p
}
#[test]
fn concurrent_processes_preserve_every_registration_and_backup() {
    let s = Scratch::new("concurrent");
    let mut children = Vec::new();
    for n in 0..24 {
        let path = manifest(&s.0, &format!("p{n}"));
        children.push(
            Command::new(env!("CARGO_BIN_EXE_doppelganger"))
                .current_dir(&s.0)
                .args(["project", "add"])
                .arg(path)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let registry: Value =
        serde_json::from_slice(&fs::read(s.0.join(".doppelganger/projects.json")).unwrap())
            .unwrap();
    assert_eq!(registry["schema_version"], 1);
    assert_eq!(registry["projects"].as_object().unwrap().len(), 24);
    let backup: Value =
        serde_json::from_slice(&fs::read(s.0.join(".doppelganger/projects.json.bak")).unwrap())
            .unwrap();
    assert_eq!(backup["projects"].as_object().unwrap().len(), 23);
    assert!(command(&s.0, &["project", "list"]).status.success());
}
#[test]
fn corruption_requires_explicit_validated_recovery_and_unknown_versions_fail() {
    let s = Scratch::new("recovery");
    for name in ["one", "two"] {
        let p = manifest(&s.0, name);
        assert!(command(&s.0, &["project", "add", p.to_str().unwrap()])
            .status
            .success());
    }
    let registry = s.0.join(".doppelganger/projects.json");
    fs::write(&registry, b"{broken").unwrap();
    assert!(!command(&s.0, &["project", "list"]).status.success());
    let p = manifest(&s.0, "three");
    assert!(!command(&s.0, &["project", "add", p.to_str().unwrap()])
        .status
        .success());
    assert_eq!(fs::read(&registry).unwrap(), b"{broken");
    assert!(command(&s.0, &["project", "recover"]).status.success());
    let value: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    assert_eq!(value["projects"].as_object().unwrap().len(), 1);
    fs::write(&registry, b"{\"schema_version\":99,\"projects\":{}}").unwrap();
    assert!(!command(&s.0, &["project", "list"]).status.success());
}
#[cfg(unix)]
#[test]
fn registry_symlinks_are_rejected_without_overwriting_target() {
    let s = Scratch::new("symlink");
    fs::create_dir(s.0.join(".doppelganger")).unwrap();
    let victim = s.0.join("victim");
    fs::write(&victim, b"original").unwrap();
    std::os::unix::fs::symlink(&victim, s.0.join(".doppelganger/projects.json")).unwrap();
    let p = manifest(&s.0, "one");
    assert!(!command(&s.0, &["project", "add", p.to_str().unwrap()])
        .status
        .success());
    assert_eq!(fs::read(victim).unwrap(), b"original");
}
#[test]
fn frozen_legacy_registry_is_upgraded_without_losing_entries() {
    let s = Scratch::new("legacy");
    let p = manifest(&s.0, "one");
    fs::create_dir(s.0.join(".doppelganger")).unwrap();
    fs::write(
        s.0.join(".doppelganger/projects.json"),
        serde_json::to_vec(&serde_json::json!({"projects":{"one":p}})).unwrap(),
    )
    .unwrap();
    let second = manifest(&s.0, "two");
    assert!(command(&s.0, &["project", "add", second.to_str().unwrap()])
        .status
        .success());
    let v: Value =
        serde_json::from_slice(&fs::read(s.0.join(".doppelganger/projects.json")).unwrap())
            .unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["projects"].as_object().unwrap().len(), 2);
}
