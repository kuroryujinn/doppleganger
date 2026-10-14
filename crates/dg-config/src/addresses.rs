//! Resolve addresses once per local test world; never serialize signing secrets.
use crate::{encode, fail, Result};
use dg_analysis::model::FieldType;
use serde::Deserialize;
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AddressDefinition {
    Literal {
        address: String,
    },
    Signer {
        #[serde(default)]
        test_seed: Option<[u8; 32]>,
        lamports: toml::Value,
    },
    Reference {
        account: String,
    },
    Program {
        version: String,
    },
    Pda {
        program: String,
        seeds: Vec<SeedDefinition>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum SeedDefinition {
    Const(ConstSeed),
    Pubkey(PubkeySeed),
    Integer(IntegerSeed),
    Bytes(BytesSeed),
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstSeed {
    pub r#const: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PubkeySeed {
    pub pubkey: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegerSeed {
    pub integer: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BytesSeed {
    pub bytes: Vec<u8>,
}

pub struct ResolvedAddresses {
    pub addresses: BTreeMap<String, Address>,
    pub signers: BTreeMap<String, Keypair>,
    pub funding: BTreeMap<String, u64>,
    pub bumps: BTreeMap<String, u8>,
}
impl ResolvedAddresses {
    pub fn strings(&self) -> BTreeMap<String, String> {
        self.addresses
            .iter()
            .map(|(name, address)| (name.clone(), address.to_string()))
            .collect()
    }
    pub fn resolve(
        definitions: &BTreeMap<String, AddressDefinition>,
        programs: &BTreeMap<String, Address>,
    ) -> Result<Self> {
        let mut result = Self {
            addresses: BTreeMap::new(),
            signers: BTreeMap::new(),
            funding: BTreeMap::new(),
            bumps: BTreeMap::new(),
        };
        for name in definitions.keys() {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(fail(
                    "addresses",
                    "names must use letters, digits, '_' or '-'",
                ));
            }
        }
        for name in definitions.keys() {
            result.visit(name, definitions, programs, &mut BTreeSet::new())?;
        }
        Ok(result)
    }
    fn source(
        &mut self,
        source: &str,
        definitions: &BTreeMap<String, AddressDefinition>,
        programs: &BTreeMap<String, Address>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<Address> {
        if let Some(name) = source.strip_prefix('$') {
            self.visit(name, definitions, programs, visiting)
        } else {
            Address::from_str(source).map_err(|e| fail("addresses", e.to_string()))
        }
    }
    fn visit(
        &mut self,
        name: &str,
        definitions: &BTreeMap<String, AddressDefinition>,
        programs: &BTreeMap<String, Address>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<Address> {
        if let Some(address) = self.addresses.get(name) {
            return Ok(*address);
        }
        let path = format!("addresses.{name}");
        if visiting.len() >= 64 || !visiting.insert(name.into()) {
            return Err(fail(
                &path,
                "cyclic address reference or maximum depth exceeded",
            ));
        }
        let definition = definitions
            .get(name)
            .ok_or_else(|| fail(&path, "unknown address reference"))?;
        let address = match definition {
            AddressDefinition::Literal { address } => {
                Address::from_str(address).map_err(|e| fail(&path, e.to_string()))?
            }
            AddressDefinition::Signer {
                test_seed,
                lamports,
            } => {
                let amount = encode(
                    lamports,
                    &FieldType::Primitive("u64".into()),
                    &BTreeMap::new(),
                    &format!("{path}.lamports"),
                )?;
                let mut bytes = [0; 8];
                bytes.copy_from_slice(&amount);
                let keypair = match test_seed {
                    Some(seed) => Keypair::new_from_array(*seed),
                    None => Keypair::new(),
                };
                let address = keypair.pubkey();
                if self.signers.values().any(|key| key.pubkey() == address) {
                    return Err(fail(
                        &path,
                        "duplicate signer public key; use a reference alias instead",
                    ));
                }
                self.signers.insert(name.into(), keypair);
                self.funding.insert(name.into(), u64::from_le_bytes(bytes));
                address
            }
            AddressDefinition::Reference { account } => {
                self.source(account, definitions, programs, visiting)?
            }
            AddressDefinition::Program { version } => *programs
                .get(version)
                .ok_or_else(|| fail(&path, "unknown program version"))?,
            AddressDefinition::Pda { program, seeds } => {
                let program_id = if let Some(id) = programs.get(program) {
                    *id
                } else {
                    self.source(program, definitions, programs, visiting)?
                };
                // The SDK appends one bump seed; reserve its slot before calling it.
                if seeds.len() > 15 {
                    return Err(fail(&path, "PDA supports at most 15 explicit seeds"));
                }
                let mut raw = Vec::new();
                for seed in seeds {
                    let bytes = match seed {
                        SeedDefinition::Const(seed) => seed.r#const.as_bytes().to_vec(),
                        SeedDefinition::Bytes(seed) => seed.bytes.clone(),
                        SeedDefinition::Pubkey(seed) => self
                            .source(&seed.pubkey, definitions, programs, visiting)?
                            .as_ref()
                            .to_vec(),
                        SeedDefinition::Integer(seed) => {
                            let (ty, _) = seed.integer.split_once(':').ok_or_else(|| {
                                fail(&path, "integer seed requires a typed value such as u64:42")
                            })?;
                            if !matches!(
                                ty,
                                "u8" | "u16"
                                    | "u32"
                                    | "u64"
                                    | "u128"
                                    | "i8"
                                    | "i16"
                                    | "i32"
                                    | "i64"
                                    | "i128"
                            ) {
                                return Err(fail(&path, "unsupported integer seed type"));
                            }
                            encode(
                                &toml::Value::String(seed.integer.clone()),
                                &FieldType::Primitive(ty.into()),
                                &BTreeMap::new(),
                                &path,
                            )?
                        }
                    };
                    if bytes.len() > 32 {
                        return Err(fail(&path, "each PDA seed must be at most 32 bytes"));
                    }
                    raw.push(bytes);
                }
                let borrowed: Vec<_> = raw.iter().map(Vec::as_slice).collect();
                let (address, bump) = Address::try_find_program_address(&borrowed, &program_id)
                    .ok_or_else(|| fail(&path, "unable to derive PDA"))?;
                self.bumps.insert(name.into(), bump);
                address
            }
        };
        visiting.remove(name);
        self.addresses.insert(name.into(), address);
        Ok(address)
    }
}
