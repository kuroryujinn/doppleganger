use dg_analysis::{
    model::{FieldType, ProgramModel, TypeModel},
    normalize,
};
use dg_state::*;
use num_bigint::BigInt;
use serde_json::json;
use std::collections::BTreeMap;

const ID: &str = "11111111111111111111111111111111";
fn model(ty: serde_json::Value, types: serde_json::Value) -> ProgramModel {
    let mut definitions = types.as_array().unwrap().clone();
    definitions.push(
        json!({"name":"Position","type":{"kind":"struct","fields":[{"name":"value","type":ty}]}}),
    );
    normalize(
        json!({"instructions":[], "accounts":[{"name":"Position","discriminator":[42,7]}],
            "types": definitions
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}
fn primitive(name: &str) -> ProgramModel {
    model(json!(name), json!([]))
}
fn bytes(payload: &[u8]) -> Vec<u8> {
    [vec![42, 7], payload.to_vec()].concat()
}
fn decode(m: &ProgramModel, payload: &[u8]) -> Result<DecodedAccount, StateError> {
    decode_account(
        m,
        "Position",
        &bytes(payload),
        DecodeLimits::default(),
        TrailingBytes::Reject,
    )
}
fn integer(n: impl Into<BigInt>) -> Value {
    Value::Integer(n.into())
}

#[test]
fn every_integer_width_decodes_independent_wire_bytes() {
    for bits in [8, 16, 32, 64, 128] {
        let width = bits / 8;
        let unsigned = primitive(&format!("u{bits}"));
        let signed = primitive(&format!("i{bits}"));
        assert_eq!(
            decode(&unsigned, &vec![0; width]).unwrap().fields["value"],
            integer(0)
        );
        assert_eq!(
            decode(&unsigned, &vec![255; width]).unwrap().fields["value"],
            integer((BigInt::from(1) << bits) - 1)
        );
        assert_eq!(
            decode(&signed, &vec![255; width]).unwrap().fields["value"],
            integer(-1)
        );
        let mut minimum = vec![0; width];
        minimum[width - 1] = 128;
        assert_eq!(
            decode(&signed, &minimum).unwrap().fields["value"],
            integer(-(BigInt::from(1) << (bits - 1)))
        );
        let mut maximum = vec![255; width];
        maximum[width - 1] = 127;
        assert_eq!(
            decode(&signed, &maximum).unwrap().fields["value"],
            integer((BigInt::from(1) << (bits - 1)) - 1)
        );
    }
}
#[test]
fn bool_and_public_key() {
    assert_eq!(
        decode(&primitive("bool"), &[1]).unwrap().fields["value"],
        Value::Bool(true)
    );
    assert_eq!(
        decode(&primitive("bool"), &[0]).unwrap().fields["value"],
        Value::Bool(false)
    );
    assert_eq!(
        decode(&primitive("pubkey"), &[0; 32]).unwrap().fields["value"],
        Value::Pubkey(ID.into())
    );
}
#[test]
fn strings_validate_utf8_and_byte_length() {
    assert_eq!(
        decode(&primitive("string"), &[2, 0, 0, 0, 0xc3, 0xa9])
            .unwrap()
            .fields["value"],
        Value::String("\u{e9}".into())
    );
    assert_eq!(
        decode(&primitive("string"), &[0, 0, 0, 0]).unwrap().fields["value"],
        Value::String(String::new())
    );
    assert_eq!(
        decode(&primitive("string"), &[1, 0, 0, 0, 255])
            .unwrap_err()
            .code,
        "INVALID_DATA"
    );
}
#[test]
fn vectors_and_bytes() {
    let m = model(json!({"vec":"u16"}), json!([]));
    assert_eq!(
        decode(&m, &[2, 0, 0, 0, 1, 0, 2, 0]).unwrap().fields["value"],
        Value::Sequence(vec![integer(1), integer(2)])
    );
    assert_eq!(
        decode(&m, &[0, 0, 0, 0]).unwrap().fields["value"],
        Value::Sequence(vec![])
    );
    assert_eq!(
        decode(&primitive("bytes"), &[2, 0, 0, 0, 1, 2])
            .unwrap()
            .fields["value"],
        Value::Sequence(vec![integer(1), integer(2)])
    );
}
#[test]
fn options_some_and_none() {
    let m = model(json!({"option":"u8"}), json!([]));
    assert_eq!(
        decode(&m, &[0]).unwrap().fields["value"],
        Value::Option(None)
    );
    assert_eq!(
        decode(&m, &[1, 9]).unwrap().fields["value"],
        Value::Option(Some(Box::new(integer(9))))
    );
    assert_eq!(decode(&m, &[2]).unwrap_err().code, "INVALID_DATA");
}
#[test]
fn arrays_and_tuples() {
    let m = model(json!({"array":["u8",2]}), json!([]));
    assert_eq!(
        decode(&m, &[4, 5]).unwrap().fields["value"],
        Value::Sequence(vec![integer(4), integer(5)])
    );
    let m = model(json!({"tuple":["bool","u16"]}), json!([]));
    assert_eq!(
        decode(&m, &[1, 0, 1]).unwrap().fields["value"],
        Value::Sequence(vec![Value::Bool(true), integer(256)])
    );
}
#[test]
fn nested_structs_and_aliases() {
    let m = model(
        json!({"defined":"Outer"}),
        json!([
            {"name":"Outer","type":{"kind":"struct","fields":[{"name":"inner","type":{"defined":"Inner"}}]}},
            {"name":"Inner","type":{"kind":"struct","fields":[{"name":"count","type":{"defined":"Count"}}]}},
            {"name":"Count","type":{"kind":"alias","value":"u64"}}
        ]),
    );
    assert_eq!(
        decode(&m, &9u64.to_le_bytes()).unwrap().fields["value"],
        Value::Struct(BTreeMap::from([(
            "inner".into(),
            Value::Struct(BTreeMap::from([("count".into(), integer(9))]))
        )]))
    );
}
fn enum_model() -> ProgramModel {
    model(
        json!({"defined":"Status"}),
        json!([
            {"name":"Status","type":{"kind":"enum","variants":[
                {"name":"Idle"}, {"name":"Tuple","fields":[{"option":"u8"},"u16"]},
                {"name":"Named","fields":[{"name":"amount","type":"u64"}]}
            ]}}
        ]),
    )
}
#[test]
fn enum_unit_tuple_and_named_variants() {
    let m = enum_model();
    assert_eq!(
        decode(&m, &[0]).unwrap().fields["value"],
        Value::Enum {
            variant: "Idle".into(),
            fields: BTreeMap::new()
        }
    );
    assert_eq!(
        decode(&m, &[1, 1, 7, 8, 0]).unwrap().fields["value"],
        Value::Enum {
            variant: "Tuple".into(),
            fields: BTreeMap::from([
                ("0".into(), Value::Option(Some(Box::new(integer(7))))),
                ("1".into(), integer(8))
            ])
        }
    );
    assert_eq!(
        decode(&m, &[vec![2], 9u64.to_le_bytes().to_vec()].concat())
            .unwrap()
            .fields["value"],
        Value::Enum {
            variant: "Named".into(),
            fields: BTreeMap::from([("amount".into(), integer(9))])
        }
    );
}
#[test]
fn malformed_tags_are_structured_errors() {
    let e = decode(&primitive("bool"), &[2]).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.path.as_str(), e.offset),
        ("INVALID_DATA", "Position.value", 2)
    );
    assert_eq!(
        decode(&enum_model(), &[3]).unwrap_err().code,
        "INVALID_DATA"
    );
}
#[test]
fn wrong_and_truncated_discriminators() {
    let m = primitive("u8");
    assert_eq!(
        decode_account(
            &m,
            "Position",
            &[0, 7, 1],
            DecodeLimits::default(),
            TrailingBytes::Reject
        )
        .unwrap_err()
        .code,
        "DISCRIMINATOR"
    );
    assert_eq!(
        decode_account(
            &m,
            "Position",
            &[42],
            DecodeLimits::default(),
            TrailingBytes::Reject
        )
        .unwrap_err()
        .code,
        "TRUNCATED"
    );
}
#[test]
fn every_truncation_is_rejected() {
    let m = model(json!({"tuple":["u128","string",{"vec":"u64"}]}), json!([]));
    let payload = [
        vec![255; 16],
        vec![2, 0, 0, 0, b'o', b'k', 1, 0, 0, 0],
        vec![0; 8],
    ]
    .concat();
    decode(&m, &payload).unwrap();
    for end in 0..payload.len() {
        assert!(decode(&m, &payload[..end]).is_err(), "truncation {end}");
    }
}
#[test]
fn extra_bytes_require_explicit_policy_and_are_preserved() {
    let m = primitive("u8");
    assert_eq!(decode(&m, &[1, 0, 9]).unwrap_err().code, "TRAILING_BYTES");
    let result = decode_account(
        &m,
        "Position",
        &bytes(&[1, 0, 9]),
        DecodeLimits::default(),
        TrailingBytes::Preserve,
    )
    .unwrap();
    assert_eq!(result.consumed_bytes, 3);
    assert_eq!(result.trailing_bytes, [0, 9]);
}
#[test]
fn unknown_and_unsupported_types_fail_cleanly() {
    let mut m = primitive("u8");
    m.accounts.get_mut("Position").unwrap().fields[0].ty = FieldType::Defined("Missing".into());
    assert_eq!(decode(&m, &[]).unwrap_err().code, "UNKNOWN_TYPE");
    assert_eq!(
        decode_account(
            &m,
            "Missing",
            &[],
            DecodeLimits::default(),
            TrailingBytes::Reject
        )
        .unwrap_err()
        .code,
        "UNKNOWN_TYPE"
    );
    assert_eq!(
        decode(&primitive("f32"), &[0; 4]).unwrap_err().code,
        "UNSUPPORTED"
    );
    m.accounts.get_mut("Position").unwrap().discriminator = None;
    assert_eq!(decode(&m, &[]).unwrap_err().code, "UNSUPPORTED");
}
#[test]
fn malformed_enum_models_fail_cleanly() {
    let mut m = enum_model();
    m.types
        .insert("Status".into(), TypeModel::Enum { variants: vec![] });
    assert_eq!(decode(&m, &[0]).unwrap_err().code, "SCHEMA");
}
#[test]
fn malicious_lengths_and_recursive_types_are_bounded() {
    let m = model(json!({"vec":"u8"}), json!([]));
    assert_eq!(decode(&m, &[255; 4]).unwrap_err().code, "LIMIT");
    assert_eq!(
        decode(&primitive("string"), &[255; 4]).unwrap_err().code,
        "TRUNCATED"
    );
    let mut m = primitive("u8");
    m.accounts.get_mut("Position").unwrap().fields[0].ty = FieldType::Defined("Loop".into());
    m.types.insert(
        "Loop".into(),
        TypeModel::Alias {
            value: FieldType::Defined("Loop".into()),
        },
    );
    assert_eq!(decode(&m, &[]).unwrap_err().code, "LIMIT");
    let input = toml::Value::Integer(1);
    assert!(
        dg_config::encode(&input, &FieldType::Defined("Loop".into()), &m.types, "loop")
            .unwrap_err()
            .message
            .contains("depth")
    );
}
#[test]
fn account_and_total_value_limits() {
    let m = primitive("u8");
    let limits = DecodeLimits {
        max_bytes: 2,
        ..DecodeLimits::default()
    };
    assert_eq!(
        decode_account(&m, "Position", &bytes(&[1]), limits, TrailingBytes::Reject)
            .unwrap_err()
            .code,
        "LIMIT"
    );
    let limits = DecodeLimits {
        max_values: 0,
        ..DecodeLimits::default()
    };
    assert_eq!(
        decode_account(&m, "Position", &bytes(&[1]), limits, TrailingBytes::Reject)
            .unwrap_err()
            .code,
        "LIMIT"
    );
}
fn fixture() -> FixtureDefinition {
    toml::from_str(&format!(
        r#"
name = "positions"
[[account]]
name = "alice_position"
type = "Position"
address = "$alice"
owner = "{ID}"
lamports = "u64:18446744073709551615"
[account.fields]
value = "u128:340282366920938463463374607431768211455"
"#
    ))
    .unwrap()
}
#[test]
fn phase_acceptance_fixture_snapshot_and_lossless_json_roundtrip() {
    let m = primitive("u128");
    let world = load_fixture(
        &fixture(),
        &m,
        &BTreeMap::from([("alice".into(), ID.into())]),
        DecodeLimits::default(),
    )
    .unwrap();
    let snapshot = &world.accounts["alice_position"];
    assert_eq!(snapshot.raw_data, bytes(&[255; 16]));
    assert_eq!(snapshot.data_length, 18);
    assert_eq!(snapshot.lamports, BigInt::from(u64::MAX));
    assert_eq!(snapshot.decoded.fields["value"], integer(u128::MAX));
    let encoded = serde_json::to_string(&world).unwrap();
    assert!(encoded.contains("\"340282366920938463463374607431768211455\""));
    let restored: RuntimeWorld = serde_json::from_str(&encoded).unwrap();
    assert_eq!(world, restored);
    restored.accounts["alice_position"]
        .validate(&m, DecodeLimits::default())
        .unwrap();
    assert_eq!(encoded, serde_json::to_string(&restored).unwrap());
}
#[test]
fn tampered_snapshot_is_rejected() {
    let m = primitive("u8");
    let mut s = AccountSnapshot::capture(
        AccountBinding {
            name: "position".into(),
            address: ID.into(),
            owner: ID.into(),
            account_type: "Position".into(),
        },
        0,
        false,
        bytes(&[1]),
        &m,
        DecodeLimits::default(),
        TrailingBytes::Reject,
    )
    .unwrap();
    s.raw_data[2] = 2;
    assert_eq!(
        s.validate(&m, DecodeLimits::default()).unwrap_err().code,
        "SNAPSHOT"
    );
    s.raw_data[2] = 1;
    s.schema_version = 2;
    assert!(s.validate(&m, DecodeLimits::default()).is_err());
    s.schema_version = 1;
    s.data_length = 99;
    assert!(s.validate(&m, DecodeLimits::default()).is_err());
}
#[test]
fn fixture_rejects_missing_bindings_fields_and_duplicates() {
    let m = primitive("u128");
    let bindings = BTreeMap::from([("alice".into(), ID.into())]);
    let mut f = fixture();
    assert!(load_fixture(&f, &m, &BTreeMap::new(), DecodeLimits::default()).is_err());
    f.account.push(f.account[0].clone());
    assert!(load_fixture(&f, &m, &bindings, DecodeLimits::default())
        .unwrap_err()
        .message
        .contains("duplicate account name"));
    f.account[1].name = "other".into();
    assert!(load_fixture(&f, &m, &bindings, DecodeLimits::default())
        .unwrap_err()
        .message
        .contains("duplicate account address"));
    f.account.pop();
    f.account[0].fields.as_mut().unwrap().clear();
    assert!(load_fixture(&f, &m, &bindings, DecodeLimits::default()).is_err());
}
#[test]
fn typed_numeric_boundaries_encode_without_toml_i64_loss() {
    for (ty, n) in [
        ("u64", u64::MAX.to_string()),
        ("u128", u128::MAX.to_string()),
        ("i128", i128::MIN.to_string()),
        ("i128", i128::MAX.to_string()),
    ] {
        let input = toml::Value::String(format!("{ty}:{n}"));
        let encoded = dg_config::encode(
            &input,
            &FieldType::Primitive(ty.into()),
            &BTreeMap::new(),
            "value",
        )
        .unwrap();
        assert_eq!(
            decode(&primitive(ty), &encoded).unwrap().fields["value"],
            Value::Integer(n.parse().unwrap())
        );
    }
    for (ty, n) in [
        ("u64", "18446744073709551616"),
        ("u128", "340282366920938463463374607431768211456"),
        ("i128", "-170141183460469231731687303715884105729"),
        ("u64", "-1"),
        ("u8", "u16:1"),
    ] {
        assert!(dg_config::encode(
            &toml::Value::String(n.into()),
            &FieldType::Primitive(ty.into()),
            &BTreeMap::new(),
            "value"
        )
        .is_err());
    }
}

#[derive(borsh::BorshSerialize)]
struct BorshRecord {
    active: bool,
    amount: u128,
    label: String,
    samples: Vec<u16>,
    optional: Option<i128>,
    fixed: [u8; 2],
    pair: (bool, u8),
    status: BorshStatus,
}
#[derive(borsh::BorshSerialize)]
enum BorshStatus {
    Done { count: u64 },
}

#[test]
fn complex_fixture_matches_independent_borsh_serializer() {
    let m = model(
        json!({"defined":"Record"}),
        json!([
            {"name":"Record","type":{"kind":"struct","fields":[
                {"name":"active","type":"bool"}, {"name":"amount","type":"u128"},
                {"name":"label","type":"string"}, {"name":"samples","type":{"vec":"u16"}},
                {"name":"optional","type":{"option":"i128"}}, {"name":"fixed","type":{"array":["u8",2]}},
                {"name":"pair","type":{"tuple":["bool","u8"]}}, {"name":"status","type":{"defined":"Status"}}
            ]}},
            {"name":"Status","type":{"kind":"enum","variants":[{"name":"Done","fields":[{"name":"count","type":"u64"}]}]}}
        ]),
    );
    let f: FixtureDefinition = toml::from_str(&format!(
        r#"
name="records"
[[account]]
name="record"
type="Position"
address="{ID}"
owner="{ID}"
lamports=1
[account.fields.value]
active=true
amount="u128:340282366920938463463374607431768211455"
label="sample"
samples=[0,65535]
optional={{some="i128:-170141183460469231731687303715884105728"}}
fixed=[7,8]
pair=[false,9]
status={{variant="Done",fields={{count="u64:18446744073709551615"}}}}
"#
    ))
    .unwrap();
    let world = load_fixture(&f, &m, &BTreeMap::new(), DecodeLimits::default()).unwrap();
    let independently_encoded = borsh::to_vec(&BorshRecord {
        active: true,
        amount: u128::MAX,
        label: "sample".into(),
        samples: vec![0, u16::MAX],
        optional: Some(i128::MIN),
        fixed: [7, 8],
        pair: (false, 9),
        status: BorshStatus::Done { count: u64::MAX },
    })
    .unwrap();
    assert_eq!(
        world.accounts["record"].raw_data,
        bytes(&independently_encoded)
    );
    world.accounts["record"]
        .validate(&m, DecodeLimits::default())
        .unwrap();
}

#[test]
fn arbitrary_byte_corpus_never_panics() {
    let models = [
        primitive("string"),
        enum_model(),
        model(json!({"vec":{"option":"u128"}}), json!([])),
    ];
    let mut seed = 7u32;
    for len in 0..128 {
        let payload: Vec<_> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        for m in &models {
            let _ = decode(m, &payload);
        }
    }
}

#[test]
fn nested_enum_payloads_decode_recursively() {
    let m = model(
        json!({"defined":"Outer"}),
        json!([
            {"name":"Outer","type":{"kind":"enum","variants":[{"name":"Wrapped","fields":[{"defined":"Inner"}]}]}},
            {"name":"Inner","type":{"kind":"enum","variants":[{"name":"Empty"},{"name":"Value","fields":["u64"]}]}}
        ]),
    );
    assert_eq!(
        decode(&m, &[vec![0, 1], u64::MAX.to_le_bytes().to_vec()].concat())
            .unwrap()
            .fields["value"],
        Value::Enum {
            variant: "Wrapped".into(),
            fields: BTreeMap::from([(
                "0".into(),
                Value::Enum {
                    variant: "Value".into(),
                    fields: BTreeMap::from([("0".into(), integer(u64::MAX))])
                }
            )])
        }
    );
}
