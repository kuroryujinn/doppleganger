use dg_config::{addresses::*, FixtureDefinition};
use solana_address::Address;
use solana_signer::Signer;
use std::{collections::BTreeMap, str::FromStr};
const ID: &str = "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS";
const SYSTEM: &str = "11111111111111111111111111111111";
fn programs() -> BTreeMap<String, Address> {
    BTreeMap::from([
        ("v1".into(), Address::from_str(ID).unwrap()),
        ("v2".into(), Address::from_str(SYSTEM).unwrap()),
    ])
}
fn definitions(s: &str) -> BTreeMap<String, AddressDefinition> {
    toml::from_str(s).unwrap()
}
fn resolve(s: &str) -> ResolvedAddresses {
    ResolvedAddresses::resolve(&definitions(s), &programs()).unwrap()
}
#[test]
fn literal_program_and_reference_addresses() {
    let r=resolve(&format!("[a]\ntype='reference'\naccount='$z'\n[z]\ntype='literal'\naddress='{SYSTEM}'\n[program]\ntype='program'\nversion='v1'"));
    assert_eq!(r.addresses["a"], r.addresses["z"]);
    assert_eq!(r.addresses["program"], programs()["v1"]);
}
#[test]
fn generated_signer_and_alias_share_one_keypair() {
    let r=resolve("[alice]\ntype='signer'\nlamports='u64:18446744073709551615'\n[authority]\ntype='reference'\naccount='$alice'");
    assert_eq!(r.addresses["alice"], r.signers["alice"].pubkey());
    assert_eq!(r.addresses["alice"], r.addresses["authority"]);
    assert_eq!(r.signers.len(), 1);
    assert_eq!(r.funding["alice"], u64::MAX);
}
#[test]
fn deterministic_test_signer_and_stable_logical_mapping() {
    let mut defs = BTreeMap::new();
    defs.insert(
        "alice".into(),
        AddressDefinition::Signer {
            test_seed: Some([7; 32]),
            lamports: toml::Value::Integer(5),
        },
    );
    let a = ResolvedAddresses::resolve(&defs, &programs()).unwrap();
    let b = ResolvedAddresses::resolve(&defs, &programs()).unwrap();
    assert_eq!(a.addresses, b.addresses);
    defs.insert("duplicate".into(), defs["alice"].clone());
    assert!(ResolvedAddresses::resolve(&defs, &programs()).is_err());
}
#[test]
fn constant_pubkey_integer_and_raw_pda_seeds_match_sdk() {
    let r=resolve(&format!("[alice]\ntype='literal'\naddress='{SYSTEM}'\n[position]\ntype='pda'\nprogram='v1'\nseeds=[{{const='position'}},{{pubkey='$alice'}},{{integer='u64:42'}},{{bytes=[1,2]}}]"));
    let (expected, bump) = Address::find_program_address(
        &[
            b"position",
            r.addresses["alice"].as_ref(),
            &42u64.to_le_bytes(),
            &[1, 2],
        ],
        &programs()["v1"],
    );
    assert_eq!(r.addresses["position"], expected);
    assert_eq!(r.bumps["position"], bump);
}
#[test]
fn different_program_ids_and_seed_order_produce_distinct_pdas() {
    let r=resolve("[a]\ntype='pda'\nprogram='v1'\nseeds=[{const='a'},{const='b'}]\n[b]\ntype='pda'\nprogram='v2'\nseeds=[{const='a'},{const='b'}]\n[c]\ntype='pda'\nprogram='v1'\nseeds=[{const='b'},{const='a'}]");
    assert_ne!(r.addresses["a"], r.addresses["b"]);
    assert_ne!(r.addresses["a"], r.addresses["c"]);
}
#[test]
fn bad_seeds_and_unknown_references_fail_without_panics() {
    for source in [
        "[a]\ntype='reference'\naccount='$missing'".to_string(),
        "[a]\ntype='reference'\naccount='$b'\n[b]\ntype='reference'\naccount='$a'".to_string(),
        "[a]\ntype='program'\nversion='v3'".to_string(),
        "[a]\ntype='literal'\naddress='invalid'".to_string(),
        format!(
            "[a]\ntype='pda'\nprogram='v1'\nseeds=[{{const='{}'}}]",
            "x".repeat(33)
        ),
        format!(
            "[a]\ntype='pda'\nprogram='v1'\nseeds=[{}]",
            vec!["{const='x'}"; 16].join(",")
        ),
        "[a]\ntype='pda'\nprogram='v1'\nseeds=[{integer='42'}]".to_string(),
        "[a]\ntype='pda'\nprogram='v1'\nseeds=[{integer='u8:256'}]".to_string(),
    ] {
        assert!(
            ResolvedAddresses::resolve(&definitions(&source), &programs()).is_err(),
            "{source}"
        );
    }
    assert!(toml::from_str::<BTreeMap<String, AddressDefinition>>(
        "[a]\ntype='pda'\nprogram='v1'\nseeds=[{const='x',pubkey='$a'}]"
    )
    .is_err());
}
#[test]
fn fixture_actions_use_generic_builder_and_preserve_order() {
    let model=dg_analysis::normalize(serde_json::json!({"instructions":[
        {"name":"initialize","discriminator":[1],"accounts":[{"name":"authority","signer":true}],"args":[{"name":"owner","type":"pubkey"}]},
        {"name":"add","discriminator":[2],"accounts":[{"name":"authority","signer":true}],"args":[{"name":"amount","type":"u64"}]}
    ]}).to_string().as_bytes()).unwrap();
    let fixture:FixtureDefinition=toml::from_str("name='ledger'\n[[initialize]]\ninstruction='initialize'\naccount_mapping={authority='alice'}\nargs={owner='$alice'}\n[[setup]]\ninstruction='add'\naccount_mapping={authority='alice'}\nargs={amount='u64:18446744073709551615'}\n[[setup]]\ninstruction='add'\naccount_mapping={authority='alice'}\nargs={amount=0}").unwrap();
    let addresses = resolve("[alice]\ntype='signer'\nlamports=100");
    let plan = fixture.instructions(&model, ID, &addresses).unwrap();
    assert_eq!(
        plan.initialize[0].data,
        [vec![1], addresses.addresses["alice"].as_ref().to_vec()].concat()
    );
    assert_eq!(
        plan.setup[0].data,
        [vec![2], u64::MAX.to_le_bytes().to_vec()].concat()
    );
    assert_eq!(plan.setup[1].data, [2, 0, 0, 0, 0, 0, 0, 0, 0]);
    let missing_signer = resolve(&format!("[alice]\ntype='literal'\naddress='{SYSTEM}'"));
    assert!(fixture.instructions(&model, ID, &missing_signer).is_err());
}

#[test]
fn invalid_fixture_accounts_and_actions_fail_before_execution() {
    let model=dg_analysis::normalize(serde_json::json!({"instructions":[
        {"name":"create","discriminator":[1],"accounts":[{"name":"owner","signer":true}],"args":[{"name":"amount","type":"u8"}]}
    ],"accounts":[{"name":"Record","discriminator":[2],"type":{"kind":"struct","fields":[{"name":"amount","type":"u8"}]}}]}).to_string().as_bytes()).unwrap();
    let base=format!("name='records'\n[[account]]\nname='record'\ntype='Record'\naddress='{ID}'\nowner='{ID}'\nlamports=0\n[[initialize]]\ninstruction='create'\naccount_mapping={{owner='alice'}}\nargs={{amount=1}}");
    let addresses = resolve("[alice]\ntype='signer'\nlamports=100");
    let fixture: FixtureDefinition = toml::from_str(&base).unwrap();
    fixture.instructions(&model, ID, &addresses).unwrap();
    for source in [
        base.replace("type='Record'", "type='Unknown'"),
        base.replace("name='record'", "name=''"),
        base.replace("amount=1", "amount=256"),
        base.replace("instruction='create'", "instruction='missing'"),
        base.replace("owner='alice'", "owner='missing'"),
        base.replace("lamports=0", "lamports=-1"),
    ] {
        let fixture: FixtureDefinition = toml::from_str(&source).unwrap();
        assert!(
            fixture.instructions(&model, ID, &addresses).is_err(),
            "{source}"
        );
    }
    let mut duplicate = fixture.clone();
    duplicate.account.push(duplicate.account[0].clone());
    assert!(duplicate.instructions(&model, ID, &addresses).is_err());
    let mut no_setup = fixture;
    no_setup.initialize.clear();
    assert!(no_setup.instructions(&model, ID, &addresses).is_err());
}

#[test]
fn address_reference_depth_is_bounded() {
    let mut defs = BTreeMap::new();
    for i in 0..70 {
        defs.insert(
            format!("n{i:03}"),
            AddressDefinition::Reference {
                account: format!("$n{:03}", i + 1),
            },
        );
    }
    defs.insert(
        "n070".into(),
        AddressDefinition::Literal {
            address: SYSTEM.into(),
        },
    );
    assert!(ResolvedAddresses::resolve(&defs, &programs()).is_err());
}
