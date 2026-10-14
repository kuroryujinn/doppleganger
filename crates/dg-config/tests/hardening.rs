use dg_config::ProjectManifest;
fn manifest() -> String {
    include_str!("../../../examples/escrow-runtime/doppelganger.toml").into()
}
#[test]
fn versioned_and_frozen_legacy_manifests_are_distinguished() {
    assert_eq!(
        ProjectManifest::parse(&manifest()).unwrap().schema_version,
        Some(1)
    );
    assert_eq!(
        ProjectManifest::parse(&manifest().replacen("schema_version = 1\n", "", 1))
            .unwrap()
            .schema_version,
        None
    );
    for version in [0, 2, 999] {
        assert_eq!(
            ProjectManifest::parse(&manifest().replacen(
                "schema_version = 1",
                &format!("schema_version = {version}"),
                1
            ))
            .unwrap_err()
            .field,
            "schema_version"
        );
    }
}
#[test]
fn duplicate_targets_unknown_fields_and_oversized_config_fail() {
    let mut source: toml::Value = toml::from_str(&manifest()).unwrap();
    let target = source["migration_targets"].as_array().unwrap()[0].clone();
    source["migration_targets"]
        .as_array_mut()
        .unwrap()
        .push(target);
    assert_eq!(
        ProjectManifest::parse(&toml::to_string(&source).unwrap())
            .unwrap_err()
            .field,
        "migration_targets"
    );
    assert!(
        ProjectManifest::parse(&("dangerous_option = true\n".to_owned() + &manifest())).is_err()
    );
    assert!(ProjectManifest::parse(&" ".repeat(4 * 1024 * 1024 + 1))
        .unwrap_err()
        .message
        .contains("limit"));
}
#[test]
fn seeded_toml_mutations_do_not_panic() {
    let original = manifest();
    let mut seed = 123u64;
    for n in 0..1_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = original.as_bytes().to_vec();
        let index = seed as usize % bytes.len();
        if n % 2 == 0 {
            bytes.truncate(index);
        } else {
            bytes[index] = b'!';
        }
        if let Ok(text) = String::from_utf8(bytes) {
            assert!(std::panic::catch_unwind(|| ProjectManifest::parse(&text)).is_ok());
        }
    }
}
