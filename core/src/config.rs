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
//! // Fail fast if a required ID is missing:
//! let id = cfg.require("CONTRACT_POLICY_VAULT", cfg.policy_vault.as_deref())
//!             .expect("policy vault contract ID must be set");
//! ```

use std::env;

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
}
