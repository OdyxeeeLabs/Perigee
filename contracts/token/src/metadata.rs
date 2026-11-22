#![allow(dead_code)]
use crate::storage_types::{DataKey, TokenMetadata};
use soroban_sdk::{Env, String};

/// Maximum token name length in bytes.
pub const MAX_NAME_BYTES: u32 = 32;
/// Maximum token symbol length in bytes.
pub const MAX_SYMBOL_BYTES: u32 = 12;

fn validate_utf8_text(value: &String, max_bytes: u32, field: &str) {
    if value.is_empty() {
        panic!("{field} is empty");
    }
    if value.len() > max_bytes {
        panic!("{field} exceeds MAX_{field}_BYTES");
    }

    let mut buf = [0u8; 32];
    let n = value.len() as usize;
    value.copy_into_slice(&mut buf[..n]);
    let text = core::str::from_utf8(&buf[..n]).expect("metadata contains invalid UTF-8");
    if text.chars().any(|ch| ch.is_control()) {
        panic!("{field} contains control characters");
    }
}

/// Validate name and symbol before initialization or a metadata write.
pub fn validate_metadata(name: &String, symbol: &String) {
    validate_utf8_text(name, MAX_NAME_BYTES, "name");
    validate_utf8_text(symbol, MAX_SYMBOL_BYTES, "symbol");
}

fn load_metadata(e: &Env) -> TokenMetadata {
    e.storage().instance().get(&DataKey::Metadata).unwrap()
}

fn save_metadata(e: &Env, meta: &TokenMetadata) {
    e.storage().instance().set(&DataKey::Metadata, meta);
}

/// Write all three metadata fields in a single storage entry.
pub fn write_metadata(e: &Env, name: &String, symbol: &String, decimals: u32) {
    validate_metadata(name, symbol);
    save_metadata(
        e,
        &TokenMetadata {
            name: name.clone(),
            symbol: symbol.clone(),
            decimals,
        },
    );
}

pub fn read_decimal(e: &Env) -> u32 {
    load_metadata(e).decimals
}

pub fn read_name(e: &Env) -> String {
    load_metadata(e).name
}

pub fn read_symbol(e: &Env) -> String {
    load_metadata(e).symbol
}

// Legacy single-field writers kept for compatibility with admin set_* paths.
pub fn write_decimal(e: &Env, d: u32) {
    let mut meta = load_metadata(e);
    meta.decimals = d;
    save_metadata(e, &meta);
}

pub fn write_name(e: &Env, name: &String) {
    validate_utf8_text(name, MAX_NAME_BYTES, "name");
    let mut meta = load_metadata(e);
    meta.name = name.clone();
    save_metadata(e, &meta);
}

pub fn write_symbol(e: &Env, symbol: &String) {
    validate_utf8_text(symbol, MAX_SYMBOL_BYTES, "symbol");
    let mut meta = load_metadata(e);
    meta.symbol = symbol.clone();
    save_metadata(e, &meta);
}
