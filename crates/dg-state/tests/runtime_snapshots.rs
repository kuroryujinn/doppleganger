use dg_state::{
    raw::{RawAccountSnapshot, SnapshotSet},
    *,
};
use num_bigint::BigInt;
use std::collections::BTreeMap;
const ID: &str = "11111111111111111111111111111111";
const OTHER: &str = "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS";
fn capture(data: Vec<u8>, lamports: u64) -> RawAccountSnapshot {
    RawAccountSnapshot::capture(
        ID.into(),
        OTHER.into(),
        lamports,
        false,
        data,
        DecodeLimits::default(),
    )
    .unwrap()
}
#[test]
fn arbitrary_bytes_and_zero_lamports_need_no_idl() {
    let s = capture(vec![255, 0, 1, 23], 0);
    assert_eq!(s.raw_data, [255, 0, 1, 23]);
    assert_eq!(s.lamports, BigInt::from(0));
    assert_eq!(s.data_length, 4);
}
#[test]
fn large_raw_account_and_byte_limit() {
    let s = capture(vec![255; 1024 * 1024], u64::MAX);
    s.validate(DecodeLimits::default()).unwrap();
    assert_eq!(
        s.validate(DecodeLimits {
            max_bytes: 1024,
            ..DecodeLimits::default()
        })
        .unwrap_err()
        .code,
        "LIMIT"
    );
}
#[test]
fn raw_snapshot_serialization_roundtrip() {
    let mut s = capture(vec![0, 255, 0], u64::MAX);
    s.metadata.insert("slot".into(), "123".into());
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"18446744073709551615\""));
    let restored: RawAccountSnapshot = serde_json::from_str(&json).unwrap();
    restored.validate(DecodeLimits::default()).unwrap();
    assert_eq!(s, restored);
}
#[test]
fn multiple_accounts_missing_account_and_wrong_owner() {
    let first = capture(vec![1], 0);
    let mut second = capture(vec![2], 100);
    second.address = OTHER.into();
    let set = SnapshotSet::capture([first, second], DecodeLimits::default()).unwrap();
    assert_eq!(set.accounts.len(), 2);
    assert_eq!(set.account(ID, OTHER).unwrap().raw_data, [1]);
    assert_eq!(set.account(ID, ID).unwrap_err().code, "OWNER_MISMATCH");
    assert_eq!(
        set.account("missing", ID).unwrap_err().code,
        "MISSING_ACCOUNT"
    );
}
#[test]
fn restore_replaces_state_and_equality_detects_changes() {
    let original = SnapshotSet::capture([capture(vec![1], 1)], DecodeLimits::default()).unwrap();
    let mut target = SnapshotSet::capture([capture(vec![2], 2)], DecodeLimits::default()).unwrap();
    assert_ne!(original, target);
    original
        .restore_into(&mut target, DecodeLimits::default())
        .unwrap();
    assert_eq!(original, target);
}
#[test]
fn malformed_snapshot_restore_is_atomic() {
    let mut original =
        SnapshotSet::capture([capture(vec![1], 1)], DecodeLimits::default()).unwrap();
    let mut target = SnapshotSet::default();
    original.accounts.get_mut(ID).unwrap().data_length = 99;
    assert!(original
        .restore_into(&mut target, DecodeLimits::default())
        .is_err());
    assert_eq!(target, SnapshotSet::default());
    let record = original.accounts.remove(ID).unwrap();
    original.accounts.insert(OTHER.into(), record);
    assert!(original
        .restore_into(&mut target, DecodeLimits::default())
        .is_err());
}
#[test]
fn malformed_raw_metadata_and_duplicates_fail() {
    let s = capture(vec![], 0);
    assert!(SnapshotSet::capture([s.clone(), s.clone()], DecodeLimits::default()).is_err());
    for (version, length, amount) in [
        (2, 0, BigInt::from(0)),
        (1, 3, BigInt::from(0)),
        (1, 0, BigInt::from(-1)),
        (1, 0, BigInt::from(u64::MAX) + 1),
    ] {
        let mut bad = s.clone();
        bad.schema_version = version;
        bad.data_length = length;
        bad.lamports = amount;
        assert!(bad.validate(DecodeLimits::default()).is_err());
    }
}
#[test]
fn decoding_checks_observed_owner_and_preserves_raw_failure_evidence() {
    let model=dg_analysis::normalize(br#"{"instructions":[],"accounts":[{"name":"Record","discriminator":[1],"type":{"kind":"struct","fields":[{"name":"amount","type":"u8"}]}}]}"#).unwrap();
    let s = capture(vec![1, 9], 0);
    let mut binding = AccountBinding {
        name: "record".into(),
        address: ID.into(),
        owner: OTHER.into(),
        account_type: "Record".into(),
    };
    let decoded = s
        .decode(
            binding.clone(),
            &model,
            DecodeLimits::default(),
            TrailingBytes::Reject,
        )
        .unwrap();
    assert_eq!(decoded.raw_data, s.raw_data);
    assert_eq!(
        decoded.decoded.fields,
        BTreeMap::from([("amount".into(), Value::Integer(9.into()))])
    );
    binding.owner = ID.into();
    assert!(s
        .decode(
            binding,
            &model,
            DecodeLimits::default(),
            TrailingBytes::Reject
        )
        .is_err());
    let malformed = capture(vec![255], 0);
    assert_eq!(malformed.raw_data, [255]);
}
