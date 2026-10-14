use dg_config::ProjectManifest;
use std::path::PathBuf;

#[test]
fn loaded_migration_public_key_argument_matches_runtime_binding_resolution() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = ProjectManifest::load(&root.join("examples/escrow-runtime")).unwrap();
    let arg = project
        .migration
        .arguments
        .iter()
        .find(|a| a.name == "arbitrator")
        .unwrap();
    let expected: solana_address::Address = "3qbR1eZRqXUWroWKKYhbDmR3FfqTHfqSU8zZSxtANzYh"
        .parse()
        .unwrap();
    assert_eq!(arg.encoded, expected.as_ref());
}
