//! Explicit, bounded Devnet inputs. Connection settings are never copied to evidence.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    Devnet,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotConfig {
    pub source: SnapshotSource,
    pub rpc_url: Option<String>,
    pub rpc_url_env: Option<String>,
    pub min_context_slot: Option<u64>,
    #[serde(default = "default_limit")]
    pub max_account_bytes: usize,
    pub accounts: Vec<SnapshotAccount>,
    #[serde(default)]
    pub programs: Vec<LocalProgram>,
}
fn default_limit() -> usize {
    1024 * 1024
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotAccount {
    pub name: String,
    pub address: String,
    pub owner: Option<String>,
    pub account_type: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalProgram {
    pub program_id: String,
    pub program_path: PathBuf,
}
