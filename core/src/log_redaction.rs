#![allow(dead_code)]

use sha2::{Digest, Sha256};

pub struct LogRedactor {
    salt: String,
}

impl LogRedactor {
    pub fn new(salt: &str) -> Self {
        Self {
            salt: salt.to_string(),
        }
    }

    pub fn redact_vault_id(&self, vault_id: &str) -> String {
        let hash = self.hash_for_audit(vault_id);
        format!("vault:{}", &hash[..16])
    }

    pub fn redact_address(&self, address: &str) -> String {
        let hash = self.hash_for_audit(address);
        format!("addr:{}", &hash[..16])
    }

    pub fn redact_secret_key(&self, key: &str) -> String {
        let hash = self.hash_for_audit(key);
        format!("secret:{}", &hash[..16])
    }

    pub fn redact_contract_address(&self, addr: &str) -> String {
        let hash = self.hash_for_audit(addr);
        format!("contract:{}", &hash[..16])
    }

    pub fn redact_log_line(&self, log_line: &str) -> String {
        let mut result = log_line.to_string();

        let secret_keys = extract_stellar_secret_keys(&result);
        for matched in &secret_keys {
            let redacted = self.redact_secret_key(matched);
            result = result.replace(matched, &redacted);
        }

        let contract_addrs = extract_soroban_contract_addresses(&result);
        for matched in &contract_addrs {
            let redacted = self.redact_contract_address(matched);
            result = result.replace(matched, &redacted);
        }

        let vault_pattern = regex_like_pattern("vault", &result);
        for matched in vault_pattern {
            let redacted = self.redact_vault_id(&matched);
            result = result.replace(&matched, &redacted);
        }

        let addr_pattern = regex_like_pattern("addr", &result);
        for matched in addr_pattern {
            let redacted = self.redact_address(&matched);
            result = result.replace(&matched, &redacted);
        }

        result
    }

    pub fn hash_for_audit(&self, identifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.salt.as_bytes());
        hasher.update(identifier.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

pub fn redact_endpoint(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return "[REDACTED_ENDPOINT]".to_string();
    };

    if !url.username().is_empty() {
        let _ = url.set_username("redacted");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("redacted"));
    }
    if redact_sensitive_text(url.path()) != url.path() {
        let _ = url.set_path("/[REDACTED]");
    }
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

pub fn redact_sensitive_text(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let sensitive_markers = [
        "password",
        "passphrase",
        "secret",
        "token",
        "authorization",
        "api-key",
        "api_key",
        "apikey",
        "private_key",
        "jwt",
        "bearer",
        "credential",
        "access_token",
        "refresh_token",
        "cookie",
        "://",
    ];

    if sensitive_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        "[REDACTED]".to_string()
    } else {
        value.to_string()
    }
}

pub fn redact_display(value: &impl std::fmt::Display) -> String {
    redact_sensitive_text(&value.to_string())
}

fn extract_stellar_secret_keys(input: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'S' && i + 55 < chars.len() {
            let candidate: String = chars[i..=i + 55].iter().collect();
            if candidate.chars().all(|c| c.is_ascii_alphanumeric()) {
                keys.push(candidate);
                i += 56;
                continue;
            }
        }
        i += 1;
    }
    keys
}

fn extract_soroban_contract_addresses(input: &str) -> Vec<String> {
    let mut addrs = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'C' && i + 55 < chars.len() {
            let candidate: String = chars[i..=i + 55].iter().collect();
            if candidate.chars().all(|c| c.is_ascii_alphanumeric()) {
                addrs.push(candidate);
                i += 56;
                continue;
            }
        }
        i += 1;
    }
    addrs
}

fn regex_like_pattern(_prefix: &str, _input: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_vault_id() {
        let redactor = LogRedactor::new("test_salt");
        let redacted = redactor.redact_vault_id("vault_abc123");
        assert!(redacted.starts_with("vault:"));
        assert_ne!(redacted, "vault_abc123");
    }

    #[test]
    fn test_redact_address() {
        let redactor = LogRedactor::new("test_salt");
        let redacted = redactor.redact_address("GAEXAMPLEADDRESS123");
        assert!(redacted.starts_with("addr:"));
    }

    #[test]
    fn test_hash_consistency() {
        let redactor = LogRedactor::new("salt");
        let h1 = redactor.hash_for_audit("test");
        let h2 = redactor.hash_for_audit("test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_different_salt_different_hash() {
        let r1 = LogRedactor::new("salt1");
        let r2 = LogRedactor::new("salt2");
        assert_ne!(r1.hash_for_audit("test"), r2.hash_for_audit("test"));
    }

    #[test]
    fn test_redact_stellar_secret_key() {
        let redactor = LogRedactor::new("test_salt");
        let secret = "SABCDEFGHIJKLMNOPQRSTUVWXYZ23456789abcdefghijklmnopqrstu";
        assert_eq!(secret.len(), 56);
        let log_line = format!("Found key: {} in logs", secret);
        let redacted = redactor.redact_log_line(&log_line);
        assert!(!redacted.contains(secret));
        assert!(redacted.contains("secret:"));
    }

    #[test]
    fn test_redact_soroban_contract_address() {
        let redactor = LogRedactor::new("test_salt");
        let contract = "CABCDEFGHIJKLMNOPQRSTUVWXYZ23456789abcdefghijklmnopqrstu";
        assert_eq!(contract.len(), 56);
        let log_line = format!("Contract: {} deployed", contract);
        let redacted = redactor.redact_log_line(&log_line);
        assert!(!redacted.contains(contract));
        assert!(redacted.contains("contract:"));
    }

    #[test]
    fn test_does_not_redact_public_keys() {
        let redactor = LogRedactor::new("test_salt");
        let pubkey = "GABCDEFGHIJKLMNOPQRSTUVWXYZ23456789abcdefghijklmnopqrstu";
        assert_eq!(pubkey.len(), 56);
        let log_line = format!("Public key: {}", pubkey);
        let redacted = redactor.redact_log_line(&log_line);
        assert!(redacted.contains(pubkey));
    }

    #[test]
    fn test_extract_stellar_secret_keys() {
        let input = "key=SABCDEFGHIJKLMNOPQRSTUVWXYZ23456789abcdefghijklmnopqrstu found";
        let keys = extract_stellar_secret_keys(input);
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_extract_soroban_contract_addresses() {
        let input = "contract=CABCDEFGHIJKLMNOPQRSTUVWXYZ23456789abcdefghijklmnopqrstu deployed";
        let addrs = extract_soroban_contract_addresses(input);
        assert_eq!(addrs.len(), 1);
    }
}
