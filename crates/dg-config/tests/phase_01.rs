use dg_analysis::normalize;
use dg_config::{MigrationDefinition, ProjectManifest};
use serde_json::json;
use solana_address::Address;
use std::{
    collections::BTreeMap,
    fs,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

const ID: &str = "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS";

fn idl() -> dg_analysis::model::ProgramModel {
    let raw = json!({
        "address": ID,
        "metadata":{"name":"test","version":"2"},
        "instructions":[{"name":"migrate","discriminator":[7,8],
            "accounts":[{"name":"state","writable":true},{"name":"authority","signer":true}],
            "args":[{"name":"version","type":"u16"},{"name":"label","type":"string"}]}]
    });
    normalize(raw.to_string().as_bytes()).unwrap()
}

fn definition() -> MigrationDefinition {
    toml::from_str(
        r#"
        instruction = "migrate"
        [account_mapping]
        state = "position"
        authority = "payer"
        [arguments]
        version = 2
        label = "ok"
    "#,
    )
    .unwrap()
}

#[test]
fn resolves_idl_order_flags_discriminator_and_borsh_arguments() {
    let binding = definition().resolve(&idl(), ID).unwrap();
    assert_eq!(
        binding
            .accounts
            .iter()
            .map(|a| a.idl_name.as_str())
            .collect::<Vec<_>>(),
        vec!["state", "authority"]
    );
    assert!(binding.accounts[0].writable && !binding.accounts[0].signer);
    assert!(!binding.accounts[1].writable && binding.accounts[1].signer);
    let mut sources = BTreeMap::new();
    sources.insert("position".into(), Address::from_str(ID).unwrap());
    sources.insert(
        "payer".into(),
        Address::from_str("11111111111111111111111111111111").unwrap(),
    );
    let ix = binding.build(&sources).unwrap();
    assert_eq!(ix.data, [7, 8, 2, 0, 2, 0, 0, 0, b'o', b'k']);
    assert_eq!(ix.accounts[0].pubkey, sources["position"]);
    assert!(ix.accounts[0].is_writable && !ix.accounts[0].is_signer);
    assert_eq!(ix.accounts[1].pubkey, sources["payer"]);
    assert!(!ix.accounts[1].is_writable && ix.accounts[1].is_signer);
}

#[test]
fn invalid_instruction_account_and_argument_fail_before_runtime() {
    let mut value = definition();
    value.instruction = "missing".into();
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("migration.instruction"));
    let mut value = definition();
    value
        .account_mapping
        .insert("unknown".into(), "payer".into());
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("unknown IDL account"));
    let mut value = definition();
    value.account_mapping.remove("state");
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("missing account binding"));
    let mut value = definition();
    value
        .arguments
        .insert("unknown".into(), toml::Value::Integer(1));
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("unknown IDL argument"));
    let mut value = definition();
    value
        .arguments
        .insert("version".into(), toml::Value::String("wrong".into()));
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("expected integer"));
    let mut value = definition();
    value
        .arguments
        .insert("version".into(), toml::Value::Integer(65536));
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("outside u16 range"));
    let mut value = definition();
    value.arguments.remove("label");
    assert!(value
        .resolve(&idl(), ID)
        .unwrap_err()
        .to_string()
        .contains("missing argument"));
}

#[test]
fn parser_rejects_missing_versions_and_invalid_project() {
    assert!(ProjectManifest::parse("[project]\nname='x'\nframework='anchor'").is_err());
    let source = format!("[project]\nname='x'\nframework='anchor'\n[versions.v1]\nprogram_path='v1.so'\nidl='v1.json'\nprogram_id='{ID}'\n[versions.v2]\nprogram_path='v2.so'\nidl='v2.json'\nprogram_id='{ID}'\n[migration]\ninstruction='migrate'\n[migration.account_mapping]\nstate='state'\n");
    ProjectManifest::parse(&source).unwrap();
    assert!(
        ProjectManifest::parse(&source.replace("name='x'", "name=''"))
            .unwrap_err()
            .to_string()
            .contains("project.name")
    );
    assert!(ProjectManifest::parse(&source.replace("v1.json", ""))
        .unwrap_err()
        .to_string()
        .contains("versions.v1.idl"));
    assert!(ProjectManifest::parse(&source.replace("v2.so", ""))
        .unwrap_err()
        .to_string()
        .contains("versions.v2.program_path"));
}

#[test]
fn load_resolves_paths_and_rejects_missing_artifacts() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work = std::env::temp_dir().join(format!("dg-config-{}-{unique}", std::process::id()));
    fs::create_dir(&work).unwrap();
    let mut old = json!({"address":ID,"metadata":{"name":"old","version":"1"},"instructions":[]});
    let new = json!({"address":ID,"metadata":{"name":"new","version":"2"},"instructions":[{"name":"migrate","discriminator":[7],"accounts":[{"name":"state"}],"args":[]}]});
    fs::write(work.join("old.json"), old.to_string()).unwrap();
    fs::write(work.join("new.json"), new.to_string()).unwrap();
    fs::write(work.join("old.so"), []).unwrap();
    fs::write(work.join("new.so"), []).unwrap();
    let manifest = format!("[project]\nname='ledger'\nframework='anchor'\n[versions.v1]\nprogram_path='old.so'\nidl='old.json'\nprogram_id='{ID}'\n[versions.v2]\nprogram_path='new.so'\nidl='new.json'\nprogram_id='{ID}'\n[migration]\ninstruction='migrate'\n[migration.account_mapping]\nstate='state'\n");
    fs::write(work.join("doppelganger.toml"), &manifest).unwrap();
    let loaded = ProjectManifest::load(&work).unwrap();
    assert_eq!(loaded.manifest.project.name, "ledger");
    assert_eq!(loaded.migration.accounts[0].source, "state");
    old["address"] = json!("wrong");
    fs::write(work.join("old.json"), old.to_string()).unwrap();
    assert!(ProjectManifest::load(&work)
        .unwrap_err()
        .to_string()
        .contains("versions.v1.program_id"));
    fs::write(
        work.join("old.json"),
        json!({"address":ID,"instructions":[]}).to_string(),
    )
    .unwrap();
    fs::rename(work.join("new.json"), work.join("missing.json")).unwrap();
    assert!(ProjectManifest::load(&work)
        .unwrap_err()
        .to_string()
        .contains("versions.v2.idl"));
    fs::remove_dir_all(work).unwrap();
}

#[test]
fn structured_arguments_encode_from_idl_types() {
    let raw = json!({
        "address":ID,
        "instructions":[{"name":"migrate","discriminator":[3],"accounts":[],"args":[
            {"name":"enabled","type":"bool"},
            {"name":"scores","type":{"vec":"u16"}},
            {"name":"maybe","type":{"option":"u8"}},
            {"name":"owner","type":"pubkey"},
            {"name":"details","type":{"defined":{"name":"Details"}}},
            {"name":"stage","type":{"defined":{"name":"Stage"}}}
        ]}],
        "types":[
            {"name":"Details","type":{"kind":"struct","fields":[{"name":"count","type":"u8"}]}},
            {"name":"Stage","type":{"kind":"enum","variants":[{"name":"Open"},{"name":"Closed","fields":["u8"]}]}}
        ]
    });
    let model = normalize(raw.to_string().as_bytes()).unwrap();
    let definition: MigrationDefinition = toml::from_str(&format!(
        "instruction='migrate'\n[account_mapping]\n[arguments]\nenabled=true\nscores=[2, 3]\nmaybe={{some=5}}\nowner='{ID}'\ndetails={{count=4}}\nstage={{variant='Closed', fields=[6]}}\n"
    )).unwrap();
    let binding = definition.resolve(&model, ID).unwrap();
    let ix = binding.build(&BTreeMap::new()).unwrap();
    let mut expected = vec![3, 1, 2, 0, 0, 0, 2, 0, 3, 0, 1, 5];
    expected.extend(Address::from_str(ID).unwrap().as_ref());
    expected.extend([4, 1, 6]);
    assert_eq!(ix.data, expected);
}

#[test]
fn migration_arguments_accept_full_width_typed_integers() {
    let model = normalize(
        json!({"instructions":[{"name":"migrate","discriminator":[1],"accounts":[],
        "args":[{"name":"amount","type":"u128"},{"name":"debt","type":"i128"}]}]})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let definition: MigrationDefinition = toml::from_str("instruction='migrate'\n[account_mapping]\n[arguments]\namount='u128:340282366920938463463374607431768211455'\ndebt='i128:-170141183460469231731687303715884105728'\n").unwrap();
    let instruction = definition
        .resolve(&model, ID)
        .unwrap()
        .build(&BTreeMap::new())
        .unwrap();
    assert_eq!(
        instruction.data,
        [
            vec![1],
            u128::MAX.to_le_bytes().to_vec(),
            i128::MIN.to_le_bytes().to_vec()
        ]
        .concat()
    );
}
