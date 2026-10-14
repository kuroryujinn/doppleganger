use dg_analysis::normalize;
use serde_json::{json, Value};
#[test]
fn real_generated_idls_from_two_anchor_releases_normalize() {
    for bytes in [
        include_bytes!("../../../tests/compatibility/anchor-0.30.1/external.json").as_slice(),
        include_bytes!("../../../tests/compatibility/anchor-0.30.1/external-legacy.json")
            .as_slice(),
        include_bytes!("../../../tests/compatibility/anchor-1.2.0/escrow-v1.json").as_slice(),
        include_bytes!("../../../tests/compatibility/anchor-1.2.0/escrow-v2.json").as_slice(),
    ] {
        let model = normalize(bytes).unwrap();
        assert!(!model.accounts.is_empty());
        assert!(!model.instructions.is_empty());
    }
}
#[test]
fn unsupported_zero_copy_and_repr_fail_closed() {
    for extra in [
        json!({"serialization":"bytemuck"}),
        json!({"repr":{"kind":"c"}}),
    ] {
        let mut ty = json!({"name":"State","type":{"kind":"struct","fields":[]}});
        ty.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let idl = json!({"instructions":[],"types":[ty]});
        assert!(normalize(idl.to_string().as_bytes())
            .unwrap_err()
            .message
            .contains("Borsh"));
    }
}
#[test]
fn recursive_and_exponential_type_bombs_stop_with_structured_error() {
    let mut types = vec![json!({"name":"T0","type":{"kind":"struct","fields":[]}})];
    for n in 1..40 {
        types.push(json!({"name":format!("T{n}"),"type":{"kind":"struct","fields":[{"name":"left","type":{"defined":format!("T{}",n-1)}},{"name":"right","type":{"defined":format!("T{}",n-1)}}]}}));
    }
    let error = normalize(
        json!({"instructions":[],"types":types})
            .to_string()
            .as_bytes(),
    )
    .unwrap_err();
    assert!(error.message.contains("work limit"));
    let recursive = json!({"instructions":[],"types":[{"name":"Loop","type":{"kind":"alias","value":{"defined":"Loop"}}}]});
    assert!(normalize(recursive.to_string().as_bytes())
        .unwrap_err()
        .message
        .contains("cyclic"));
}
#[test]
fn excessive_size_nodes_and_depth_are_rejected() {
    assert!(normalize(&vec![b' '; 8 * 1024 * 1024 + 1])
        .unwrap_err()
        .message
        .contains("MiB"));
    let large = json!({"instructions":[],"docs":vec![0;100_001]});
    assert!(normalize(large.to_string().as_bytes())
        .unwrap_err()
        .message
        .contains("limit"));
    let mut value = Value::Null;
    for _ in 0..70 {
        value = json!([value]);
    }
    assert!(normalize(
        json!({"instructions":[],"docs":value})
            .to_string()
            .as_bytes()
    )
    .is_err());
}
#[test]
fn seeded_parser_mutations_do_not_panic() {
    let seed = include_bytes!("../../../tests/compatibility/anchor-0.30.1/external.json");
    let mut state = 0x5eed_u64;
    for iteration in 0..2_000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = seed.to_vec();
        let index = state as usize % bytes.len();
        if iteration % 3 == 0 {
            bytes.truncate(index);
        } else {
            bytes[index] = (state >> 32) as u8;
        }
        assert!(
            std::panic::catch_unwind(|| normalize(&bytes)).is_ok(),
            "mutation {iteration}"
        );
    }
}
#[test]
fn direct_size_estimates_of_recursive_models_are_bounded() {
    use dg_analysis::{
        model::{FieldType, TypeModel},
        size::{estimate_type, SizeKind},
    };
    let types = std::collections::BTreeMap::from([(
        "Loop".into(),
        TypeModel::Alias {
            value: FieldType::Defined("Loop".into()),
        },
    )]);
    assert_eq!(
        estimate_type(&FieldType::Defined("Loop".into()), &types).kind,
        SizeKind::Unknown
    );
}
