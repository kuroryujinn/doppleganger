use base64::{engine::general_purpose::STANDARD, Engine};
use dg_config::{
    snapshot::{SnapshotAccount, SnapshotConfig, SnapshotSource},
    ProjectManifest,
};
use dg_runtime::{MigrationExecutor, RuntimeWorldBuilder};
use dg_snapshot::{
    capture, digest, load,
    replay::{build_world, replay, CandidateOverride},
    rpc::{parse_account, AccountBatch, HttpRpc, ReadOnlyRpc},
    save, Acquisition, DecodeStatus, DevnetSnapshot, DEVNET_GENESIS,
};
use serde_json::{json, Value};
use solana_address::Address;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn project() -> (ProjectManifest, PathBuf) {
    let base = root().join("examples/escrow-runtime");
    let mut m =
        ProjectManifest::parse(&fs::read_to_string(base.join("doppelganger.toml")).unwrap())
            .unwrap();
    m.snapshot = Some(SnapshotConfig {
        source: SnapshotSource::Devnet,
        rpc_url: None,
        rpc_url_env: None,
        min_context_slot: Some(40),
        max_account_bytes: 1024,
        programs: vec![],
        accounts: vec![
            SnapshotAccount {
                name: "escrow".into(),
                address: "$escrow".into(),
                owner: None,
                account_type: None,
                optional: false,
            },
            SnapshotAccount {
                name: "initializer".into(),
                address: "$initializer".into(),
                owner: Some("$system_program".into()),
                account_type: None,
                optional: false,
            },
            SnapshotAccount {
                name: "beneficiary".into(),
                address: "$beneficiary".into(),
                owner: Some("$system_program".into()),
                account_type: None,
                optional: false,
            },
        ],
    });
    (m, base)
}

struct MockRpc {
    values: BTreeMap<String, Value>,
    slot: u64,
    genesis: String,
    calls: Mutex<Vec<&'static str>>,
}
impl ReadOnlyRpc for MockRpc {
    fn genesis_hash(&self) -> dg_snapshot::Result<String> {
        self.calls.lock().unwrap().push("getGenesisHash");
        Ok(self.genesis.clone())
    }
    fn multiple_accounts(
        &self,
        addresses: &[String],
        min: Option<u64>,
    ) -> dg_snapshot::Result<AccountBatch> {
        self.calls.lock().unwrap().push("getMultipleAccounts");
        assert_eq!(min, Some(40));
        Ok(AccountBatch {
            slot: self.slot,
            accounts: addresses
                .iter()
                .map(|a| self.values.get(a).cloned().unwrap_or(Value::Null))
                .collect(),
        })
    }
    fn origin(&self) -> String {
        "https://user:secret@rpc.example.test/path-api-key?token=secret".into()
    }
    fn acquisition(&self) -> Acquisition {
        Acquisition::MockRpc
    }
}
fn rpc() -> MockRpc {
    let (m, base) = project();
    let built = RuntimeWorldBuilder::default()
        .build_manifest(&m, &base)
        .unwrap();
    let values=m.snapshot.as_ref().unwrap().accounts.iter().map(|a| {
        let address=built.world.resolved.addresses[&a.name];
        let account=built.world.runtime.get_account(&address).unwrap();
        (address.to_string(),json!({"data":[STANDARD.encode(&account.data),"base64"],"owner":account.owner.to_string(),"lamports":account.lamports,"executable":account.executable,"rentEpoch":account.rent_epoch,"space":account.data.len()}))
    }).collect();
    MockRpc {
        values,
        slot: 42,
        genesis: DEVNET_GENESIS.into(),
        calls: Mutex::new(vec![]),
    }
}
fn snapshot() -> DevnetSnapshot {
    let (m, base) = project();
    capture(&m, &base, &rpc()).unwrap()
}
fn fixture() -> DevnetSnapshot {
    load(&root().join("fixtures/devnet-snapshots/escrow-mock")).unwrap()
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "dg-r11-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Fixture generation is explicit and never calls a real RPC or handles secret keys.
fn seal(s: &mut DevnetSnapshot) {
    for a in &mut s.accounts {
        a.data_sha256 = a.raw.as_ref().map(|r| digest(&r.raw_data));
        a.record_sha256.clear();
        a.record_sha256 = digest(&serde_json::to_vec_pretty(a).unwrap());
    }
    s.snapshot_id.clear();
    s.snapshot_sha256.clear();
    let mut identity = s.clone();
    identity.provenance.captured_at_unix = 0;
    let id = format!(
        "SNAP-{}",
        digest(&serde_json::to_vec_pretty(&identity).unwrap())
    );
    let hash = digest(&serde_json::to_vec_pretty(s).unwrap());
    s.snapshot_id = id;
    s.snapshot_sha256 = hash;
}

#[test]
fn capture_preserves_data_provenance_and_read_only_calls() {
    let (m, base) = project();
    let rpc = rpc();
    let mut s = capture(&m, &base, &rpc).unwrap();
    assert_eq!(
        *rpc.calls.lock().unwrap(),
        ["getGenesisHash", "getMultipleAccounts"]
    );
    assert_eq!(s.provenance.rpc_origin, "https://rpc.example.test");
    assert_eq!(s.provenance.slot, 42);
    assert!(!s.provenance.non_atomic_capture);
    assert_eq!(
        s.accounts
            .iter()
            .filter(|a| a.decode_status == DecodeStatus::Decoded)
            .count(),
        1
    );
    for a in &s.accounts {
        let raw = a.raw.as_ref().unwrap();
        assert_eq!(
            raw.raw_data,
            STANDARD
                .decode(rpc.values[&a.address]["data"][0].as_str().unwrap())
                .unwrap()
        );
        assert_eq!(a.data_sha256, Some(digest(&raw.raw_data)));
    }
    assert!(!serde_json::to_string(&s).unwrap().contains("secret"));
    if std::env::var("DG_UPDATE_MOCK_SNAPSHOT").as_deref() == Ok("1") {
        s.provenance.captured_at_unix = 0;
        // SBF builds may differ by toolchain; this mock pins the IDL and raw state.
        s.provenance.artifacts.get_mut("v1_binary").unwrap().sha256 = None;
        seal(&mut s);
        fs::write(
            root().join("fixtures/devnet-snapshots/escrow-mock/snapshot.json"),
            serde_json::to_vec_pretty(&s).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn immutable_storage_identity_and_tamper_detection() {
    let temp = Temp::new();
    let s = snapshot();
    let p = save(&s, &temp.0).unwrap();
    let mut later = s.clone();
    later.provenance.captured_at_unix += 1;
    seal(&mut later);
    assert_eq!(later.snapshot_id, s.snapshot_id);
    assert_ne!(later.snapshot_sha256, s.snapshot_sha256);
    save(&later, &temp.0).unwrap();
    assert_eq!(load(&p).unwrap(), s);
    let mut changed = s.clone();
    changed.accounts[0].raw.as_mut().unwrap().lamports += 1;
    seal(&mut changed);
    assert_ne!(changed.snapshot_id, s.snapshot_id);
    assert_eq!(save(&changed, &temp.0).unwrap_err().code, "SNAPSHOT_EXISTS");
    let mut a = s.clone();
    a.accounts[0].raw.as_mut().unwrap().raw_data.push(1);
    assert_eq!(a.verify().unwrap_err().code, "ACCOUNT_HASH_MISMATCH");
    let mut a = s.clone();
    a.provenance.slot += 1;
    assert_eq!(a.verify().unwrap_err().code, "SNAPSHOT_HASH_MISMATCH");
    let mut a = s.clone();
    a.schema_version = 2;
    assert_eq!(a.verify().unwrap_err().code, "SNAPSHOT_SCHEMA_UNSUPPORTED");
    fs::write(&p, serde_json::to_vec(&a).unwrap()).unwrap();
    assert_eq!(load(&p).unwrap_err().code, "SNAPSHOT_SCHEMA_UNSUPPORTED");
}

#[test]
fn full_devnet_genesis_is_accepted_and_truncated_hash_is_rejected() {
    let (m, base) = project();
    let mut rpc = rpc();
    // Independent literal observed from public Devnet, not the production constant.
    rpc.genesis = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG".into();
    let s = capture(&m, &base, &rpc).unwrap();
    s.verify().unwrap();
    rpc.genesis = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1".into();
    assert_eq!(
        capture(&m, &base, &rpc).unwrap_err().code,
        "CLUSTER_MISMATCH"
    );
}

#[test]
fn cluster_slot_owner_and_absence_are_distinct() {
    let (mut m, base) = project();
    let mut rpc = rpc();
    rpc.genesis = "mainnet".into();
    assert_eq!(
        capture(&m, &base, &rpc).unwrap_err().code,
        "CLUSTER_MISMATCH"
    );
    rpc.genesis = DEVNET_GENESIS.into();
    rpc.slot = 39;
    assert_eq!(
        capture(&m, &base, &rpc).unwrap_err().code,
        "SLOT_CONSISTENCY_FAILED"
    );
    rpc.slot = 42;
    rpc.values.clear();
    assert_eq!(
        capture(&m, &base, &rpc).unwrap_err().code,
        "ACCOUNT_NOT_FOUND"
    );
    for a in &mut m.snapshot.as_mut().unwrap().accounts {
        a.optional = true;
    }
    let s = capture(&m, &base, &rpc).unwrap();
    assert!(s
        .accounts
        .iter()
        .all(|a| !a.exists && a.raw.is_none() && a.data_sha256.is_none()));
    let restored = build_world(&s, &m, &base).unwrap();
    for a in &s.accounts {
        assert!(restored
            .world
            .runtime
            .get_account(&a.address.parse().unwrap())
            .is_none());
    }
    let rpc = super_rpc_with_wrong_owner();
    assert_eq!(
        capture(&project().0, &base, &rpc).unwrap_err().code,
        "OWNER_MISMATCH"
    );
}
fn super_rpc_with_wrong_owner() -> MockRpc {
    let mut rpc = rpc();
    for value in rpc.values.values_mut() {
        value["owner"] = json!(Address::new_from_array([99; 32]).to_string());
    }
    rpc
}

#[test]
fn malformed_and_oversized_rpc_accounts_are_rejected() {
    let pubkey = Address::new_from_array([1; 32]).to_string();
    let good = json!({"owner":"11111111111111111111111111111111","lamports":u64::MAX,"rentEpoch":u64::MAX,"executable":false,"data":["AQID","base64"],"space":3});
    assert_eq!(
        parse_account(&pubkey, good.clone(), 3).unwrap().unwrap().1,
        u64::MAX
    );
    for (key, bad) in [
        ("owner", json!("invalid")),
        ("lamports", json!(-1)),
        ("rentEpoch", json!("0")),
        ("executable", json!(0)),
        ("data", json!(["!", "base64"])),
        ("space", json!(4)),
    ] {
        let mut v = good.clone();
        v[key] = bad;
        assert_eq!(
            parse_account(&pubkey, v, 3).unwrap_err().code,
            "RPC_RESPONSE_INVALID"
        );
    }
    assert_eq!(
        parse_account(&pubkey, good, 2).unwrap_err().code,
        "ACCOUNT_DATA_TOO_LARGE"
    );
    assert_eq!(
        parse_account("invalid", Value::Null, 3).unwrap_err().code,
        "RPC_RESPONSE_INVALID"
    );
}

#[test]
fn decode_error_keeps_authoritative_raw_bytes() {
    let (m, base) = project();
    let mut rpc = rpc();
    let original = snapshot();
    let escrow = original
        .accounts
        .iter()
        .find(|a| a.logical_name == "escrow")
        .unwrap();
    let v = rpc.values.get_mut(&escrow.address).unwrap();
    v["data"] = json!(["AQID", "base64"]);
    v["space"] = json!(3);
    let s = capture(&m, &base, &rpc).unwrap();
    let a = s
        .accounts
        .iter()
        .find(|a| a.logical_name == "escrow")
        .unwrap();
    assert_eq!(a.decode_status, DecodeStatus::Error);
    assert!(a.decode_error.is_some());
    assert_eq!(a.raw.as_ref().unwrap().raw_data, [1, 2, 3]);
    s.verify().unwrap();
}

#[test]
fn offline_restore_matches_r5_and_good_candidate_is_deterministic() {
    let (m, base) = project();
    let s = fixture();
    let mut local = RuntimeWorldBuilder::default()
        .build_manifest(&m, &base)
        .unwrap();
    let imported = build_world(&s, &m, &base).unwrap();
    assert!(imported.executed_actions.is_empty());
    for a in &s.accounts {
        let key = a.address.parse().unwrap();
        let actual = imported.world.runtime.get_account(&key).unwrap();
        assert_eq!(Some(actual.clone()), local.world.runtime.get_account(&key));
        let raw = a.raw.as_ref().unwrap();
        assert_eq!(actual.data, raw.raw_data);
        assert_eq!(actual.owner.to_string(), raw.owner);
        assert_eq!(actual.lamports.to_string(), raw.lamports.to_string());
        assert_eq!(actual.executable, raw.executable);
    }
    let expected = MigrationExecutor::default()
        .execute_manifest(&mut local.world, &m, &base)
        .unwrap();
    let a = replay(&s, &m, &base, &Default::default(), true).unwrap();
    let b = replay(&s, &m, &base, &Default::default(), true).unwrap();
    assert_eq!(a, b);
    let expected = serde_json::to_value(expected).unwrap();
    for field in ["before_accounts", "after_accounts"] {
        assert_eq!(a["execution"][field], expected[field]);
    }
    assert_eq!(
        a["execution"]["transaction"]["success"],
        expected["transaction"]["success"]
    );
    assert_eq!(a["preservation"]["passed"], 11);
    assert_eq!(a["lifecycle"]["passed"], 6);
    assert_eq!(a["lifecycle"]["errors"], 0);
    assert_eq!(a["remote_transactions_sent"], false);
    assert_eq!(a["migration_executed_locally"], true);
}

#[test]
fn same_snapshot_compares_candidates_and_r10_reproduces_offline() {
    let (m, base) = project();
    let s = fixture();
    let temp = Temp::new();
    let binary = root().join("target/deploy/escrow-broken/dg_escrow_v2.so");
    let good = replay(&s, &m, &base, &Default::default(), true).unwrap();
    let broken = replay(
        &s,
        &m,
        &base,
        &CandidateOverride {
            binary: Some(binary.clone()),
            idl: None,
        },
        true,
    )
    .unwrap();
    assert_eq!(good["snapshot"], broken["snapshot"]);
    assert_eq!(
        good["execution"]["before_accounts"],
        broken["execution"]["before_accounts"]
    );
    assert_eq!(broken["execution"]["transaction"]["success"], true);
    assert_eq!(broken["preservation"]["failed"], 1);
    assert_eq!(
        broken["preservation"]["findings"][0]["id"],
        "PRESERVE-amount-retained"
    );
    let report = temp.0.join("report.json");
    fs::write(&report, serde_json::to_vec_pretty(&broken).unwrap()).unwrap();
    let mut config: toml::Value =
        toml::from_str(&fs::read_to_string(base.join("doppelganger.toml")).unwrap()).unwrap();
    for v in ["v1", "v2"] {
        for field in ["program_path", "idl"] {
            config["versions"][v][field] = toml::Value::String(
                fs::canonicalize(base.join(config["versions"][v][field].as_str().unwrap()))
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    let mut settings = toml::Value::try_from(m.snapshot.as_ref().unwrap()).unwrap();
    settings.as_table_mut().unwrap().insert(
        "rpc_url".into(),
        toml::Value::String("https://user:secret@localhost:1/api-key?token=secret".into()),
    );
    config
        .as_table_mut()
        .unwrap()
        .insert("snapshot".into(), settings);
    let manifest = temp.0.join("doppelganger.toml");
    fs::write(&manifest, toml::to_string_pretty(&config).unwrap()).unwrap();
    let case = dg_regression::create(
        &report,
        &manifest,
        dg_regression::Source::Preservation("PRESERVE-amount-retained".into()),
        &temp.0.join("regressions"),
    )
    .unwrap();
    assert_eq!(
        case.case.reconstruction,
        dg_regression::Reconstruction::SnapshotReplay
    );
    assert_eq!(
        load(&case.directory.join("snapshots/source.json")).unwrap(),
        s
    );
    assert!(!fs::read_to_string(case.directory.join("manifest.toml"))
        .unwrap()
        .contains("secret"));
    fs::remove_file(report).unwrap();
    let r = dg_regression::reproduce(&case.directory, &Default::default()).unwrap();
    assert_eq!(r.result, dg_regression::ReplayStatus::Reproduced);
    let r = dg_regression::reproduce(
        &case.directory,
        &dg_regression::CandidateOverride {
            binary: Some(root().join("target/deploy/escrow-v2/dg_escrow_v2.so")),
            idl: None,
        },
    )
    .unwrap();
    assert_eq!(r.result, dg_regression::ReplayStatus::NotReproduced);
}

#[test]
fn unavailable_signers_are_never_forged_and_mismatch_is_explicit() {
    let (mut m, base) = project();
    let s = fixture();
    let signer = s
        .accounts
        .iter()
        .find(|a| a.logical_name == "initializer")
        .unwrap();
    let literal = dg_config::AddressDefinition::Literal {
        address: signer.address.clone(),
    };
    m.addresses.insert("initializer".into(), literal);
    assert_eq!(
        replay(&s, &m, &base, &Default::default(), false)
            .unwrap_err()
            .code,
        "MISSING_LOCAL_SIGNER"
    );
    let (mut m, base) = project();
    if let dg_config::AddressDefinition::Signer { test_seed, .. } =
        m.addresses.get_mut("initializer").unwrap()
    {
        *test_seed = Some([99; 32]);
    }
    // Put the imported signer first so the specific error precedes a dependent PDA mismatch.
    m.snapshot
        .as_mut()
        .unwrap()
        .accounts
        .sort_by_key(|a| a.name != "initializer");
    assert_eq!(
        build_world(&s, &m, &base).err().unwrap().code,
        "SIGNER_ADDRESS_MISMATCH"
    );
    let (mut m, base) = project();
    let beneficiary = s
        .accounts
        .iter()
        .find(|a| a.logical_name == "beneficiary")
        .unwrap();
    m.addresses.insert(
        "beneficiary".into(),
        dg_config::AddressDefinition::Literal {
            address: beneficiary.address.clone(),
        },
    );
    let report = replay(&s, &m, &base, &Default::default(), true).unwrap();
    assert_eq!(report["execution"]["transaction"]["success"], true);
    assert!(report["lifecycle"]["errors"].as_u64().unwrap() > 0);
    assert!(report["lifecycle"]["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["status"] == "ERROR")
        .all(|s| s.to_string().contains("MISSING_SIGNER")));
}

#[test]
fn bounds_and_candidate_compatibility_are_enforced() {
    let (mut m, base) = project();
    let s = fixture();
    let c = m.snapshot.as_mut().unwrap();
    let a = c.accounts[0].clone();
    c.accounts = vec![a; 101];
    assert_eq!(
        capture(&m, &base, &rpc()).unwrap_err().code,
        "ACCOUNT_DATA_TOO_LARGE"
    );
    let (m, base) = project();
    assert_eq!(
        replay(
            &s,
            &m,
            &base,
            &CandidateOverride {
                binary: None,
                idl: Some(root().join("fixtures/escrow/idl/v2.json"))
            },
            false
        )
        .unwrap_err()
        .code,
        "CANDIDATE_INCOMPATIBLE"
    );
    assert_eq!(
        replay(
            &s,
            &m,
            &base,
            &CandidateOverride {
                binary: Some("absent.so".into()),
                idl: None
            },
            false
        )
        .unwrap_err()
        .code,
        "MISSING_PROGRAM_ARTIFACT"
    );
    let (mut m, base) = project();
    m.versions.v1.program_path = "absent.so".into();
    assert_eq!(
        replay(&s, &m, &base, &Default::default(), false)
            .unwrap_err()
            .code,
        "MISSING_PROGRAM_ARTIFACT"
    );
}

#[test]
fn snapshot_keeps_zero_lamport_accounts_and_changed_state_has_new_identity() {
    let (m, base) = project();
    let mut rpc = rpc();
    let initial = capture(&m, &base, &rpc).unwrap();
    for value in rpc.values.values_mut() {
        value["lamports"] = json!(0);
    }
    let changed = capture(&m, &base, &rpc).unwrap();
    assert_ne!(initial.snapshot_id, changed.snapshot_id);
    let world = build_world(&changed, &m, &base).unwrap().world;
    for a in &changed.accounts {
        let actual = world
            .runtime
            .get_account(&a.address.parse().unwrap())
            .unwrap();
        assert_eq!(actual.lamports, 0);
        assert_eq!(actual.data, a.raw.as_ref().unwrap().raw_data);
    }
    let (m, base) = project();
    assert_eq!(
        replay(&fixture(), &m, &base, &Default::default(), false).unwrap()["preservation"]
            ["passed"],
        11
    );
}

#[test]
fn http_rejects_malformed_context_envelope_and_account_count() {
    let (m, base) = project();
    for result in [
        json!({"context":{"slot":-1},"value":[]}),
        json!({"context":{"slot":42},"value":[]}),
        json!({"value":[]}),
    ] {
        let (url, _, task) = server(vec![
            (
                200,
                json!({"jsonrpc":"2.0","id":1,"result":DEVNET_GENESIS}).to_string(),
                Duration::ZERO,
            ),
            (
                200,
                json!({"jsonrpc":"2.0","id":1,"result":result}).to_string(),
                Duration::ZERO,
            ),
        ]);
        let e = capture(
            &m,
            &base,
            &HttpRpc::new(&url, Duration::from_secs(3)).unwrap(),
        )
        .unwrap_err();
        task.join().unwrap();
        assert_eq!(e.code, "RPC_RESPONSE_INVALID");
    }
    let (url, _, task) = server(vec![(
        200,
        json!({"jsonrpc":"2.0","id":2,"result":DEVNET_GENESIS}).to_string(),
        Duration::ZERO,
    )]);
    assert_eq!(
        HttpRpc::new(&url, Duration::from_secs(3))
            .unwrap()
            .genesis_hash()
            .unwrap_err()
            .code,
        "RPC_RESPONSE_INVALID"
    );
    task.join().unwrap();
}

fn server(
    responses: Vec<(u16, String, Duration)>,
) -> (String, Arc<Mutex<Vec<Value>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(vec![]));
    let copy = calls.clone();
    let task = thread::spawn(move || {
        for (status, body, delay) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = vec![];
            let mut buf = [0; 4096];
            let end = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < end + length {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            copy.lock()
                .unwrap()
                .push(serde_json::from_slice(&bytes[end..end + length]).unwrap());
            thread::sleep(delay);
            let response=format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (endpoint, calls, task)
}

#[test]
fn http_capture_sends_only_allowed_reads_and_records_batch_context() {
    let (m, base) = project();
    let values = rpc();
    let s = snapshot();
    let resolved = build_world(&s, &m, &base).unwrap().world.resolved;
    let accounts: Vec<_> = m
        .snapshot
        .as_ref()
        .unwrap()
        .accounts
        .iter()
        .map(|a| values.values[&resolved.addresses[&a.name].to_string()].clone())
        .collect();
    let (url, calls, task) = server(vec![
        (
            200,
            json!({"jsonrpc":"2.0","id":1,"result":DEVNET_GENESIS}).to_string(),
            Duration::ZERO,
        ),
        (
            200,
            json!({"jsonrpc":"2.0","id":1,"result":{"context":{"slot":42},"value":accounts}})
                .to_string(),
            Duration::ZERO,
        ),
    ]);
    let http = HttpRpc::new(&url, Duration::from_secs(3)).unwrap();
    let captured = capture(&m, &base, &http).unwrap();
    task.join().unwrap();
    assert_eq!(captured.provenance.acquisition, Acquisition::Rpc);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["method"], "getGenesisHash");
    assert_eq!(calls[1]["method"], "getMultipleAccounts");
    assert_eq!(calls[1]["params"][1]["minContextSlot"], 40);
    assert_eq!(
        calls
            .iter()
            .filter(|c| !["getGenesisHash", "getMultipleAccounts"]
                .contains(&c["method"].as_str().unwrap()))
            .count(),
        0
    );
    // The mock server is now closed; replay must still succeed with no transport.
    assert_eq!(
        replay(&captured, &m, &base, &Default::default(), false).unwrap()["preservation"]["passed"],
        11
    );
}

#[test]
fn http_failures_are_structured_and_redacted() {
    for (status, body, delay, expected) in [
        (429, "secret".to_owned(), Duration::ZERO, "RPC_RATE_LIMITED"),
        (503, "secret".to_owned(), Duration::ZERO, "RPC_UNAVAILABLE"),
        (
            200,
            "not-json-secret".to_owned(),
            Duration::ZERO,
            "RPC_RESPONSE_INVALID",
        ),
        (
            200,
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32016,"message":"secret"}}).to_string(),
            Duration::ZERO,
            "SLOT_CONSISTENCY_FAILED",
        ),
        (
            200,
            json!({"jsonrpc":"2.0","id":1,"result":DEVNET_GENESIS}).to_string(),
            Duration::from_secs(1),
            "RPC_TIMEOUT",
        ),
    ] {
        let (url, _, task) = server(vec![(status, body, delay)]);
        let timeout = if expected == "RPC_TIMEOUT" {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(5)
        };
        let http = HttpRpc::new(&format!("{url}/secret?token=secret"), timeout).unwrap();
        let error = http.genesis_hash().unwrap_err();
        task.join().unwrap();
        assert_eq!(error.code, expected);
        assert!(!error.to_string().contains("secret"));
        assert_eq!(http.origin(), url);
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    assert_eq!(
        HttpRpc::new(&url, Duration::from_secs(5))
            .unwrap()
            .genesis_hash()
            .unwrap_err()
            .code,
        "RPC_UNAVAILABLE"
    );
}

#[test]
fn retained_devnet_vault_restores_and_compares_candidates_offline() {
    let directory = root().join("fixtures/devnet-snapshots/vault-devnet");
    let s = load(&directory).unwrap();
    assert_eq!(s.provenance.acquisition, Acquisition::Rpc);
    assert_eq!(s.provenance.slot, 507756125);
    let base = root().join("examples/vault-runtime-devnet");
    let mut m =
        ProjectManifest::parse(&fs::read_to_string(base.join("doppelganger.toml")).unwrap())
            .unwrap();
    let config = m.snapshot.as_mut().unwrap();
    config.rpc_url_env = None;
    config.rpc_url = Some("http://127.0.0.1:1".into());
    let imported = build_world(&s, &m, &base).unwrap();
    assert!(imported.executed_actions.is_empty());
    for a in &s.accounts {
        let raw = a.raw.as_ref().unwrap();
        let actual = imported
            .world
            .runtime
            .get_account(&a.address.parse().unwrap())
            .unwrap();
        assert_eq!(actual.owner.to_string(), raw.owner);
        assert_eq!(actual.lamports, u64::try_from(&raw.lamports).unwrap());
        assert_eq!(actual.executable, raw.executable);
        assert_eq!(actual.data.len(), raw.data_length);
        assert_eq!(actual.data, raw.raw_data);
        assert_eq!(Some(actual.rent_epoch.to_string()), a.rent_epoch);
    }
    let good = replay(&s, &m, &base, &Default::default(), true).unwrap();
    let broken = replay(
        &s,
        &m,
        &base,
        &CandidateOverride {
            binary: Some(directory.join("artifacts/v2-broken.bin")),
            idl: None,
        },
        true,
    )
    .unwrap();
    for report in [&good, &broken] {
        assert_eq!(report["snapshot"], serde_json::to_value(&s).unwrap());
        assert_eq!(report["execution"]["transaction"]["success"], true);
        assert_eq!(report["remote_state_imported"], true);
        assert_eq!(report["rpc_write_calls"], 0);
        assert_eq!(report["lifecycle"]["passed"], 7);
    }
    assert_eq!(good["preservation"]["passed"], 7);
    assert_eq!(broken["preservation"]["passed"], 6);
    assert_eq!(
        broken["preservation"]["findings"][0]["id"],
        "PRESERVE-balance-field"
    );
    let mut tampered = s.clone();
    tampered
        .accounts
        .iter_mut()
        .find(|a| a.logical_name == "vault")
        .unwrap()
        .raw
        .as_mut()
        .unwrap()
        .raw_data[40] ^= 1;
    let temp = Temp::new();
    let copy = temp.0.join("snapshot.json");
    fs::write(&copy, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert_eq!(load(&copy).unwrap_err().code, "ACCOUNT_HASH_MISMATCH");
    assert_eq!(load(&directory).unwrap(), s);
}

#[test]
#[ignore = "requires an explicitly configured disposable Devnet application"]
fn live_devnet_application_capture() {
    assert_eq!(std::env::var("DG_RUN_DEVNET_TESTS").as_deref(), Ok("1"));
    let path = PathBuf::from(std::env::var("DG_DEVNET_PROJECT").expect("DG_DEVNET_PROJECT"));
    let manifest_path = if path.is_dir() {
        path.join("doppelganger.toml")
    } else {
        path
    };
    let m = ProjectManifest::parse(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let base = manifest_path.parent().unwrap();
    let http = HttpRpc::from_config(m.snapshot.as_ref().expect("snapshot config")).unwrap();
    let s = capture(&m, base, &http).unwrap();
    assert_eq!(s.provenance.acquisition, Acquisition::Rpc);
    assert!(s.provenance.slot > 0);
    let targets = MigrationExecutor::targets(&m);
    assert!(!targets.is_empty(), "application migration target required");
    for target in targets {
        let a = s
            .accounts
            .iter()
            .find(|a| a.logical_name == target.name)
            .unwrap();
        let raw = a.raw.as_ref().expect("live application must exist");
        assert_eq!(raw.owner, m.versions.v1.program_id);
        assert!(u64::try_from(&raw.lamports).unwrap() > 0);
        assert!(!raw.raw_data.is_empty());
        assert_eq!(a.decode_status, DecodeStatus::Decoded);
    }
    let temp = Temp::new();
    save(&s, &temp.0).unwrap();
    assert_eq!(load(&temp.0).unwrap(), s);
    let world = build_world(&s, &m, base).unwrap();
    for a in s.accounts.iter().filter(|a| a.exists) {
        let address: Address = a.address.parse().unwrap();
        let actual = world.world.runtime.get_account(&address).unwrap();
        let raw = a.raw.as_ref().unwrap();
        assert_eq!(actual.owner.to_string(), raw.owner);
        assert_eq!(actual.lamports, u64::try_from(&raw.lamports).unwrap());
        assert_eq!(actual.executable, raw.executable);
        assert_eq!(actual.data.len(), raw.data_length);
        assert_eq!(actual.data, raw.raw_data);
    }
    eprintln!(
        "LIVE DEVNET APPLICATION CAPTURE: PASS {} slot {}",
        s.snapshot_id, s.provenance.slot
    );
}

#[test]
fn live_devnet_capture_only_when_explicitly_enabled() {
    if std::env::var("DG_RUN_DEVNET_TESTS").as_deref() != Ok("1") {
        eprintln!("LIVE DEVNET INTEGRATION: NOT RUN");
        return;
    }
    let (mut m, base) = project();
    let c = m.snapshot.as_mut().unwrap();
    c.min_context_slot = None;
    c.rpc_url = std::env::var("DG_DEVNET_RPC_URL").ok();
    // A builtin exists without a deployment or funding transaction.
    c.accounts = vec![SnapshotAccount {
        name: "system".into(),
        address: "11111111111111111111111111111111".into(),
        owner: Some("NativeLoader1111111111111111111111111111111".into()),
        account_type: None,
        optional: false,
    }];
    let http = HttpRpc::from_config(c).unwrap();
    let s = capture(&m, &base, &http).unwrap();
    assert!(s.provenance.slot > 0);
    assert!(s.accounts[0].exists);
    let temp = Temp::new();
    save(&s, &temp.0).unwrap();
    assert_eq!(load(&temp.0).unwrap(), s);
    eprintln!("LIVE DEVNET INTEGRATION: PASS (capture only, no application replay)");
}
