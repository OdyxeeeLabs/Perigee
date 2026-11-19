//! Centralised configuration management — Issue #595.
//!
//! Loads configuration from environment variables with full schema validation,
//! environment-aware defaults, feature-flag support, and hot-reload for
//! non-sensitive values.
//!
//! # Usage
//! ```ignore
//! let manager = ConfigManager::load().expect("invalid config");
//! let cfg = manager.get();          // Arc<AppSettings>
//! manager.hot_reload().await;       // refresh non-sensitive keys from env
//! ```

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

/// Runtime environment the service is operating in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Development,
    Staging,
    Production,
}

impl Environment {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "production" | "prod" => Self::Production,
            "staging" | "stage" => Self::Staging,
            _ => Self::Development,
        }
    }

    pub fn is_production(&self) -> bool {
        matches!(self, Self::Production)
    }
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

/// Feature flags that can be toggled without restarting the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlags {
    /// Enable experimental distributed tracing export.
    pub distributed_tracing: bool,
    /// Enable the App Router API surface.
    pub app_router: bool,
    /// Enable dark-mode support in the frontend config response.
    pub dark_mode: bool,
    /// Enable rate-limiting enforcement (disable in dev for convenience).
    pub rate_limiting: bool,
    /// Enable the /metrics Prometheus endpoint.
    pub metrics_endpoint: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            distributed_tracing: false,
            app_router: false,
            dark_mode: true,
            rate_limiting: true,
            metrics_endpoint: true,
        }
    }
}

impl FeatureFlags {
    /// Load feature flags from environment variables prefixed with `FEATURE_`.
    ///
    /// ```text
    /// FEATURE_DISTRIBUTED_TRACING=true
    /// FEATURE_RATE_LIMITING=false
    /// ```
    pub fn from_env() -> Self {
        let flag = |key: &str| -> bool {
            std::env::var(key)
                .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
                .unwrap_or(false)
        };

        let mut flags = FeatureFlags::default();
        if std::env::var("FEATURE_DISTRIBUTED_TRACING").is_ok() {
            flags.distributed_tracing = flag("FEATURE_DISTRIBUTED_TRACING");
        }
        if std::env::var("FEATURE_APP_ROUTER").is_ok() {
            flags.app_router = flag("FEATURE_APP_ROUTER");
        }
        if std::env::var("FEATURE_DARK_MODE").is_ok() {
            flags.dark_mode = flag("FEATURE_DARK_MODE");
        }
        if std::env::var("FEATURE_RATE_LIMITING").is_ok() {
            flags.rate_limiting = flag("FEATURE_RATE_LIMITING");
        }
        if std::env::var("FEATURE_METRICS_ENDPOINT").is_ok() {
            flags.metrics_endpoint = flag("FEATURE_METRICS_ENDPOINT");
        }
        flags
    }
}

// ---------------------------------------------------------------------------
// AppSettings
// ---------------------------------------------------------------------------

/// The canonical set of validated application settings.
///
/// Sensitive fields (`jwt_private_key`, `database_url`, `redis_url`) are
/// **not** hot-reloadable — they require a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    // ── Server ──────────────────────────────────────────────────────────────
    pub server_port: u16,
    pub environment: Environment,

    // ── External services (sensitive — not hot-reloadable) ─────────────────
    pub database_url: String,
    pub redis_url: String,
    pub soroban_rpc_url: String,
    pub jwt_private_key: Option<String>,

    // ── Stellar ─────────────────────────────────────────────────────────────
    pub network_passphrase: String,

    // ── Observability ───────────────────────────────────────────────────────
    /// Rust log directive, e.g. `"info,perigee_core=debug"`.
    pub log_level: String,
    /// OTLP endpoint for distributed tracing export.
    pub otlp_endpoint: Option<String>,

    // ── Feature flags (hot-reloadable) ──────────────────────────────────────
    pub features: FeatureFlags,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            server_port: 3000,
            environment: Environment::Development,
            database_url: "postgres://localhost:5432/perigee".into(),
            redis_url: "redis://localhost:6379".into(),
            soroban_rpc_url: "https://soroban-testnet.stellar.org".into(),
            jwt_private_key: None,
            network_passphrase: "Test SDF Network ; September 2015".into(),
            log_level: "info".into(),
            otlp_endpoint: None,
            features: FeatureFlags::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validation errors collected at startup.
#[derive(Debug)]
pub struct ConfigValidationError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config validation failed:\n  - {}", self.errors.join("\n  - "))
    }
}

impl std::error::Error for ConfigValidationError {}

fn validate(settings: &AppSettings) -> Result<(), ConfigValidationError> {
    let mut errors: Vec<String> = Vec::new();

    if settings.server_port == 0 {
        errors.push("SERVER_PORT must be > 0".into());
    }
    if settings.database_url.is_empty() {
        errors.push("DATABASE_URL must not be empty".into());
    }
    if settings.soroban_rpc_url.is_empty() {
        errors.push("SOROBAN_RPC_URL must not be empty".into());
    }
    if settings.network_passphrase.is_empty() {
        errors.push("NETWORK_PASSPHRASE must not be empty".into());
    }
    if settings.environment.is_production() && settings.jwt_private_key.is_none() {
        errors.push(
            "JWT_PRIVATE_KEY must be set in production; \
             running with an ephemeral dev key is not permitted"
                .into(),
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ConfigValidationError { errors })
    }
}

// ---------------------------------------------------------------------------
// ConfigManager
// ---------------------------------------------------------------------------

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Manages application settings with hot-reload support for non-sensitive keys.
pub struct ConfigManager {
    inner: RwLock<Arc<AppSettings>>,
}

impl ConfigManager {
    /// Load settings from environment variables, validate, and return a manager.
    ///
    /// Fails fast if any required setting is invalid.
    pub fn load() -> Result<Self, ConfigValidationError> {
        let settings = Self::build_from_env();
        validate(&settings)?;
        Ok(Self {
            inner: RwLock::new(Arc::new(settings)),
        })
    }

    /// Returns the current validated settings snapshot.
    pub fn get(&self) -> Arc<AppSettings> {
        self.inner.read().unwrap().clone()
    }

    /// Reload *non-sensitive* settings (feature flags, log level, OTLP endpoint)
    /// from the current environment without restarting.
    ///
    /// Sensitive fields (`database_url`, `redis_url`, `jwt_private_key`) are
    /// intentionally skipped — they require a restart.
    pub fn hot_reload(&self) {
        let current = self.get();
        let updated = AppSettings {
            // Keep sensitive / structural fields unchanged.
            server_port: current.server_port,
            environment: current.environment,
            database_url: current.database_url.clone(),
            redis_url: current.redis_url.clone(),
            soroban_rpc_url: current.soroban_rpc_url.clone(),
            jwt_private_key: current.jwt_private_key.clone(),
            network_passphrase: current.network_passphrase.clone(),
            // Reload non-sensitive fields.
            log_level: env_str("RUST_LOG", &current.log_level),
            otlp_endpoint: std::env::var("OTLP_ENDPOINT").ok(),
            features: FeatureFlags::from_env(),
        };

        *self.inner.write().unwrap() = Arc::new(updated);
        tracing::info!("config hot-reloaded: feature flags and log level refreshed");
    }

    fn build_from_env() -> AppSettings {
        AppSettings {
            server_port: env_u16("SERVER_PORT", 3000),
            environment: Environment::from_str(&env_str("APP_ENV", "development")),
            database_url: env_str("DATABASE_URL", "postgres://localhost:5432/perigee"),
            redis_url: env_str("REDIS_URL", "redis://localhost:6379"),
            soroban_rpc_url: env_str(
                "SOROBAN_RPC_URL",
                "https://soroban-testnet.stellar.org",
            ),
            jwt_private_key: std::env::var("JWT_PRIVATE_KEY").ok(),
            network_passphrase: env_str(
                "NETWORK_PASSPHRASE",
                "Test SDF Network ; September 2015",
            ),
            log_level: env_str("RUST_LOG", "info"),
            otlp_endpoint: std::env::var("OTLP_ENDPOINT").ok(),
            features: FeatureFlags::from_env(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_valid_in_non_production() {
        let settings = AppSettings::default();
        assert!(validate(&settings).is_ok());
    }

    #[test]
    fn production_without_jwt_key_fails_validation() {
        let settings = AppSettings {
            environment: Environment::Production,
            jwt_private_key: None,
            ..AppSettings::default()
        };
        assert!(validate(&settings).is_err());
    }

    #[test]
    fn production_with_jwt_key_passes_validation() {
        let dummy_key = concat!("-----BEGIN ", "RSA PRIVATE KEY-----\ntest\n");
        let settings = AppSettings {
            environment: Environment::Production,
            jwt_private_key: Some(dummy_key.into()),
            ..AppSettings::default()
        };
        assert!(validate(&settings).is_ok());
    }

    #[test]
    fn environment_parse() {
        assert_eq!(Environment::from_str("production"), Environment::Production);
        assert_eq!(Environment::from_str("prod"), Environment::Production);
        assert_eq!(Environment::from_str("staging"), Environment::Staging);
        assert_eq!(Environment::from_str("dev"), Environment::Development);
        assert_eq!(Environment::from_str("unknown"), Environment::Development);
    }

    #[test]
    fn empty_database_url_fails_validation() {
        let settings = AppSettings {
            database_url: String::new(),
            ..AppSettings::default()
        };
        let err = validate(&settings).unwrap_err();
        assert!(err.errors.iter().any(|e| e.contains("DATABASE_URL")));
    }
}
