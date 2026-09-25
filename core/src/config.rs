//! config.rs
//!
//! Single source of truth for contract IDs consumed by the Perigee core
//! service.  Values are read from environment variables at startup so the same
//! binary can be pointed at local, testnet, or mainnet by changing env vars
//! alone — no recompile required.
//!
//! # Environment variables
//!
//! | Variable                            | Contract                        |
//! |-------------------------------------|---------------------------------|
//! | `CONTRACT_POLICY_VAULT`             | Policy Vault                    |
//! | `CONTRACT_STRATEGY_TRIGGER`         | Strategy Trigger                |
//! | `CONTRACT_FEE_ACCRUAL`              | Fee Accrual / high-water mark   |
//! | `CONTRACT_EMERGENCY_GUARD`          | Emergency Guard circuit breaker |
//! | `CONTRACT_LIQUIDITY_POOL`           | AMM / stable LP rotation        |
//! | `CONTRACT_TOKEN`                    | Token (USDC anchor)             |
//! | `CONTRACT_ORACLE_AGGREGATOR`        | Oracle Aggregator               |
//! | `CONTRACT_CROSS_CHAIN_VERIFIER`     | Cross-Chain Verifier            |
//! | `CONTRACT_HELLO_SOROBAN`            | Hello Soroban (smoke test)      |
//!
//! Per-env defaults are provided for the well-known testnet deployment so that
//! local developer workflows require minimal setup.  Override any value by
//! setting the corresponding env var before starting the service.
//!
//! # Usage
//!
//! ```rust,no_run
//! use Perigee_core::config::ContractConfig;
//!
//! let cfg = ContractConfig::from_env();
//! println!("policy vault: {:?}", cfg.policy_vault);
//!
//! // Fail fast if a required ID is missing:
//! let id = cfg.require("CONTRACT_POLICY_VAULT", cfg.policy_vault.as_deref())
//!             .expect("policy vault contract ID must be set");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::{env, sync::RwLock};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretKeyringError {
    #[error("secret key id must not be empty")]
    EmptyKeyId,
    #[error("secret key id must contain only letters, digits, '.', '_' or '-'")]
    InvalidKeyId,
    #[error("secret key id is duplicated")]
    DuplicateKeyId,
    #[error("secret key overlap must be greater than zero")]
    InvalidOverlap,
    #[error("secret key is not valid for verification")]
    InactiveKey,
    #[error("secret keyring lock is unavailable")]
    LockPoisoned,
}

#[derive(Clone)]
pub struct SecretVersion<T> {
    id: String,
    value: T,
    expires_at: Option<u64>,
}

impl<T> SecretVersion<T> {
    pub fn new(id: impl Into<String>, value: T) -> Self {
        Self {
            id: id.into(),
            value,
            expires_at: None,
        }
    }

    pub fn with_expiry(mut self, expires_at: Option<u64>) -> Self {
        self.expires_at = expires_at;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }
}

struct SecretKeyringState<T> {
    current: SecretVersion<T>,
    previous: HashMap<String, SecretVersion<T>>,
    overlap_secs: u64,
}

pub struct SecretKeyring<T> {
    state: RwLock<SecretKeyringState<T>>,
}

fn validate_secret_id(id: &str) -> Result<(), SecretKeyringError> {
    if id.is_empty() {
        return Err(SecretKeyringError::EmptyKeyId);
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(SecretKeyringError::InvalidKeyId);
    }
    Ok(())
}

fn secret_is_active<T>(version: &SecretVersion<T>, now: u64) -> bool {
    version.expires_at.map_or(true, |expires_at| expires_at > now)
}

impl<T: Clone> SecretKeyring<T> {
    pub fn new(
        current: SecretVersion<T>,
        previous: Vec<SecretVersion<T>>,
        now: u64,
        overlap_secs: u64,
    ) -> Result<Self, SecretKeyringError> {
        if overlap_secs == 0 {
            return Err(SecretKeyringError::InvalidOverlap);
        }
        validate_secret_id(current.id())?;
        if current.expires_at.is_some() {
            return Err(SecretKeyringError::InactiveKey);
        }

        let default_expiry = now.saturating_add(overlap_secs);
        let mut ids = HashSet::new();
        ids.insert(current.id().to_string());
        let mut previous_map = HashMap::with_capacity(previous.len());

        for mut version in previous {
            validate_secret_id(version.id())?;
            if !ids.insert(version.id().to_string()) {
                return Err(SecretKeyringError::DuplicateKeyId);
            }
            if version.expires_at.is_none() {
                version.expires_at = Some(default_expiry);
            }
            if !secret_is_active(&version, now) {
                return Err(SecretKeyringError::InactiveKey);
            }
            previous_map.insert(version.id().to_string(), version);
        }

        Ok(Self {
            state: RwLock::new(SecretKeyringState {
                current,
                previous: previous_map,
                overlap_secs,
            }),
        })
    }

    pub fn current(&self) -> Result<SecretVersion<T>, SecretKeyringError> {
        self.state
            .read()
            .map(|state| state.current.clone())
            .map_err(|_| SecretKeyringError::LockPoisoned)
    }

    pub fn verification_keys(&self, now: u64) -> Result<Vec<SecretVersion<T>>, SecretKeyringError> {
        let state = self
            .state
            .read()
            .map_err(|_| SecretKeyringError::LockPoisoned)?;
        let mut keys = Vec::with_capacity(state.previous.len() + 1);
        keys.push(state.current.clone());
        keys.extend(
            state
                .previous
                .values()
                .filter(|version| secret_is_active(version, now))
                .cloned(),
        );
        Ok(keys)
    }

    pub fn verification_key(
        &self,
        id: &str,
        now: u64,
    ) -> Result<Option<SecretVersion<T>>, SecretKeyringError> {
        Ok(self
            .verification_keys(now)?
            .into_iter()
            .find(|version| version.id() == id))
    }

    pub fn rotate(
        &self,
        version: SecretVersion<T>,
        now: u64,
    ) -> Result<(), SecretKeyringError> {
        validate_secret_id(version.id())?;
        if version.expires_at.is_some() {
            return Err(SecretKeyringError::InactiveKey);
        }

        let mut state = self
            .state
            .write()
            .map_err(|_| SecretKeyringError::LockPoisoned)?;
        if state.current.id() == version.id() || state.previous.contains_key(version.id()) {
            return Err(SecretKeyringError::DuplicateKeyId);
        }

        let retired = std::mem::replace(&mut state.current, version);
        let retired_expiry = now.saturating_add(state.overlap_secs);
        state
            .previous
            .insert(retired.id().to_string(), retired.with_expiry(Some(retired_expiry)));
        state
            .previous
            .retain(|_, previous| secret_is_active(previous, now));
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FeatureFlag {
    EnableVaultV2,
    EnableNewFeeModel,
}

impl FeatureFlag {
    pub const ALL: [Self; 2] = [Self::EnableVaultV2, Self::EnableNewFeeModel];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EnableVaultV2 => "enable_vault_v2",
            Self::EnableNewFeeModel => "enable_new_fee_model",
        }
    }

    pub fn env_name(&self) -> String {
        self.as_str().to_ascii_uppercase()
    }

    pub fn from_key(value: &str) -> Result<Self, FeatureFlagConfigError> {
        Self::ALL
            .into_iter()
            .find(|flag| flag.as_str() == value.trim())
            .ok_or_else(|| FeatureFlagConfigError::UnknownFlag(value.trim().to_string()))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FeatureFlagConfigError {
    #[error("FEATURE_FLAGS is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("unknown backend feature flag '{0}'")]
    UnknownFlag(String),
    #[error("backend feature flag '{name}' must be true or false, got '{value}'")]
    InvalidValue { name: String, value: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureFlagService {
    values: BTreeMap<FeatureFlag, bool>,
}

impl FeatureFlagService {
    pub fn from_config(source: &str) -> Result<Self, FeatureFlagConfigError> {
        let mut values = Self::default();
        let trimmed = source.trim();
        if !trimmed.is_empty() {
            if trimmed.starts_with('{') {
                let raw = serde_json::from_str::<BTreeMap<String, bool>>(trimmed)
                    .map_err(|error| FeatureFlagConfigError::InvalidJson(error.to_string()))?;
                for (key, value) in raw {
                    values.insert(FeatureFlag::from_key(&key)?, value);
                }
            } else if trimmed.starts_with('[') {
                let enabled = serde_json::from_str::<Vec<String>>(trimmed)
                    .map_err(|error| FeatureFlagConfigError::InvalidJson(error.to_string()))?;
                for key in enabled {
                    values.insert(FeatureFlag::from_key(&key)?, true);
                }
            } else {
                for item in trimmed.split(',') {
                    let item = item.trim();
                    if item.is_empty() {
                        continue;
                    }
                    let (key, value) = item.split_once('=').unwrap_or((item, "true"));
                    let key = key.trim();
                    let value = value.trim();
                    let flag = FeatureFlag::from_key(key)?;
                    let enabled = match value {
                        "true" => true,
                        "false" => false,
                        _ => {
                            return Err(FeatureFlagConfigError::InvalidValue {
                                name: key.to_string(),
                                value: value.to_string(),
                            })
                        }
                    };
                    values.insert(flag, enabled);
                }
            }
        }

        for flag in FeatureFlag::ALL {
            if let Ok(raw) = env::var(flag.env_name()) {
                let value = raw.trim();
                let enabled = match value {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(FeatureFlagConfigError::InvalidValue {
                            name: flag.as_str().to_string(),
                            value: value.to_string(),
                        })
                    }
                };
                values.insert(flag, enabled);
            }
        }

        Ok(Self { values })
    }

    pub fn from_env() -> Result<Self, FeatureFlagConfigError> {
        Self::from_config(&env::var("FEATURE_FLAGS").unwrap_or_default())
    }

    pub fn is_enabled(&self, flag: FeatureFlag) -> bool {
        self.values.get(&flag).copied().unwrap_or(false)
    }

    pub fn snapshot(&self) -> BTreeMap<&'static str, bool> {
        FeatureFlag::ALL
            .into_iter()
            .map(|flag| (flag.as_str(), self.is_enabled(flag)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Testnet defaults
// The IDs below are the latest known testnet deployment.  Override any of them
// via the corresponding env var without changing this file.
// ---------------------------------------------------------------------------

/// Hardcoded testnet contract ID used as a default / smoke-test fixture.
/// This value intentionally lives in one place so tests and defaults can both
/// reference `TESTNET_DEFAULT_CONTRACT_ID` instead of raw string literals.
pub const TESTNET_DEFAULT_CONTRACT_ID: &str =
    "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";

// ---------------------------------------------------------------------------
// ContractConfig
// ---------------------------------------------------------------------------

/// Runtime contract IDs, resolved from environment variables.
///
/// All fields are `Option<String>` — `None` means the env var was not set and
/// no built-in default applies for that contract on the active stage.
#[derive(Debug, Clone)]
pub struct ContractConfig {
    pub policy_vault:          Option<String>,
    pub strategy_trigger:      Option<String>,
    pub fee_accrual:            Option<String>,
    pub emergency_guard:       Option<String>,
    pub liquidity_pool:        Option<String>,
    pub token:                 Option<String>,
    pub oracle_aggregator:     Option<String>,
    pub cross_chain_verifier:  Option<String>,
    pub hello_soroban:         Option<String>,
}

impl ContractConfig {
    /// Build a `ContractConfig` by reading environment variables.
    ///
    /// If `STELLAR_NETWORK` / `APP_ENV` is `testnet` (or not set), testnet
    /// defaults fill any env var that was not explicitly provided.
    pub fn from_env() -> Self {
        let stage = detect_stage();
        Self {
            policy_vault:         read_or_default("CONTRACT_POLICY_VAULT",            None,                           &stage),
            strategy_trigger:     read_or_default("CONTRACT_STRATEGY_TRIGGER",        None,                           &stage),
            fee_accrual:           read_or_default("CONTRACT_FEE_ACCRUAL",             None,                           &stage),
            emergency_guard:      read_or_default("CONTRACT_EMERGENCY_GUARD",         None,                           &stage),
            liquidity_pool:       read_or_default("CONTRACT_LIQUIDITY_POOL",          None,                           &stage),
            token:                read_or_default("CONTRACT_TOKEN",                   None,                           &stage),
            oracle_aggregator:    read_or_default("CONTRACT_ORACLE_AGGREGATOR",       None,                           &stage),
            cross_chain_verifier: read_or_default("CONTRACT_CROSS_CHAIN_VERIFIER",    None,                           &stage),
            // hello_soroban has a testnet default — used for smoke tests
            hello_soroban:        read_or_default("CONTRACT_HELLO_SOROBAN",
                                                  Some(TESTNET_DEFAULT_CONTRACT_ID),  &stage),
        }
    }

    /// Return the value for `var_name` or panic with a descriptive message.
    ///
    /// Use this for contract IDs that are **required** at startup (e.g. the
    /// policy vault on mainnet). Prefer `Option` accessors in library code
    /// that can degrade gracefully.
    pub fn require(&self, var_name: &str, value: Option<&str>) -> Result<String, ConfigError> {
        value.map(|s| s.to_owned()).ok_or_else(|| ConfigError::Missing(var_name.to_owned()))
    }
}

// ---------------------------------------------------------------------------
// Stage detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Local,
    Testnet,
    Mainnet,
}

/// Determine the active stage from `STELLAR_NETWORK` or `APP_ENV`.
pub fn detect_stage() -> Stage {
    let network = env::var("STELLAR_NETWORK")
        .or_else(|_| env::var("APP_ENV"))
        .unwrap_or_default()
        .to_lowercase();

    match network.as_str() {
        "mainnet" | "public" => Stage::Mainnet,
        "testnet" => Stage::Testnet,
        _ => Stage::Local,
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ConfigError {
    Missing(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Missing(var) => {
                write!(f, "required env var `{var}` is not set; \
                       check your .env file or deployment config")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read `var_name` from the environment.  If not set:
/// * on Testnet/Local: return `testnet_default` (if provided)
/// * on Mainnet: always return `None` (no defaults in production)
fn read_or_default(var_name: &str, testnet_default: Option<&str>, stage: &Stage) -> Option<String> {
    match env::var(var_name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => match stage {
            Stage::Mainnet => None,
            _ => testnet_default.map(|s| s.to_owned()),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Environment variables are process-global; see `errors::ENV_LOCK`.
    use crate::errors::env_guard;


    #[test]
    fn detect_stage_defaults_to_local() {
        let _env = env_guard();
        // Unset both vars — must return Local.
        // (env isolation is best-effort here; real isolation needs a temp env)
        let original = env::var("STELLAR_NETWORK").ok();
        unsafe { env::remove_var("STELLAR_NETWORK"); }
        unsafe { env::remove_var("APP_ENV"); }

        assert_eq!(detect_stage(), Stage::Local);

        // Restore
        if let Some(v) = original {
            unsafe { env::set_var("STELLAR_NETWORK", v); }
        }
    }

    #[test]
    fn detect_stage_testnet() {
        let _env = env_guard();
        unsafe { env::set_var("STELLAR_NETWORK", "testnet"); }
        assert_eq!(detect_stage(), Stage::Testnet);
        unsafe { env::remove_var("STELLAR_NETWORK"); }
    }

    #[test]
    fn detect_stage_mainnet() {
        let _env = env_guard();
        unsafe { env::set_var("STELLAR_NETWORK", "mainnet"); }
        assert_eq!(detect_stage(), Stage::Mainnet);
        unsafe { env::remove_var("STELLAR_NETWORK"); }

        unsafe { env::set_var("STELLAR_NETWORK", "public"); }
        assert_eq!(detect_stage(), Stage::Mainnet);
        unsafe { env::remove_var("STELLAR_NETWORK"); }
    }

    #[test]
    fn hello_soroban_has_testnet_default() {
        let _env = env_guard();
        // Without any env vars set the hello_soroban ID should be the
        // testnet default on Local/Testnet stages.
        unsafe { env::remove_var("STELLAR_NETWORK"); }
        unsafe { env::remove_var("CONTRACT_HELLO_SOROBAN"); }

        let cfg = ContractConfig::from_env();
        assert_eq!(
            cfg.hello_soroban.as_deref(),
            Some(TESTNET_DEFAULT_CONTRACT_ID),
        );
    }

    #[test]
    fn env_var_overrides_default() {
        let _env = env_guard();
        let custom = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM";
        unsafe { env::set_var("CONTRACT_HELLO_SOROBAN", custom); }

        let cfg = ContractConfig::from_env();
        assert_eq!(cfg.hello_soroban.as_deref(), Some(custom));

        unsafe { env::remove_var("CONTRACT_HELLO_SOROBAN"); }
    }

    #[test]
    fn require_returns_error_for_none() {
        let _env = env_guard();
        let cfg = ContractConfig::from_env();
        let result = cfg.require("CONTRACT_POLICY_VAULT", None);
        assert!(result.is_err());
    }

    #[test]
    fn require_returns_ok_for_some() {
        let _env = env_guard();
        let cfg = ContractConfig::from_env();
        let result = cfg.require("CONTRACT_HELLO_SOROBAN", Some(TESTNET_DEFAULT_CONTRACT_ID));
        assert_eq!(result.unwrap(), TESTNET_DEFAULT_CONTRACT_ID);
    }

    #[test]
    fn mainnet_stage_has_no_defaults() {
        let _env = env_guard();
        unsafe { env::set_var("STELLAR_NETWORK", "mainnet"); }
        unsafe { env::remove_var("CONTRACT_HELLO_SOROBAN"); }

        let cfg = ContractConfig::from_env();
        // On mainnet, no defaults should be injected.
        assert!(cfg.hello_soroban.is_none());

        unsafe { env::remove_var("STELLAR_NETWORK"); }
    }

    #[test]
    fn secret_keyring_rotates_without_dropping_previous_key() {
        let ring = SecretKeyring::new(
            SecretVersion::new("current", "old".to_string()),
            Vec::new(),
            100,
            60,
        )
        .unwrap();
        ring.rotate(SecretVersion::new("next", "new".to_string()), 110)
            .unwrap();
        assert_eq!(
            ring.verification_key("current", 150).unwrap().unwrap().value(),
            "old"
        );
        assert!(ring.verification_key("current", 171).unwrap().is_none());
        assert_eq!(ring.current().unwrap().value(), "new");
    }

    #[test]
    fn feature_flags_are_typed_and_disabled_by_default() {
        let _env = env_guard();
        let flags = FeatureFlagService::from_config("{}").unwrap();
        assert!(!flags.is_enabled(FeatureFlag::EnableVaultV2));
        assert!(!flags.is_enabled(FeatureFlag::EnableNewFeeModel));
    }

    #[test]
    fn feature_flags_accept_json_and_reject_unknown_names() {
        let _env = env_guard();
        let flags = FeatureFlagService::from_config(
            r#"{"enable_vault_v2":true,"enable_new_fee_model":false}"#,
        )
        .unwrap();
        assert!(flags.is_enabled(FeatureFlag::EnableVaultV2));
        assert!(!flags.is_enabled(FeatureFlag::EnableNewFeeModel));
        assert!(FeatureFlagService::from_config(r#"{"enable_unknown":true}"#).is_err());
    }
}
