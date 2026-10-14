//! Raw observations remain available even when an account cannot be decoded.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawAccountSnapshot {
    pub schema_version: u32,
    pub address: String,
    pub owner: String,
    #[serde(with = "decimal")]
    pub lamports: BigInt,
    pub executable: bool,
    pub data_length: usize,
    pub raw_data: Vec<u8>,
    pub metadata: BTreeMap<String, String>,
}

impl RawAccountSnapshot {
    pub fn capture(
        address: String,
        owner: String,
        lamports: u64,
        executable: bool,
        raw_data: Vec<u8>,
        limits: DecodeLimits,
    ) -> Result<Self> {
        let snapshot = Self {
            schema_version: 1,
            address,
            owner,
            lamports: lamports.into(),
            executable,
            data_length: raw_data.len(),
            raw_data,
            metadata: BTreeMap::new(),
        };
        snapshot.validate(limits)?;
        Ok(snapshot)
    }
    pub fn validate(&self, limits: DecodeLimits) -> Result<()> {
        if self.schema_version != 1
            || self.data_length != self.raw_data.len()
            || self.lamports < BigInt::from(0u8)
            || self.lamports > BigInt::from(u64::MAX)
        {
            return Err(error(
                "SNAPSHOT",
                &self.address,
                0,
                "invalid version, length or lamports",
            ));
        }
        if self.data_length > limits.max_bytes {
            return Err(error(
                "LIMIT",
                &self.address,
                0,
                "account byte limit exceeded",
            ));
        }
        for address in [&self.address, &self.owner] {
            Address::from_str(address)
                .map_err(|e| error("SNAPSHOT", &self.address, 0, e.to_string()))?;
        }
        Ok(())
    }
    pub fn decode(
        &self,
        binding: AccountBinding,
        model: &ProgramModel,
        limits: DecodeLimits,
        trailing: TrailingBytes,
    ) -> Result<AccountSnapshot> {
        self.validate(limits)?;
        if binding.address != self.address || binding.owner != self.owner {
            return Err(error(
                "SNAPSHOT",
                &binding.name,
                0,
                "observed address or owner does not match binding",
            ));
        }
        Ok(AccountSnapshot {
            schema_version: 1,
            decoded: decode_account(
                model,
                &binding.account_type,
                &self.raw_data,
                limits,
                trailing,
            )?,
            binding,
            lamports: self.lamports.clone(),
            executable: self.executable,
            raw_data: self.raw_data.clone(),
            data_length: self.data_length,
            metadata: self.metadata.clone(),
        })
    }
}

/// In-memory snapshot set; keys are public keys, not project-specific account names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSet {
    pub accounts: BTreeMap<String, RawAccountSnapshot>,
}
impl SnapshotSet {
    pub fn capture(
        accounts: impl IntoIterator<Item = RawAccountSnapshot>,
        limits: DecodeLimits,
    ) -> Result<Self> {
        let mut set = Self::default();
        for account in accounts {
            account.validate(limits)?;
            let address = account.address.clone();
            if set.accounts.insert(address.clone(), account).is_some() {
                return Err(error("SNAPSHOT", &address, 0, "duplicate account address"));
            }
        }
        Ok(set)
    }
    pub fn account(&self, address: &str, expected_owner: &str) -> Result<&RawAccountSnapshot> {
        let account = self
            .accounts
            .get(address)
            .ok_or_else(|| error("MISSING_ACCOUNT", address, 0, "account is absent"))?;
        if account.owner != expected_owner {
            return Err(error(
                "OWNER_MISMATCH",
                address,
                0,
                "observed account owner differs from expected owner",
            ));
        }
        Ok(account)
    }
    /// Validate all records before replacing an in-memory state set (no partial restore).
    /// Restoring a VM, programs, sysvars, or transaction history is a later runtime concern.
    pub fn restore_into(&self, destination: &mut Self, limits: DecodeLimits) -> Result<()> {
        for (address, account) in &self.accounts {
            if *address != account.address {
                return Err(error(
                    "SNAPSHOT",
                    address,
                    0,
                    "snapshot key does not match address",
                ));
            }
            account.validate(limits)?;
        }
        *destination = self.clone();
        Ok(())
    }
}
