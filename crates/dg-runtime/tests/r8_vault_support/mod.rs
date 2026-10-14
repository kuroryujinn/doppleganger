#![allow(dead_code)]
use dg_config::{addresses::AddressDefinition, ProjectManifest};
use dg_runtime::lifecycle::{LifecycleEngine, LifecycleReport};
use std::{fs, path::PathBuf};
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn manifest(broken: bool) -> (ProjectManifest, PathBuf) {
    let base = root().join(if broken {
        "examples/vault-runtime-r8-broken"
    } else {
        "examples/vault-runtime"
    });
    let mut m =
        ProjectManifest::parse(&fs::read_to_string(base.join("doppelganger.toml")).unwrap())
            .unwrap();
    for (name, seed) in [("owner", [11; 32]), ("stranger", [12; 32])] {
        match m.addresses.get_mut(name).unwrap() {
            AddressDefinition::Signer { test_seed, .. } => *test_seed = Some(seed),
            _ => panic!("signer required"),
        }
    }
    for version in [&m.versions.v1, &m.versions.v2] {
        assert!(
            base.join(&version.program_path).is_file(),
            "missing required SBF artifact: {}",
            base.join(&version.program_path).display()
        );
        assert!(
            base.join(&version.idl).is_file(),
            "missing required IDL: {}",
            base.join(&version.idl).display()
        );
    }
    (m, base)
}
pub fn run(broken: bool) -> LifecycleReport {
    let (m, base) = manifest(broken);
    LifecycleEngine.run(&m, &base).unwrap()
}
