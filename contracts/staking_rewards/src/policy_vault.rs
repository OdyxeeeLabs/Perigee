/// Append-only audit trail hashing for vault policy changes.
///
/// Every policy change to a vault is hashed into an append-only chain stored
/// in contract persistent storage.  The chain is a Merkle-like hash list:
///
/// ```text
/// hash[0] = SHA-256( genesis_seed )
/// hash[n] = SHA-256( hash[n-1] || change_descriptor )
/// ```
///
/// This provides a tamper-evident record of all policy modifications
/// verifiable by external parties without downloading full event history.
///
/// ## Storage eviction for expired vault configs (issue #520)
///
/// Expired / closed vault configurations should be evicted from storage to
/// reduce ledger footprint.  This module also exposes `evict_expired_vaults`
/// which sweeps a caller-supplied list of vault IDs and removes those whose
/// TTL has elapsed, reclaiming storage costs.
///
/// Closes OdyxeeeLabs/Perigee#519
/// Closes OdyxeeeLabs/Perigee#520

#![no_std]

use soroban_sdk::{contracttype, symbol_short, Address, BytesN, Env, String, Vec};
pub use Perigee_error_codes::ContractError;

// ── Storage Keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuditKey {
    /// Latest audit hash for the given vault address.
    TrailHead(Address),
    /// Number of recorded policy changes for the given vault address.
    TrailLength(Address),
    /// Expiry ledger for the given vault config (0 = no expiry).
    VaultExpiry(Address),
}

// ── Audit Trail Hashing ───────────────────────────────────────────────────────

/// A human-readable descriptor for a single policy change.
/// Callers supply this when calling `record_policy_change`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyChangeDescriptor {
    /// Short tag identifying the kind of change (e.g. `"add_signer"`).
    pub change_type: String,
    /// The vault whose policy was modified.
    pub vault: Address,
    /// Ledger at which the change was applied.
    pub ledger: u32,
}

/// Record a policy change in the append-only audit trail for `vault`.
///
/// Hashes the current trail head together with the `descriptor` to produce
/// the new trail head, then persists both the new head hash and the incremented
/// length counter.
///
/// Returns the new trail-head hash so callers can emit it as an event.
///
/// Closes OdyxeeeLabs/Perigee#519
pub fn record_policy_change(
    env: &Env,
    vault: &Address,
    descriptor: &PolicyChangeDescriptor,
) -> BytesN<32> {
    // Load previous head (genesis = all-zeros if first entry).
    let prev_head: BytesN<32> = env
        .storage()
        .persistent()
        .get(&AuditKey::TrailHead(vault.clone()))
        .unwrap_or_else(|| BytesN::from_array(env, &[0u8; 32]));

    // Construct the preimage: prev_head_bytes (32) + ledger_u32 (4)
    // We use SHA-256 of the concatenation via the Soroban crypto host function.
    let mut preimage = soroban_sdk::Bytes::new(env);
    preimage.append(&prev_head.into()); // 32 bytes

    // Append ledger as big-endian u32 (4 bytes)
    let ledger_bytes = descriptor.ledger.to_be_bytes();
    for b in ledger_bytes.iter() {
        preimage.push_back(*b);
    }

    // Append change_type length + bytes
    let ct_bytes = descriptor.change_type.to_xdr(env);
    preimage.append(&ct_bytes);

    let new_head: BytesN<32> = env.crypto().sha256(&preimage).into();

    // Persist the new head
    env.storage()
        .persistent()
        .set(&AuditKey::TrailHead(vault.clone()), &new_head);

    // Bump length counter
    let prev_len: u32 = env
        .storage()
        .persistent()
        .get(&AuditKey::TrailLength(vault.clone()))
        .unwrap_or(0u32);
    env.storage()
        .persistent()
        .set(&AuditKey::TrailLength(vault.clone()), &(prev_len + 1));

    // Extend TTL to keep the audit trail alive for 1 year of ledgers
    // (assume ~1 ledger / 5 s → 6 307 200 ledgers / year)
    let one_year: u32 = 6_307_200;
    env.storage().persistent().extend_ttl(
        &AuditKey::TrailHead(vault.clone()),
        one_year,
        one_year,
    );

    env.events().publish(
        (symbol_short!("aud_trail"), vault.clone()),
        new_head.clone(),
    );

    new_head
}

/// Return the current audit trail head hash for `vault`, or all-zeros if no
/// policy changes have been recorded yet.
pub fn get_trail_head(env: &Env, vault: &Address) -> BytesN<32> {
    env.storage()
        .persistent()
        .get(&AuditKey::TrailHead(vault.clone()))
        .unwrap_or_else(|| BytesN::from_array(env, &[0u8; 32]))
}

/// Return the number of policy changes recorded for `vault`.
pub fn get_trail_length(env: &Env, vault: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&AuditKey::TrailLength(vault.clone()))
        .unwrap_or(0u32)
}

// ── Storage Eviction for Expired Vault Configs ────────────────────────────────

/// Register an expiry ledger for `vault`.  After `expiry_ledger`, the vault
/// config can be swept by `evict_expired_vaults`.
pub fn set_vault_expiry(env: &Env, vault: &Address, expiry_ledger: u32) {
    env.storage()
        .persistent()
        .set(&AuditKey::VaultExpiry(vault.clone()), &expiry_ledger);
}

/// Return the registered expiry ledger for `vault` (0 if no expiry is set).
pub fn get_vault_expiry(env: &Env, vault: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&AuditKey::VaultExpiry(vault.clone()))
        .unwrap_or(0u32)
}

/// Sweep a caller-supplied list of vault addresses and remove those whose
/// expiry ledger is non-zero and in the past (≤ current ledger sequence).
///
/// Removes all audit trail and expiry keys for each expired vault, reclaiming
/// the storage rent that would otherwise continue to accrue.
///
/// Returns the number of vault configs that were evicted.
///
/// Closes OdyxeeeLabs/Perigee#520
pub fn evict_expired_vaults(env: &Env, vaults: &Vec<Address>) -> u32 {
    let current_ledger = env.ledger().sequence();
    let mut evicted: u32 = 0;

    for vault in vaults.iter() {
        let expiry: u32 = env
            .storage()
            .persistent()
            .get(&AuditKey::VaultExpiry(vault.clone()))
            .unwrap_or(0u32);

        // Only evict if an expiry has been set and it has elapsed.
        if expiry > 0 && current_ledger >= expiry {
            env.storage()
                .persistent()
                .remove(&AuditKey::TrailHead(vault.clone()));
            env.storage()
                .persistent()
                .remove(&AuditKey::TrailLength(vault.clone()));
            env.storage()
                .persistent()
                .remove(&AuditKey::VaultExpiry(vault.clone()));

            env.events().publish(
                (symbol_short!("evicted"), vault.clone()),
                current_ledger,
            );

            evicted += 1;
        }
    }

    evicted
}
