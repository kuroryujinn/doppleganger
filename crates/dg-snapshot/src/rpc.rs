//! The transport exposes only cluster identity and one explicit account batch read.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, Url};
use serde_json::{json, Value};
use std::time::Duration;

pub struct AccountBatch {
    pub slot: u64,
    pub accounts: Vec<Value>,
}
pub trait ReadOnlyRpc {
    fn genesis_hash(&self) -> Result<String>;
    fn multiple_accounts(
        &self,
        addresses: &[String],
        min_context_slot: Option<u64>,
    ) -> Result<AccountBatch>;
    fn origin(&self) -> String;
    fn acquisition(&self) -> Acquisition;
}

enum ReadMethod {
    Genesis,
    Accounts,
}
pub(crate) fn redacted_origin(endpoint: &str) -> Result<String> {
    let url =
        Url::parse(endpoint).map_err(|_| err("CONFIG", "rpc_origin", "invalid RPC origin"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(err("CONFIG", "rpc_origin", "expected HTTP(S) origin"));
    }
    Ok(url.origin().ascii_serialization())
}
impl ReadMethod {
    fn name(&self) -> &'static str {
        match self {
            Self::Genesis => "getGenesisHash",
            Self::Accounts => "getMultipleAccounts",
        }
    }
}
/// The public adapter has no transaction or administration methods.
/// ```compile_fail
/// fn writes(rpc: &dg_snapshot::rpc::HttpRpc) {
///     rpc.send_transaction(&[]);
///     rpc.request_airdrop("address", 1);
///     rpc.deploy("program");
///     rpc.upgrade("program");
/// }
/// ```
pub struct HttpRpc {
    client: Client,
    url: Url,
}
impl HttpRpc {
    pub fn from_config(config: &SnapshotConfig) -> Result<Self> {
        if config.rpc_url.is_some() && config.rpc_url_env.is_some() {
            return Err(err("CONFIG", "rpc", "choose rpc_url or rpc_url_env"));
        }
        let endpoint = match (&config.rpc_url, &config.rpc_url_env) {
            (Some(url), _) => url.clone(),
            (_, Some(name)) => std::env::var(name)
                .map_err(|_| err("CONFIG", "rpc_url_env", "RPC environment variable is unset"))?,
            _ => "https://api.devnet.solana.com".into(),
        };
        Self::new(&endpoint, Duration::from_secs(20))
    }
    pub fn new(endpoint: &str, timeout: Duration) -> Result<Self> {
        let url = Url::parse(endpoint).map_err(|_| err("CONFIG", "rpc", "invalid RPC URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.fragment().is_some()
        {
            return Err(err(
                "CONFIG",
                "rpc",
                "expected an HTTP(S) endpoint without a fragment",
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| err("RPC_UNAVAILABLE", "client", "cannot initialize HTTP client"))?;
        Ok(Self { client, url })
    }
    fn request(&self, method: ReadMethod, params: Value) -> Result<Value> {
        let response = self
            .client
            .post(self.url.clone())
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method.name(),"params":params}))
            .send()
            .map_err(|e| {
                err(
                    if e.is_timeout() {
                        "RPC_TIMEOUT"
                    } else {
                        "RPC_UNAVAILABLE"
                    },
                    "rpc",
                    "RPC request failed",
                )
            })?;
        let status = response.status();
        if status.as_u16() == 429 {
            return Err(err("RPC_RATE_LIMITED", "rpc", "RPC rate limit exceeded"));
        }
        if !status.is_success() {
            return Err(err(
                "RPC_UNAVAILABLE",
                "rpc",
                format!("HTTP status {}", status.as_u16()),
            ));
        }
        const MAX_RESPONSE: usize = MAX_TOTAL_BYTES * 2;
        let mut data = Vec::new();
        response
            .take((MAX_RESPONSE + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|e| {
                err(
                    if e.kind() == std::io::ErrorKind::TimedOut {
                        "RPC_TIMEOUT"
                    } else {
                        "RPC_UNAVAILABLE"
                    },
                    "rpc",
                    "could not read RPC response",
                )
            })?;
        if data.len() > MAX_RESPONSE {
            return Err(err(
                "ACCOUNT_DATA_TOO_LARGE",
                "rpc",
                "RPC response limit exceeded",
            ));
        }
        let value: Value = serde_json::from_slice(&data)
            .map_err(|_| err("RPC_RESPONSE_INVALID", "rpc", "invalid JSON response"))?;
        if value.get("jsonrpc") != Some(&json!("2.0")) || value.get("id") != Some(&json!(1)) {
            return Err(err(
                "RPC_RESPONSE_INVALID",
                "rpc",
                "invalid JSON-RPC envelope",
            ));
        }
        if let Some(e) = value.get("error") {
            let code = match e.get("code").and_then(Value::as_i64) {
                Some(-32016) => "SLOT_CONSISTENCY_FAILED",
                Some(429) => "RPC_RATE_LIMITED",
                _ => "RPC_UNAVAILABLE",
            };
            return Err(err(code, "rpc", "RPC returned an error"));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| err("RPC_RESPONSE_INVALID", "rpc", "missing result"))
    }
}
impl ReadOnlyRpc for HttpRpc {
    fn genesis_hash(&self) -> Result<String> {
        self.request(ReadMethod::Genesis, json!([]))?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| err("RPC_RESPONSE_INVALID", "genesis", "expected string"))
    }
    fn multiple_accounts(
        &self,
        addresses: &[String],
        min_context_slot: Option<u64>,
    ) -> Result<AccountBatch> {
        if addresses.is_empty() || addresses.len() > MAX_ACCOUNTS {
            return Err(err(
                "ACCOUNT_DATA_TOO_LARGE",
                "accounts",
                "expected 1..100 account addresses",
            ));
        }
        for a in addresses {
            address(a)?;
        }
        let mut options = json!({"encoding":"base64","commitment":"finalized"});
        if let Some(slot) = min_context_slot {
            options["minContextSlot"] = json!(slot);
        }
        let result = self.request(ReadMethod::Accounts, json!([addresses, options]))?;
        let slot = result
            .pointer("/context/slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                err(
                    "RPC_RESPONSE_INVALID",
                    "slot",
                    "missing or invalid context slot",
                )
            })?;
        let accounts = result
            .get("value")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| err("RPC_RESPONSE_INVALID", "accounts", "expected account array"))?;
        Ok(AccountBatch { slot, accounts })
    }
    fn origin(&self) -> String {
        // Providers also put API keys in URL paths. Retain only scheme, host and port.
        self.url.origin().ascii_serialization()
    }
    fn acquisition(&self) -> Acquisition {
        Acquisition::Rpc
    }
}

pub fn parse_account(
    pubkey: &str,
    value: Value,
    limit: usize,
) -> Result<Option<(RawAccountSnapshot, u64)>> {
    address(pubkey)?;
    if limit == 0 || limit > MAX_ACCOUNT_BYTES {
        return Err(err(
            "ACCOUNT_DATA_TOO_LARGE",
            pubkey,
            "invalid account limit",
        ));
    }
    if value.is_null() {
        return Ok(None);
    }
    let invalid = || err("RPC_RESPONSE_INVALID", pubkey, "malformed account response");
    let encoded = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if encoded.len() != 2 || encoded[1] != json!("base64") {
        return Err(invalid());
    }
    let data = encoded[0].as_str().ok_or_else(invalid)?;
    if data.len() > limit.div_ceil(3).saturating_mul(4) {
        return Err(err(
            "ACCOUNT_DATA_TOO_LARGE",
            pubkey,
            "encoded account exceeds byte limit",
        ));
    }
    let data = STANDARD.decode(data).map_err(|_| invalid())?;
    if data.len() > limit {
        return Err(err(
            "ACCOUNT_DATA_TOO_LARGE",
            pubkey,
            "account exceeds byte limit",
        ));
    }
    if let Some(space) = value.get("space") {
        if space.as_u64() != Some(data.len() as u64) {
            return Err(invalid());
        }
    }
    let owner = value
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    address(owner)?;
    let lamports = value
        .get("lamports")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let executable = value
        .get("executable")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    let rent = value
        .get("rentEpoch")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let raw = RawAccountSnapshot::capture(
        pubkey.into(),
        owner.into(),
        lamports,
        executable,
        data,
        DecodeLimits {
            max_bytes: limit,
            ..DecodeLimits::default()
        },
    )
    .map_err(|_| invalid())?;
    Ok(Some((raw, rent)))
}
