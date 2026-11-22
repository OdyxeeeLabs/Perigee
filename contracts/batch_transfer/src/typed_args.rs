/// Type-safe argument deserialization for all contract entry points.
///
/// Currently contract arguments are decoded via generic `Symbol` / `Vec<Val>`
/// types, which defers type errors to runtime and produces opaque panics.
/// This module provides typed wrappers that:
///
///  - Validate argument types *before* executing any state changes.
///  - Return structured errors (`ContractError::InvalidInput`) on mismatch.
///  - Are `#[no_std]`-compatible and zero-allocation on the hot path.
///
/// Closes OdyxeeeLabs/Perigee#508

#![no_std]

use soroban_sdk::{Address, Env, String, Val, Vec};
pub use Perigee_error_codes::ContractError;

// ── Primitive Deserializers ───────────────────────────────────────────────────

/// Decode a `Val` as an `i128`, returning `ContractError::InvalidInput` on
/// type mismatch instead of panicking.
pub fn decode_i128(env: &Env, val: Val) -> Result<i128, ContractError> {
    i128::try_from_val(env, &val).map_err(|_| ContractError::InvalidInput)
}

/// Decode a `Val` as a `u32`.
pub fn decode_u32(env: &Env, val: Val) -> Result<u32, ContractError> {
    u32::try_from_val(env, &val).map_err(|_| ContractError::InvalidInput)
}

/// Decode a `Val` as a `bool`.
pub fn decode_bool(env: &Env, val: Val) -> Result<bool, ContractError> {
    bool::try_from_val(env, &val).map_err(|_| ContractError::InvalidInput)
}

/// Decode a `Val` as a Soroban `Address`.
pub fn decode_address(env: &Env, val: Val) -> Result<Address, ContractError> {
    Address::try_from_val(env, &val).map_err(|_| ContractError::InvalidInput)
}

/// Decode a `Val` as a Soroban `String`.
pub fn decode_string(env: &Env, val: Val) -> Result<String, ContractError> {
    String::try_from_val(env, &val).map_err(|_| ContractError::InvalidInput)
}

// ── Positional Argument Extractor ─────────────────────────────────────────────

/// Helper that extracts the n-th argument from a `Vec<Val>` argument list and
/// decodes it using a provided decoder function.
///
/// ```rust
/// let amount = typed_arg(&env, &args, 0, decode_i128)?;
/// let to     = typed_arg(&env, &args, 1, decode_address)?;
/// ```
pub fn typed_arg<T>(
    env: &Env,
    args: &Vec<Val>,
    index: u32,
    decode: impl Fn(&Env, Val) -> Result<T, ContractError>,
) -> Result<T, ContractError> {
    let val = args.get(index).ok_or(ContractError::InvalidInput)?;
    decode(env, val)
}

// ── Validated Struct Decoders ─────────────────────────────────────────────────

/// A fully typed argument set for `batch_transfer::execute`.
/// Used as the canonical "parse and validate before execute" pattern.
pub struct BatchTransferArgs {
    pub sender: Address,
    pub recipients_count: u32,
    pub total_amount: i128,
}

impl BatchTransferArgs {
    /// Validate that:
    /// - `total_amount` is positive.
    /// - `recipients_count` is within the allowed maximum (see `MAX_BATCH_SIZE`).
    pub fn validate(&self) -> Result<(), ContractError> {
        const MAX_BATCH_SIZE: u32 = 500;
        if self.total_amount <= 0 {
            return Err(ContractError::InvalidInput);
        }
        if self.recipients_count == 0 || self.recipients_count > MAX_BATCH_SIZE {
            return Err(ContractError::InvalidInput);
        }
        Ok(())
    }
}

/// A fully typed argument set for `staking_rewards::stake`.
pub struct StakeArgs {
    pub user: Address,
    pub amount: i128,
}

impl StakeArgs {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.amount <= 0 {
            return Err(ContractError::InvalidInput);
        }
        Ok(())
    }
}

/// A fully typed argument set for `governance::create_proposal`.
pub struct CreateProposalArgs {
    pub title_len: u32,
    pub description_len: u32,
    pub voting_ends_at: u32,
    pub current_ledger: u32,
}

impl CreateProposalArgs {
    pub fn validate(&self) -> Result<(), ContractError> {
        const MAX_TITLE_LEN: u32 = 256;
        const MAX_DESCRIPTION_LEN: u32 = 4096;

        if self.title_len == 0 || self.title_len > MAX_TITLE_LEN {
            return Err(ContractError::InvalidInput);
        }
        if self.description_len == 0 || self.description_len > MAX_DESCRIPTION_LEN {
            return Err(ContractError::InvalidInput);
        }
        if self.voting_ends_at <= self.current_ledger {
            return Err(ContractError::InvalidInput);
        }
        Ok(())
    }
}
