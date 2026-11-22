//! # Versioned persisted-query registry (API-11)
//!
//! Approved queries are loaded from a JSON registry instead of being registered
//! in code:
//!
//! ```json
//! { "version": 3, "queries": [ { "hash": "<sha256 hex>", "query": "{ me { id } }" } ] }
//! ```
//!
//! Every entry is validated (non-empty, well-formed hash, hash equals the
//! SHA-256 of the text, no duplicate hashes) and the whole registry is rejected
//! if any entry is bad, so a failed rollout keeps the previous allowlist. The
//! loader only rebuilds when `version` changes; reverting is publishing the
//! earlier file with a higher version. Allowlist enforcement stays in
//! [`PersistedQueries`].

use crate::graphql_gateway::{hash_query, PersistedMode, PersistedQueries};
use serde::Deserialize;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Deserialize)]
pub struct Registry {
    pub version: u64,
    pub queries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub hash: String,
    pub query: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("registry is not valid JSON: {0}")]
    Malformed(String),
    #[error("registry entry {0} is invalid")]
    InvalidEntry(usize),
    #[error("duplicate hash in registry entry {0}")]
    DuplicateHash(usize),
    #[error("hash does not match query text in registry entry {0}")]
    HashMismatch(usize),
}

pub fn parse_registry(source: &str) -> Result<Registry, RegistryError> {
    let registry: Registry = serde_json::from_str(source).map_err(|e| RegistryError::Malformed(e.to_string()))?;
    let mut seen = HashSet::new();
    for (i, entry) in registry.queries.iter().enumerate() {
        let hash_ok = entry.hash.len() == 64 && entry.hash.chars().all(|c| c.is_ascii_hexdigit());
        if entry.query.trim().is_empty() || !hash_ok {
            return Err(RegistryError::InvalidEntry(i));
        }
        if hash_query(&entry.query) != entry.hash.to_ascii_lowercase() {
            return Err(RegistryError::HashMismatch(i));
        }
        if !seen.insert(entry.hash.to_ascii_lowercase()) {
            return Err(RegistryError::DuplicateHash(i));
        }
    }
    Ok(registry)
}

/// Tracks the loaded version so unchanged registries are not rebuilt.
#[derive(Debug, Default)]
pub struct RegistryLoader {
    pub loaded_version: Option<u64>,
}

impl RegistryLoader {
    /// `Ok(Some(store))` when a new version was loaded (assign it to
    /// `Gateway::persisted`), `Ok(None)` when the version is unchanged, `Err`
    /// when the registry is invalid (keep serving the current store).
    pub fn reload(&mut self, source: &str, mode: PersistedMode) -> Result<Option<PersistedQueries>, RegistryError> {
        let registry = parse_registry(source)?;
        if self.loaded_version == Some(registry.version) {
            return Ok(None);
        }
        let mut store = PersistedQueries::new(mode);
        for entry in &registry.queries {
            store.register(&entry.query);
        }
        self.loaded_version = Some(registry.version);
        Ok(Some(store))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql_gateway::{GatewayError, GraphqlRequest};
    use serde_json::json;

    fn registry(version: u64, queries: &[&str]) -> String {
        let entries: Vec<_> = queries.iter().map(|q| json!({"hash": hash_query(q), "query": q})).collect();
        json!({"version": version, "queries": entries}).to_string()
    }

    #[test]
    fn loads_valid_registries_and_reloads_only_on_version_change() {
        let mut loader = RegistryLoader::default();
        let store = loader.reload(&registry(1, &["{ me { id } }"]), PersistedMode::AllowlistOnly).unwrap().unwrap();
        let known = GraphqlRequest { persisted_hash: Some(hash_query("{ me { id } }")), ..Default::default() };
        assert_eq!(store.resolve(&known).unwrap(), "{ me { id } }");
        let unknown = GraphqlRequest { persisted_hash: Some("00".repeat(32)), ..Default::default() };
        assert_eq!(store.resolve(&unknown), Err(GatewayError::PersistedQueryNotFound));

        assert!(loader.reload(&registry(1, &["{ me { id } }"]), PersistedMode::AllowlistOnly).unwrap().is_none());
        assert!(loader.reload(&registry(2, &["{ a }"]), PersistedMode::AllowlistOnly).unwrap().is_some());
    }

    #[test]
    fn rejects_malformed_duplicate_and_mismatched_entries() {
        assert!(matches!(parse_registry("nope"), Err(RegistryError::Malformed(_))));
        let bad_hash = json!({"version": 1, "queries": [{"hash": "abc", "query": "{ a }"}]}).to_string();
        assert_eq!(parse_registry(&bad_hash).unwrap_err(), RegistryError::InvalidEntry(0));
        let mismatch = json!({"version": 1, "queries": [{"hash": "00".repeat(32), "query": "{ a }"}]}).to_string();
        assert_eq!(parse_registry(&mismatch).unwrap_err(), RegistryError::HashMismatch(0));
        assert_eq!(parse_registry(&registry(1, &["{ a }", "{ a }"])).unwrap_err(), RegistryError::DuplicateHash(1));
    }
}
