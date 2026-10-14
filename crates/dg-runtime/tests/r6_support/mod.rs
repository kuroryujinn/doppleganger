#![allow(dead_code)]
use anchor_lang::Discriminator;
use dg_analysis::{model::ProgramModel, normalize};
use dg_config::{FixtureDefinition, MigrationDefinition, MigrationTarget, ResolvedAddresses};
use dg_runtime::{
    migration::{MigrationRequest, Result},
    MigrationExecution, MigrationExecutor, RuntimeWorld, RuntimeWorldBuilder,
};
use dg_state::ProgramArtifact;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn artifact(v2: bool) -> ProgramArtifact {
    let v = if v2 { 2 } else { 1 };
    ProgramArtifact {
        program_id: dg_r6_fixture_v1::id().to_string(),
        program_path: root().join(format!("target/deploy/r6-v{v}/dg_r6_fixture_v{v}.so")),
        idl_path: PathBuf::new(),
    }
}
fn ix(name: &str, discriminator: &[u8], accounts: Value, args: Value) -> Value {
    json!({"name":name,"discriminator":discriminator,"accounts":accounts,"args":args})
}
pub fn idl(v2: bool) -> Value {
    let writable = json!([{"name":"record","writable":true},{"name":"authority","writable":true,"signer":true},{"name":"system_program"}]);
    let read = json!([{"name":"record"},{"name":"authority","signer":true}]);
    let instructions = if v2 {
        vec![
            ix(
                "migrate",
                dg_r6_fixture_v2::instruction::Migrate::DISCRIMINATOR,
                writable,
                json!([{"name":"mode","type":"u8"},{"name":"version","type":"u8"},{"name":"value","type":"u64"}]),
            ),
            ix(
                "reject",
                dg_r6_fixture_v2::instruction::Reject::DISCRIMINATOR,
                read.clone(),
                json!([]),
            ),
            ix(
                "create",
                dg_r6_fixture_v2::instruction::Create::DISCRIMINATOR,
                json!([{"name":"receipt","writable":true},{"name":"authority","writable":true,"signer":true},{"name":"system_program"}]),
                json!([]),
            ),
            ix(
                "close",
                dg_r6_fixture_v2::instruction::Close::DISCRIMINATOR,
                json!([{"name":"record","writable":true},{"name":"authority","writable":true,"signer":true}]),
                json!([]),
            ),
            ix(
                "arguments",
                dg_r6_fixture_v2::instruction::Arguments::DISCRIMINATOR,
                read,
                json!([{"name":"payload","type":{"defined":"Payload"}}]),
            ),
        ]
    } else {
        vec![
            ix(
                "initialize",
                dg_r6_fixture_v1::instruction::Initialize::DISCRIMINATOR,
                writable,
                json!([{"name":"value","type":"u64"}]),
            ),
            ix(
                "advance",
                dg_r6_fixture_v1::instruction::Advance::DISCRIMINATOR,
                json!([{"name":"record","writable":true},{"name":"authority","signer":true}]),
                json!([{"name":"value","type":"u64"}]),
            ),
        ]
    };
    let name = if v2 { "RecordV2" } else { "RecordV1" };
    let discriminator = if v2 {
        dg_r6_fixture_v2::RecordV2::DISCRIMINATOR
    } else {
        dg_r6_fixture_v1::RecordV1::DISCRIMINATOR
    };
    let mut fields = vec![
        json!({"name":"authority","type":"pubkey"}),
        json!({"name":"value","type":"u64"}),
    ];
    if v2 {
        fields.push(json!({"name":"version","type":"u8"}));
    }
    let mut types = vec![json!({"name":name,"type":{"kind":"struct","fields":fields}})];
    if v2 {
        types.extend([
            json!({"name":"Nested","type":{"kind":"struct","fields":[{"name":"flag","type":"bool"}]}}),
            json!({"name":"Choice","type":{"kind":"enum","variants":[{"name":"Empty"},{"name":"Number","fields":["u64"]}]}}),
            json!({"name":"Payload","type":{"kind":"struct","fields":[
                {"name":"wide","type":"u128"},{"name":"signed","type":"i128"},
                {"name":"nested","type":{"defined":"Nested"}},{"name":"choice","type":{"defined":"Choice"}},
                {"name":"values","type":{"vec":"u64"}},{"name":"maybe","type":{"option":"u64"}},
                {"name":"array","type":{"array":["u8",2]}},{"name":"pair","type":{"tuple":["u8","u16"]}}
            ]}}),
        ]);
    }
    json!({"address":artifact(v2).program_id,"instructions":instructions,"accounts":[{"name":name,"discriminator":discriminator}],"types":types})
}
pub fn model(v2: bool) -> ProgramModel {
    normalize(idl(v2).to_string().as_bytes()).unwrap()
}
pub fn definition(name: &str) -> MigrationDefinition {
    let mut d = MigrationDefinition {
        instruction: name.into(),
        account_mapping: BTreeMap::from([
            ("record".into(), "record".into()),
            ("authority".into(), "alias".into()),
        ]),
        arguments: BTreeMap::new(),
    };
    if matches!(name, "initialize" | "migrate" | "create") {
        d.account_mapping
            .insert("system_program".into(), "system".into());
    }
    if name == "create" {
        d.account_mapping.remove("record");
        d.account_mapping.insert("receipt".into(), "receipt".into());
    }
    if matches!(name, "initialize" | "advance" | "migrate") {
        d.arguments.insert("value".into(), toml::Value::Integer(42));
    }
    if name == "migrate" {
        d.arguments.extend([
            ("mode".into(), toml::Value::Integer(0)),
            ("version".into(), toml::Value::Integer(2)),
        ]);
    }
    d
}
pub fn world() -> RuntimeWorld {
    let seed = vec![7; 32];
    let definitions = toml::from_str(&format!(
        r#"
        [owner]
        type = "signer"
        test_seed = {seed:?}
        lamports = "u64:10000000000"
        [unused]
        type = "signer"
        test_seed = {:?}
        lamports = "u64:1000000000"
        [alias]
        type = "reference"
        account = "$owner"
        [program]
        type = "program"
        version = "v1"
        [system]
        type = "literal"
        address = "11111111111111111111111111111111"
        [record]
        type = "pda"
        program = "v1"
        seeds = [{{ const = "record" }}, {{ pubkey = "$owner" }}]
        [receipt]
        type = "pda"
        program = "v1"
        seeds = [{{ const = "receipt" }}, {{ pubkey = "$owner" }}]
    "#,
        vec![8; 32]
    ))
    .unwrap();
    let resolved = ResolvedAddresses::resolve(
        &definitions,
        &BTreeMap::from([("v1".into(), dg_r6_fixture_v1::id())]),
    )
    .unwrap();
    let mut f: FixtureDefinition = toml::from_str("name='r6'\npayer='owner'\n[[account]]\nname='record'\ntype='RecordV1'\naddress='$record'\nowner='$program'\nlamports=0").unwrap();
    let mut initialize = definition("initialize");
    initialize
        .arguments
        .insert("value".into(), toml::Value::Integer(1));
    f.initialize.push(initialize);
    f.setup.push(definition("advance"));
    let m = model(false);
    let actions = f
        .instructions(&m, &artifact(false).program_id, &resolved)
        .unwrap();
    RuntimeWorldBuilder::default()
        .build(artifact(false), &m, &f, resolved, actions)
        .unwrap()
        .world
}
pub fn targets() -> Vec<MigrationTarget> {
    vec![MigrationTarget {
        name: "record".into(),
        address: "$record".into(),
        v1_type: Some("RecordV1".into()),
        v2_type: Some("RecordV2".into()),
    }]
}
pub fn run(
    world: &mut RuntimeWorld,
    d: &MigrationDefinition,
    m: &ProgramModel,
    targets: &[MigrationTarget],
) -> Result<MigrationExecution> {
    MigrationExecutor::default().execute(
        world,
        MigrationRequest {
            project: "r6-fixture",
            candidate: artifact(true),
            model: m,
            definition: d,
            targets,
            payer: Some("owner"),
        },
    )
}
pub fn migrate(world: &mut RuntimeWorld) -> MigrationExecution {
    run(world, &definition("migrate"), &model(true), &targets()).unwrap()
}
