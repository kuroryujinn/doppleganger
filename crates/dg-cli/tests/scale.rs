//! Representative corpus qualification. No timing thresholds on shared CI hosts.
use dg_config::ProjectManifest;
use dg_runtime::{preservation, MigrationExecutor, RuntimeWorldBuilder};
use dg_state::{raw::RawAccountSnapshot, DecodeLimits, TrailingBytes};
use serde_json::json;
use solana_address::Address;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    time::Instant,
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn manifest(count: usize) -> ProjectManifest {
    let program = "9xQeWvG816bUx9EPfEZzU4kL9fVMKcVx5yFvTJTQHnN";
    let mut text=format!("schema_version=1\n[project]\nname='scale-record'\nframework='anchor'\n[versions.v1]\nprogram_id='{program}'\nprogram_path='target/deploy/r5/dg_r5_fixture.so'\nidl='target/idl/r5.json'\n[versions.v2]\nprogram_id='{program}'\nprogram_path='target/deploy/r5/dg_r5_fixture.so'\nidl='target/idl/r5.json'\n[addresses.owner]\ntype='signer'\ntest_seed=[{}]\nlamports='u64:10000000000'\n[addresses.program]\ntype='program'\nversion='v1'\n",vec!["7";32].join(","));
    for n in 0..count {
        let mut bytes = [42; 32];
        bytes[..4].copy_from_slice(&(n as u32).to_le_bytes());
        text += &format!(
            "[addresses.record{n}]\ntype='literal'\naddress='{}'\n",
            Address::new_from_array(bytes)
        );
    }
    text+="[migration]\ninstruction='advance'\n[migration.account_mapping]\nrecord='record0'\nauthority='owner'\n[migration.arguments]\nexpected='u64:10'\nnext='u64:20'\n[fixture]\nname='record-corpus'\npayer='owner'\n";
    for n in 0..count {
        text+=&format!("[[fixture.account]]\nname='record{n}'\ntype='Record'\naddress='$record{n}'\nowner='$program'\nlamports='u64:1300000'\n[fixture.account.fields]\nauthority='$owner'\nvalue='u64:10'\n");
    }
    for n in 0..count {
        text+=&format!("[[migration_targets]]\nname='record{n}'\naddress='$record{n}'\nv1_type='Record'\nv2_type='Record'\n[[preservation]]\nid='authority-{n}'\naccount='record{n}'\nfield='authority'\nrule='equal'\n");
    }
    text+="[[preservation]]\nid='advanced'\naccount='record0'\nfield='value'\nrule='default'\nvalue='u64:20'\n";
    ProjectManifest::parse(&text).unwrap()
}
fn graph_account(e: &dg_runtime::migration::AccountEvidence) -> dg_policy::relationships::Account {
    dg_policy::relationships::Account {
        name: e.name.clone(),
        address: e.address.clone(),
        owner: e.snapshot.as_ref().map(|s| s.owner.clone()),
        exists: e.snapshot.is_some(),
        fields: e.decoded.as_ref().map(|d| d.fields.clone()),
        decode_error: e.decode_error.as_ref().map(ToString::to_string),
    }
}
#[test]
fn one_to_thousand_accounts_execute_compare_and_report() {
    let root = root();
    let mut measurements = Vec::new();
    for count in [1, 10, 100, 1_000] {
        let m = manifest(count);
        let now = Instant::now();
        let idl =
            dg_analysis::read_bounded(&root.join("target/idl/r5.json"), 8 * 1024 * 1024).unwrap();
        let model = dg_analysis::normalize(&idl).unwrap();
        let analysis = dg_analysis::analyze(
            "scale-record",
            &model,
            &model,
            &[("Record".into(), "Record".into())],
        );
        let analysis_us = now.elapsed().as_micros();
        let now = Instant::now();
        let mut built = RuntimeWorldBuilder::default()
            .build_manifest(&m, &root)
            .unwrap();
        let world_us = now.elapsed().as_micros();
        assert_eq!(built.observed_accounts.len(), count);
        let now = Instant::now();
        let e = MigrationExecutor::default()
            .execute_manifest(&mut built.world, &m, &root)
            .unwrap();
        let migration_us = now.elapsed().as_micros();
        assert!(e.transaction.as_ref().unwrap().success);
        assert_eq!(e.after_accounts.len(), count);
        let now = Instant::now();
        for a in &e.after_accounts {
            let raw = a.snapshot.as_ref().unwrap();
            let decoded = dg_state::decode_account(
                &model,
                "Record",
                &raw.raw_data,
                DecodeLimits::default(),
                TrailingBytes::Reject,
            )
            .unwrap();
            assert_eq!(&decoded, a.decoded.as_ref().unwrap());
        }
        let decode_us = now.elapsed().as_micros();
        let now = Instant::now();
        let preserve = preservation::evaluate(&e, &m.preservation, &model).unwrap();
        assert_eq!(preserve.failed, 0);
        assert_eq!(preserve.errors, 0);
        let rules = (0..count)
            .map(|n| dg_policy::relationships::Relationship {
                id: format!("edge-{n}"),
                kind: dg_policy::relationships::Kind::Authority,
                account: format!("record{n}"),
                field: Some("authority".into()),
                before_field: None,
                target: "$owner".into(),
                check_before: true,
                preserve: true,
            })
            .collect::<Vec<_>>();
        let graph = dg_policy::relationships::evaluate(
            &rules,
            &e.before_accounts
                .iter()
                .map(graph_account)
                .collect::<Vec<_>>(),
            &e.after_accounts
                .iter()
                .map(graph_account)
                .collect::<Vec<_>>(),
            &built.world.resolved.strings(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(graph.passed, count);
        let comparison_us = now.elapsed().as_micros();
        let now = Instant::now();
        let report=serde_json::to_vec(&json!({"analysis":analysis,"execution":e,"preservation":preserve,"relationships":graph})).unwrap();
        let report_us = now.elapsed().as_micros();
        assert!(!report.is_empty());
        measurements.push(json!({"accounts":count,"analysis_us":analysis_us,"world_build_us":world_us,"migration_and_capture_us":migration_us,"decoding_us":decode_us,"comparison_and_graph_us":comparison_us,"report_generation_us":report_us,"report_bytes":report.len()}));
    }
    let directory = root.join("report/mvp-qualification");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("account-scale.json"),serde_json::to_vec_pretty(&json!({"schema_version":1,"scope":"one configured migration transaction with 1–1000 observed synthetic accounts; timings include observation where named","measurements":measurements})).unwrap()).unwrap();
}
#[test]
fn ten_thousand_snapshot_records_round_trip_and_validate() {
    let now = Instant::now();
    let mut accounts = BTreeMap::new();
    for n in 0..10_000u32 {
        let mut bytes = [42; 32];
        bytes[..4].copy_from_slice(&n.to_le_bytes());
        let address = Address::new_from_array(bytes).to_string();
        let raw = RawAccountSnapshot::capture(
            address.clone(),
            Address::new_from_array([99; 32]).to_string(),
            1_000_000,
            false,
            vec![n as u8; 48],
            DecodeLimits::default(),
        )
        .unwrap();
        accounts.insert(address, raw);
    }
    let data = serde_json::to_vec(&accounts).unwrap();
    let serialization_us = now.elapsed().as_micros();
    let now = Instant::now();
    let restored: BTreeMap<String, RawAccountSnapshot> = serde_json::from_slice(&data).unwrap();
    for raw in restored.values() {
        raw.validate(DecodeLimits::default()).unwrap();
    }
    assert_eq!(accounts, restored);
    let loading_and_validation_us = now.elapsed().as_micros();
    let directory = root().join("report/mvp-qualification");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("snapshot-scale.json"),serde_json::to_vec_pretty(&json!({"records":10_000,"bytes":data.len(),"serialization_us":serialization_us,"loading_and_validation_us":loading_and_validation_us,"scope":"generic stored raw snapshot corpus; Devnet importer retains its single-batch 100-account limit"})).unwrap()).unwrap();
}
