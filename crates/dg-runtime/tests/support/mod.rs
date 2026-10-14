#![allow(dead_code)]
use anchor_lang::Discriminator;
use dg_analysis::{model::ProgramModel, normalize};
use dg_config::{
    addresses::{AddressDefinition, ConstSeed, IntegerSeed, PubkeySeed, SeedDefinition},
    FixtureAccount, FixtureDefinition, MigrationDefinition, ResolvedAddresses,
};
use dg_runtime::{RuntimeWorldBuilder, WorldBuildResult};
use dg_state::ProgramArtifact;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn artifact() -> ProgramArtifact {
    ProgramArtifact {
        program_id: dg_r5_fixture::id().to_string(),
        program_path: root().join("target/deploy/r5/dg_r5_fixture.so"),
        idl_path: PathBuf::new(),
    }
}
pub fn model() -> ProgramModel {
    normalize(json!({"address":dg_r5_fixture::id().to_string(),"instructions":[
        {"name":"initialize","discriminator":dg_r5_fixture::instruction::Initialize::DISCRIMINATOR,"accounts":[{"name":"record","writable":true},{"name":"authority","signer":true,"writable":true},{"name":"system_program"}],"args":[{"name":"tag","type":"u8"},{"name":"value","type":"u64"}]},
        {"name":"advance","discriminator":dg_r5_fixture::instruction::Advance::DISCRIMINATOR,"accounts":[{"name":"record","writable":true},{"name":"authority","signer":true}],"args":[{"name":"expected","type":"u64"},{"name":"next","type":"u64"}]},
        {"name":"corrupt","discriminator":dg_r5_fixture::instruction::Corrupt::DISCRIMINATOR,"accounts":[{"name":"record","writable":true}],"args":[]}
    ],"accounts":[{"name":"Record","discriminator":dg_r5_fixture::Record::DISCRIMINATOR},{"name":"Blob","discriminator":[9]},{"name":"Raw","discriminator":[10]}],"types":[
        {"name":"Record","type":{"kind":"struct","fields":[{"name":"authority","type":"pubkey"},{"name":"value","type":"u64"}]}},
        {"name":"Blob","type":{"kind":"struct","fields":[{"name":"body","type":"string"}]}},
        {"name":"Raw","type":{"kind":"struct","fields":[{"name":"body","type":"bytes"}]}}
    ]}).to_string().as_bytes()).unwrap()
}
pub fn definitions(seed: Option<[u8; 32]>, funding: u64) -> BTreeMap<String, AddressDefinition> {
    let mut d = BTreeMap::from([
        (
            "owner".into(),
            AddressDefinition::Signer {
                test_seed: seed,
                lamports: toml::Value::String(format!("u64:{funding}")),
            },
        ),
        (
            "alias".into(),
            AddressDefinition::Reference {
                account: "$owner".into(),
            },
        ),
        (
            "program".into(),
            AddressDefinition::Program {
                version: "v1".into(),
            },
        ),
        (
            "system".into(),
            AddressDefinition::Literal {
                address: "11111111111111111111111111111111".into(),
            },
        ),
    ]);
    for tag in 0..2 {
        d.insert(
            format!("record{tag}"),
            AddressDefinition::Pda {
                program: "v1".into(),
                seeds: vec![
                    SeedDefinition::Const(ConstSeed {
                        r#const: "record".into(),
                    }),
                    SeedDefinition::Pubkey(PubkeySeed {
                        pubkey: "$owner".into(),
                    }),
                    SeedDefinition::Integer(IntegerSeed {
                        integer: format!("u8:{tag}"),
                    }),
                ],
            },
        );
    }
    d
}
pub fn resolved() -> ResolvedAddresses {
    resolve(definitions(Some([7; 32]), 10_000_000_000))
}
pub fn resolve(definitions: BTreeMap<String, AddressDefinition>) -> ResolvedAddresses {
    ResolvedAddresses::resolve(
        &definitions,
        &BTreeMap::from([("v1".into(), dg_r5_fixture::id())]),
    )
    .unwrap()
}
pub fn fixture() -> FixtureDefinition {
    toml::from_str("name='test'\npayer='owner'").unwrap()
}
pub fn observe(tag: u8) -> FixtureAccount {
    toml::from_str(&format!(
        "name='record{tag}'\ntype='Record'\naddress='$record{tag}'\nowner='$program'\nlamports=0"
    ))
    .unwrap()
}
pub fn init(tag: u8, value: i64) -> MigrationDefinition {
    MigrationDefinition {
        instruction: "initialize".into(),
        account_mapping: BTreeMap::from([
            ("record".into(), format!("record{tag}")),
            ("authority".into(), "alias".into()),
            ("system_program".into(), "system".into()),
        ]),
        arguments: BTreeMap::from([
            ("tag".into(), toml::Value::Integer(tag.into())),
            ("value".into(), toml::Value::Integer(value)),
        ]),
    }
}
pub fn advance(tag: u8, expected: i64, next: i64) -> MigrationDefinition {
    MigrationDefinition {
        instruction: "advance".into(),
        account_mapping: BTreeMap::from([
            ("record".into(), format!("record{tag}")),
            ("authority".into(), "alias".into()),
        ]),
        arguments: BTreeMap::from([
            ("expected".into(), toml::Value::Integer(expected)),
            ("next".into(), toml::Value::Integer(next)),
        ]),
    }
}
pub fn initialized() -> FixtureDefinition {
    let mut f = fixture();
    f.account.push(observe(0));
    f.initialize.push(init(0, 1));
    f
}
pub fn build(f: &FixtureDefinition) -> WorldBuildResult {
    build_with(f, resolved()).unwrap_or_else(|e| panic!("{e:#?}"))
}
pub fn build_with(
    f: &FixtureDefinition,
    r: ResolvedAddresses,
) -> dg_runtime::Result<WorldBuildResult> {
    let m = model();
    let actions = f.instructions(&m, &artifact().program_id, &r).unwrap();
    RuntimeWorldBuilder::default().build(artifact(), &m, f, r, actions)
}
