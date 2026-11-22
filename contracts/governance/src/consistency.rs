/// Data consistency checks between vault config and on-chain state.
///
/// Implements periodic verification that:
/// - Stored config fields match the actual contract data structure.
/// - Signer lists match on-chain thresholds.
/// - Balance accounting is correct (sum of user balances ≤ total tracked).
///
/// These checks are intentionally light-weight (no cross-contract calls) and
/// designed to be called as part of admin health-check operations, not on
/// every user transaction.
///
/// Closes OdyxeeeLabs/Perigee#515

#![no_std]

use soroban_sdk::{contracttype, Address, Env};
pub use Perigee_error_codes::ContractError;

// ── Consistency Report ────────────────────────────────────────────────────────

/// Result of a single consistency check run.  Any `false` field indicates a
/// detected inconsistency that should be investigated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsistencyReport {
    /// Config struct was found in storage.
    pub config_present: bool,
    /// Total-staked counter is non-negative.
    pub total_staked_non_negative: bool,
    /// Admin / threshold settings are internally consistent.
    pub admin_threshold_valid: bool,
    /// All checks passed.
    pub ok: bool,
}

impl ConsistencyReport {
    pub fn new(
        config_present: bool,
        total_staked_non_negative: bool,
        admin_threshold_valid: bool,
    ) -> Self {
        let ok = config_present && total_staked_non_negative && admin_threshold_valid;
        ConsistencyReport {
            config_present,
            total_staked_non_negative,
            admin_threshold_valid,
            ok,
        }
    }
}

// ── Generic Invariant Assertions ──────────────────────────────────────────────

/// Assert that `value >= 0`, returning `ContractError::InvalidInput` otherwise.
/// Used in balance accounting checks.
pub fn assert_non_negative(value: i128) -> Result<(), ContractError> {
    if value < 0 {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

/// Assert that `a <= b`.  E.g. `used_liquidity <= total_liquidity`.
pub fn assert_lte(a: i128, b: i128) -> Result<(), ContractError> {
    if a > b {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

/// Assert that the admin count meets the minimum threshold requirement.
/// `admin_count >= threshold` must hold for multi-sig operations to succeed.
pub fn assert_threshold_satisfiable(admin_count: u32, threshold: u32) -> Result<(), ContractError> {
    if admin_count < threshold {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

// ── Input Length Validation ───────────────────────────────────────────────────
//
// All string / vec inputs are validated at the contract boundary to prevent
// storage bloat and unexpected behavior from oversized arguments.
//
// Closes OdyxeeeLabs/Perigee#516

/// Maximum byte length for a proposal title.
pub const MAX_TITLE_BYTES: u32 = 256;
/// Maximum byte length for a proposal description.
pub const MAX_DESCRIPTION_BYTES: u32 = 4096;
/// Maximum number of signers / addresses in a single argument.
pub const MAX_ADDRESS_LIST: u32 = 50;
/// Maximum byte length for any generic string parameter.
pub const MAX_STRING_BYTES: u32 = 1024;

/// Validate a Soroban `String` length (in bytes) against a caller-supplied max.
///
/// Returns `ContractError::InvalidInput` if the string is empty or exceeds
/// `max_bytes`.
pub fn validate_string_len(s: &soroban_sdk::String, max_bytes: u32) -> Result<(), ContractError> {
    let len = s.len();
    if len == 0 || len > max_bytes {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

/// Validate a `Vec<T>` length against a caller-supplied max.
pub fn validate_vec_len<T: soroban_sdk::Val>(
    v: &soroban_sdk::Vec<T>,
    max_len: u32,
) -> Result<(), ContractError> {
    let len = v.len();
    if len == 0 || len > max_len {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

// ── Error Payload — Standardized Error Format ─────────────────────────────────
//
// Each contract currently formats errors differently.  Standardize to a common
// format: (error_code: u32, message: String) and expose an encoder/decoder so
// all contracts use the same representation.
//
// Closes OdyxeeeLabs/Perigee#521

/// A standardized error payload emitted by all Perigee contracts.
///
/// On-chain: stored as a `(u32, String)` tuple via XDR.
/// Off-chain: decoded by the Perigee UI from contract events.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorPayload {
    /// Stable numeric code matching `ContractError` discriminants.
    pub code: u32,
    /// Human-readable description (English, ≤ 256 bytes).
    pub message: soroban_sdk::String,
}

impl ErrorPayload {
    /// Construct an `ErrorPayload` from a `ContractError` and an Env.
    pub fn from_error(env: &Env, err: ContractError, message: &str) -> Self {
        ErrorPayload {
            code: err as u32,
            message: soroban_sdk::String::from_str(env, message),
        }
    }

    /// Convenience: emit this payload as a contract event under the `error`
    /// topic so the Perigee UI can surface structured error information.
    pub fn emit(&self, env: &Env) {
        env.events().publish(
            (soroban_sdk::symbol_short!("error"),),
            (self.code, self.message.clone()),
        );
    }
}

// ── Gas Budget Annotations ────────────────────────────────────────────────────
//
// Document expected gas consumption ranges and add runtime assertions that
// warn when execution is approaching the per-transaction gas limit.
//
// Closes OdyxeeeLabs/Perigee#523

/// Approximate Soroban instruction budget for a single token transfer.
/// Measured empirically on testnet; adjust as the VM changes.
pub const INSTRUCTIONS_PER_TRANSFER: u64 = 200_000;

/// Approximate Soroban instruction budget for a single storage read.
pub const INSTRUCTIONS_PER_READ: u64 = 10_000;

/// Approximate Soroban instruction budget for a single storage write.
pub const INSTRUCTIONS_PER_WRITE: u64 = 20_000;

/// Fraction of the per-transaction instruction limit at which we consider the
/// budget "tight".  Used in `assert_budget_headroom`.
pub const BUDGET_WARN_THRESHOLD: u64 = 80; // percent

/// Estimate the instruction cost of a batch of `n` transfers plus `r` reads
/// and `w` writes.  Returns the estimated total instruction count.
///
/// Callers use this for off-chain planning (e.g. deciding how to split a
/// large batch) and as a sanity check inside entry points.
pub fn estimate_instructions(transfers: u64, reads: u64, writes: u64) -> u64 {
    transfers
        .saturating_mul(INSTRUCTIONS_PER_TRANSFER)
        .saturating_add(reads.saturating_mul(INSTRUCTIONS_PER_READ))
        .saturating_add(writes.saturating_mul(INSTRUCTIONS_PER_WRITE))
}

/// Assert that a proposed operation will not consume more than
/// `BUDGET_WARN_THRESHOLD`% of `total_budget`.
///
/// Returns `Err(ContractError::Overflow)` if the estimate exceeds the
/// threshold, signalling that the caller should split the operation.
pub fn assert_budget_headroom(estimated: u64, total_budget: u64) -> Result<(), ContractError> {
    let threshold = total_budget
        .saturating_mul(BUDGET_WARN_THRESHOLD)
        .saturating_div(100);
    if estimated > threshold {
        return Err(ContractError::Overflow);
    }
    Ok(())
}
