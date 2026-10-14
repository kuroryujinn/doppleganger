use dg_analysis::{model::FieldType, normalize};
use dg_state::{decode_account, decode_value, DecodeLimits, TrailingBytes, Value};
#[test]
fn anchor_0301_wire_bytes_decode_and_legacy_discriminators_are_explicitly_unsupported() {
    let modern = normalize(include_bytes!(
        "../../../tests/compatibility/anchor-0.30.1/external.json"
    ))
    .unwrap();
    let mut data = vec![246, 28, 6, 87, 251, 45, 50, 42];
    data.extend(u32::MAX.to_le_bytes());
    let account = decode_account(
        &modern,
        "MyAccount",
        &data,
        DecodeLimits::default(),
        TrailingBytes::Reject,
    )
    .unwrap();
    assert_eq!(account.fields["field"], Value::Integer(u32::MAX.into()));
    let legacy = normalize(include_bytes!(
        "../../../tests/compatibility/anchor-0.30.1/external-legacy.json"
    ))
    .unwrap();
    assert_eq!(
        decode_account(
            &legacy,
            "MyAccount",
            &data,
            DecodeLimits::default(),
            TrailingBytes::Reject
        )
        .unwrap_err()
        .code,
        "UNSUPPORTED"
    );
}
#[test]
fn seeded_borsh_mutations_and_all_truncations_do_not_panic() {
    let model = normalize(include_bytes!(
        "../../../tests/compatibility/anchor-1.2.0/escrow-v1.json"
    ))
    .unwrap();
    let ty = FieldType::Vec(Box::new(FieldType::Option(Box::new(FieldType::Primitive(
        "string".into(),
    )))));
    let mut seed = 42u64;
    for n in 0..5_000 {
        let mut data = Vec::new();
        for _ in 0..n % 128 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            data.push((seed >> 32) as u8);
        }
        assert!(std::panic::catch_unwind(|| decode_value(
            &model,
            &ty,
            &data,
            DecodeLimits::default()
        ))
        .is_ok());
    }
}
