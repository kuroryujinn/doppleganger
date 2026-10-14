use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramModel {
    pub name: String,
    pub version: String,
    pub address: Option<String>,
    pub instructions: BTreeMap<String, InstructionModel>,
    pub accounts: BTreeMap<String, AccountSchemaModel>,
    pub types: BTreeMap<String, TypeModel>,
    pub events: BTreeMap<String, EventModel>,
    pub errors: BTreeMap<String, ErrorModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionModel {
    pub name: String,
    pub discriminator: Option<Vec<u8>>,
    pub accounts: Vec<InstructionAccountModel>,
    pub args: Vec<FieldModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionAccountModel {
    pub name: String,
    pub signer: bool,
    pub writable: bool,
    pub optional: bool,
    pub pda: Option<PdaModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdaModel {
    pub seeds: Vec<SeedModel>,
    pub program: Option<SeedModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedModel {
    pub kind: String,
    pub value: Option<Vec<u8>>,
    pub path: Option<String>,
    pub ty: Option<FieldType>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSchemaModel {
    pub name: String,
    pub discriminator: Option<Vec<u8>>,
    pub fields: Vec<FieldModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldModel {
    pub name: String,
    pub ty: FieldType,
}

pub type ArgumentModel = FieldModel;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FieldType {
    Primitive(String),
    Defined(String),
    Vec(Box<FieldType>),
    Option(Box<FieldType>),
    Array(Box<FieldType>, usize),
    Tuple(Vec<FieldType>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeModel {
    Struct { fields: Vec<FieldModel> },
    Enum { variants: Vec<EnumVariantModel> },
    Alias { value: FieldType },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumVariantModel {
    pub name: String,
    pub fields: Vec<FieldModel>,
}

pub type EnumModel = Vec<EnumVariantModel>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventModel {
    pub name: String,
    pub discriminator: Option<Vec<u8>>,
    pub fields: Vec<FieldModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorModel {
    pub name: String,
    pub code: u64,
    pub message: Option<String>,
}
