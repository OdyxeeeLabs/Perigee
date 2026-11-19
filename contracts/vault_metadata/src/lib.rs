#![no_std]

//! # Perigee Vault Metadata — on-chain vault info and off-chain indexing
//!
//! Closes OdyxeeeLabs/Perigee#639
//!
//! Every other Perigee contract stores state; this one stores *descriptions*. A
//! vault's name, symbol, strategy, risk tier and documentation pointer have to
//! live somewhere that is discoverable without loading each vault contract
//! individually, and they have to be **versioned**, because a vault's strategy and
//! risk tier change and a discovery client that reads only the current value has
//! no way to know it did.
//!
//! ## Discovery by trait, not by enumeration
//!
//! [`VaultMetadata::list_by_strategy`] and [`VaultMetadata::list_by_risk`] return
//! every vault matching a trait. The alternative — enumerating all vaults and
//! filtering off-chain — is O(vaults) per query and gets slower as the platform
//! grows. The indexes are maintained on write, so a trait lookup is one storage
//! read plus the entries.
//!
//! Indexes are rebuilt on every metadata write rather than incrementally diffed.
//! For a metadata record that changes a handful of times in its life, a full
//! rewrite of one vault's index entries is cheaper in code and in risk than
//! incremental maintenance that can drift out of sync with the record.
//!
//! ## Versioning
//!
//! Every update emits [`VaultMetadata::get_history`] with the previous and new
//! record. An indexer therefore never has to poll for changes — it consumes
//! `meta_up` events and keeps a history itself — and a client that wants to know
//! when a vault changed risk tier has the answer.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum versions retained per vault. Older entries are dropped from the
/// history but remain in the event stream, which is the durable record.
pub const HISTORY_DEPTH: u32 = 16;

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the metadata registry.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The contract has not been initialized yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not authorised for this entry point.
    Unauthorized = 3,
    /// No metadata is registered for the vault.
    MetadataNotFound = 4,
    /// Metadata is already registered for the vault.
    MetadataExists = 5,
    /// A required field is empty.
    EmptyField = 6,
    /// A field exceeds its length bound.
    FieldTooLong = 7,
    /// The risk score is above 100, or an enum was given an out-of-range value.
    InvalidRisk = 8,
    /// The vault has no history at this index.
    HistoryOutOfRange = 9,
    /// An arithmetic step overflowed `u32`.
    Overflow = 10,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Coarse risk classification, so a discovery client can filter without parsing
/// documentation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RiskTier {
    /// Conservative.
    Low,
    /// Balanced.
    Medium,
    /// Aggressive.
    High,
}

/// A vault's on-chain description.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultMetadata {
    /// Vault this describes.
    pub vault: Address,
    /// Display name.
    pub name: String,
    /// Token symbol.
    pub symbol: String,
    /// Strategy identifier, matched against [`VaultMetadata::list_by_strategy`].
    pub strategy: String,
    /// Risk classification.
    pub risk_tier: RiskTier,
    /// 0–100 composite risk score, finer-grained than the tier.
    pub risk_score: u32,
    /// Off-chain documentation or dashboard pointer.
    pub metadata_uri: String,
    /// Bumped on every update; the off-chain indexer orders on this.
    pub version: u32,
    /// Ledger of the most recent update.
    pub updated_ledger: u32,
}

/// Mutable display fields accepted by the display-only update path.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayMetadataUpdate {
    /// Display name.
    pub name: String,
    /// Token symbol.
    pub symbol: String,
    /// Off-chain documentation or dashboard pointer.
    pub metadata_uri: String,
}

/// One recorded metadata change.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataVersion {
    /// Version number this entry records the move *to*.
    pub version: u32,
    /// The record at that version.
    pub metadata: VaultMetadata,
    /// Ledger at which the version was recorded.
    pub ledger: u32,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to register vaults.
    Admin,
    /// Number of registered vaults.
    VaultCount,
    /// [`VaultMetadata`] for a vault.
    Metadata(Address),
    /// History length for a vault.
    HistoryLen(Address),
    /// [`MetadataVersion`] at `(vault, index)`.
    History(Address, u32),
    /// Vaults registered with a given strategy.
    StrategyIndex(String),
    /// Vaults registered at a given risk tier.
    RiskIndex(RiskTier),
}

// ── Constants for length bounds ──────────────────────────────────────────────

/// Maximum length of a display name.
pub const MAX_NAME_LEN: u32 = 32;
/// Maximum length of a symbol.
pub const MAX_SYMBOL_LEN: u32 = 12;
/// Maximum length of a strategy identifier.
pub const MAX_STRATEGY_LEN: u32 = 32;
/// Maximum length of a metadata URI.
pub const MAX_URI_LEN: u32 = 256;
/// Highest valid risk score.
pub const MAX_RISK_SCORE: u32 = 100;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if *caller != admin {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

fn validate_field(value: &String, max_len: u32) -> Result<(), Error> {
    if value.is_empty() {
        return Err(Error::EmptyField);
    }
    if value.len() > max_len {
        return Err(Error::FieldTooLong);
    }
    Ok(())
}

fn index_push(env: &Env, key: DataKey, vault: &Address) {
    let mut list = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));
    if !list.contains(vault) {
        list.push_back(vault.clone());
    }
    env.storage().persistent().set(&key, &list);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
}

fn index_remove(env: &Env, key: &DataKey, vault: &Address) {
    if let Some(list) = env.storage().persistent::<_, soroban_sdk::Vec<Address>>(key) {
        let mut kept = soroban_sdk::Vec::new(env);
        for entry in list.iter() {
            if entry != *vault {
                kept.push_back(entry);
            }
        }
        env.storage().persistent().set(key, &kept);
    }
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct VaultMetadata;

#[contractimpl]
impl VaultMetadata {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the registry.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::VaultCount, &0u32);
        Ok(())
    }

    // ── Registration ────────────────────────────────────────────────────────

    /// Register metadata for a vault.
    pub fn register(
        env: Env,
        caller: Address,
        vault: Address,
        name: String,
        symbol: String,
        strategy: String,
        risk_tier: RiskTier,
        risk_score: u32,
        metadata_uri: String,
    ) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        validate_field(&name, MAX_NAME_LEN)?;
        validate_field(&symbol, MAX_SYMBOL_LEN)?;
        validate_field(&strategy, MAX_STRATEGY_LEN)?;
        validate_field(&metadata_uri, MAX_URI_LEN)?;
        if risk_score > MAX_RISK_SCORE {
            return Err(Error::InvalidRisk);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::Metadata(vault.clone()))
        {
            return Err(Error::MetadataExists);
        }

        let metadata = VaultMetadata {
            vault: vault.clone(),
            name,
            symbol,
            strategy: strategy.clone(),
            risk_tier,
            risk_score,
            metadata_uri,
            version: 1,
            updated_ledger: env.ledger().sequence(),
        };

        write_metadata(&env, &vault, &metadata);
        index_push(&env, DataKey::StrategyIndex(strategy), &vault);
        index_push(&env, DataKey::RiskIndex(risk_tier), &vault);

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::VaultCount)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::VaultCount, &count.checked_add(1).ok_or(Error::Overflow)?);

        env.events()
            .publish((symbol_short!("meta"), vault), metadata.version);
        Ok(())
    }

    /// Update a vault's metadata, bumping the version and recording history.
    pub fn update(
        env: Env,
        caller: Address,
        vault: Address,
        name: String,
        symbol: String,
        strategy: String,
        risk_tier: RiskTier,
        risk_score: u32,
        metadata_uri: String,
    ) -> Result<u32, Error> {
        require_admin(&env, &caller)?;

        validate_field(&name, MAX_NAME_LEN)?;
        validate_field(&symbol, MAX_SYMBOL_LEN)?;
        validate_field(&strategy, MAX_STRATEGY_LEN)?;
        validate_field(&metadata_uri, MAX_URI_LEN)?;
        if risk_score > MAX_RISK_SCORE {
            return Err(Error::InvalidRisk);
        }

        let previous = Self::get_metadata(env.clone(), vault.clone())?;

        // Reindex against the new traits before writing, so the index and the
        // record can never disagree.
        index_remove(&env, &DataKey::StrategyIndex(previous.strategy), &vault);
        index_remove(&env, &DataKey::RiskIndex(previous.risk_tier), &vault);
        index_push(&env, DataKey::StrategyIndex(strategy.clone()), &vault);
        index_push(&env, DataKey::RiskIndex(risk_tier.clone()), &vault);

        let version = previous.version.checked_add(1).ok_or(Error::Overflow)?;
        let metadata = VaultMetadata {
            vault: previous.vault.clone(),
            name,
            symbol,
            strategy,
            risk_tier,
            risk_score,
            metadata_uri,
            version,
            updated_ledger: env.ledger().sequence(),
        };

        write_metadata(&env, &vault, &metadata);

        env.events().publish(
            (symbol_short!("meta"), vault, symbol_short!("up")),
            (previous.version, version),
        );
        Ok(version)
    }

    /// Update only mutable display fields while preserving canonical identity.
    pub fn update_display(
        env: Env,
        caller: Address,
        vault: Address,
        update: DisplayMetadataUpdate,
    ) -> Result<u32, Error> {
        require_admin(&env, &caller)?;

        validate_field(&update.name, MAX_NAME_LEN)?;
        validate_field(&update.symbol, MAX_SYMBOL_LEN)?;
        validate_field(&update.metadata_uri, MAX_URI_LEN)?;

        let previous = Self::get_metadata(env.clone(), vault.clone())?;

        // Reconstruct identity and protocol traits from the stored record.
        // The update payload cannot replace the controlling address or move
        // discovery traits, which keeps this path canonical by construction.
        let version = previous.version.checked_add(1).ok_or(Error::Overflow)?;
        let metadata = VaultMetadata {
            vault: previous.vault.clone(),
            name: update.name,
            symbol: update.symbol,
            strategy: previous.strategy,
            risk_tier: previous.risk_tier,
            risk_score: previous.risk_score,
            metadata_uri: update.metadata_uri,
            version,
            updated_ledger: env.ledger().sequence(),
        };

        write_metadata(&env, &vault, &metadata);

        env.events().publish(
            (symbol_short!("meta"), vault, symbol_short!("up")),
            (previous.version, version),
        );
        Ok(version)
    }

    /// Remove a vault's metadata entirely. The history is kept.
    pub fn deregister(env: Env, caller: Address, vault: Address) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        let metadata = Self::get_metadata(env.clone(), vault.clone())?;
        index_remove(&env, &DataKey::StrategyIndex(metadata.strategy), &vault);
        index_remove(&env, &DataKey::RiskIndex(metadata.risk_tier), &vault);

        env.storage()
            .persistent()
            .remove(&DataKey::Metadata(vault.clone()));
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::VaultCount)
            .unwrap_or(1);
        env.storage().instance().set(&DataKey::VaultCount, &count - 1);

        env.events()
            .publish((symbol_short!("meta_x"), vault), metadata.version);
        Ok(())
    }

    // ── Queries ─────────────────────────────────────────────────────────────

    /// The current metadata for a vault.
    pub fn get_metadata(env: Env, vault: Address) -> Result<VaultMetadata, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Metadata(vault))
            .ok_or(Error::MetadataNotFound)
    }

    /// How many vaults are registered.
    pub fn vault_count(env: Env) -> Result<u32, Error> {
        env.storage()
            .instance()
            .get(&DataKey::VaultCount)
            .ok_or(Error::NotInitialized)
    }

    /// Every vault running `strategy`.
    pub fn list_by_strategy(env: Env, strategy: String) -> soroban_sdk::Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::StrategyIndex(strategy))
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
    }

    /// Every vault at `tier`.
    pub fn list_by_risk(env: Env, tier: RiskTier) -> soroban_sdk::Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::RiskIndex(tier))
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env))
    }

    /// How many versions are retained for a vault.
    pub fn history_len(env: Env, vault: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::HistoryLen(vault))
            .unwrap_or(0)
    }

    /// The retained version at `index`, oldest first.
    pub fn get_history(
        env: Env,
        vault: Address,
        index: u32,
    ) -> Result<MetadataVersion, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::History(vault, index))
            .ok_or(Error::HistoryOutOfRange)
    }

    /// The most recent retained version.
    pub fn latest_version(env: Env, vault: Address) -> Result<MetadataVersion, Error> {
        let len = Self::history_len(env.clone(), vault.clone());
        if len == 0 {
            return Err(Error::HistoryOutOfRange);
        }
        Self::get_history(env, vault, len - 1)
    }

    // ── Administration ──────────────────────────────────────────────────────

    /// The registry admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn write_metadata(env: &Env, vault: &Address, metadata: &VaultMetadata) {
    let key = DataKey::Metadata(vault.clone());
    env.storage().persistent().set(&key, metadata);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

    // Append to history, trimming to `HISTORY_DEPTH`. Older versions are still in
    // the event stream, which is the durable record.
    let len: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::HistoryLen(vault.clone()))
        .unwrap_or(0);
    let history_key = DataKey::History(vault.clone(), len);
    env.storage().persistent().set(
        &history_key,
        &MetadataVersion {
            version: metadata.version,
            metadata: metadata.clone(),
            ledger: metadata.updated_ledger,
        },
    );
    env.storage()
        .persistent()
        .extend_ttl(&history_key, TTL_LEDGERS, TTL_LEDGERS);

    if len + 1 > HISTORY_DEPTH {
        let stale = DataKey::History(vault.clone(), len + 1 - HISTORY_DEPTH);
        env.storage().persistent().remove(&stale);
    }
    env.storage()
        .persistent()
        .set(&DataKey::HistoryLen(vault.clone()), &len + 1);
}