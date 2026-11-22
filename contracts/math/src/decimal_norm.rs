/// Decimal precision normalization for cross-contract math.
///
/// Different contracts use different decimal precisions:
/// - Token contracts: typically 7 decimals (Stellar standard)
/// - Staking/yield contracts: 18 decimals (Perigee `Fixed` type)
/// - LP shares: 18 decimals
/// - Fee percentages: 9 decimals
///
/// Without explicit normalization at contract boundaries, precision
/// differences silently cause severe rounding bugs.  This module
/// provides a single shared utility that all contracts use for
/// cross-contract math.
///
/// Closes OdyxeeeLabs/Perigee#512

#![no_std]

use soroban_sdk::{contract, contractimpl};
pub use Perigee_error_codes::ContractError;

// ── Decimal Constants ─────────────────────────────────────────────────────────

/// Stellar-native token decimals (SEP-0041 standard).
pub const STELLAR_DECIMALS: u32 = 7;
pub const STELLAR_SCALE: i128 = 10_000_000; // 10^7

/// Perigee internal math decimals (Fixed-point type).
pub const PERIGEE_DECIMALS: u32 = 18;
pub const PERIGEE_SCALE: i128 = 1_000_000_000_000_000_000; // 10^18

/// Common precision used for fee / rate percentages.
pub const FEE_DECIMALS: u32 = 9;
pub const FEE_SCALE: i128 = 1_000_000_000; // 10^9

// ── Normalization Functions ───────────────────────────────────────────────────

/// Convert `amount` from `from_decimals` to `to_decimals`.
///
/// Returns `ContractError::Overflow` if the conversion overflows an `i128`.
/// Returns `ContractError::InvalidInput` if either decimal count is > 38
/// (the maximum representable with `i128`).
///
/// ## Examples
///
/// ```rust
/// // 1.0 in 7-decimal token units (= 10_000_000) → 18-decimal Perigee units
/// let normalized = normalize_decimals(10_000_000, 7, 18)?;
/// assert_eq!(normalized, 1_000_000_000_000_000_000);
///
/// // 1.0 in 18-decimal → 7-decimal (truncates sub-satoshi precision)
/// let truncated = normalize_decimals(1_000_000_000_000_000_000, 18, 7)?;
/// assert_eq!(truncated, 10_000_000);
/// ```
pub fn normalize_decimals(
    amount: i128,
    from_decimals: u32,
    to_decimals: u32,
) -> Result<i128, ContractError> {
    if from_decimals > 38 || to_decimals > 38 {
        return Err(ContractError::InvalidInput);
    }

    if from_decimals == to_decimals {
        return Ok(amount);
    }

    if to_decimals > from_decimals {
        // Scale UP — multiply
        let diff = to_decimals - from_decimals;
        let factor = pow10(diff)?;
        amount.checked_mul(factor).ok_or(ContractError::Overflow)
    } else {
        // Scale DOWN — divide (truncates; callers handle rounding if needed)
        let diff = from_decimals - to_decimals;
        let factor = pow10(diff)?;
        Ok(amount / factor)
    }
}

/// Convert a Stellar token amount (7 decimals) to Perigee internal units (18 dec).
///
/// This is the most frequent cross-contract normalization needed when staking
/// or depositing LP tokens.
pub fn stellar_to_perigee(amount: i128) -> Result<i128, ContractError> {
    normalize_decimals(amount, STELLAR_DECIMALS, PERIGEE_DECIMALS)
}

/// Convert a Perigee internal amount (18 decimals) to a Stellar token amount
/// (7 decimals).  Sub-satoshi precision is truncated.
pub fn perigee_to_stellar(amount: i128) -> Result<i128, ContractError> {
    normalize_decimals(amount, PERIGEE_DECIMALS, STELLAR_DECIMALS)
}

/// Convert a Perigee internal amount (18 decimals) to a fee-rate value
/// (9 decimals).
pub fn perigee_to_fee_scale(amount: i128) -> Result<i128, ContractError> {
    normalize_decimals(amount, PERIGEE_DECIMALS, FEE_DECIMALS)
}

/// Convert a fee-rate value (9 decimals) to Perigee internal units (18 dec).
pub fn fee_scale_to_perigee(amount: i128) -> Result<i128, ContractError> {
    normalize_decimals(amount, FEE_DECIMALS, PERIGEE_DECIMALS)
}

// ── Precision Arithmetic Helpers ──────────────────────────────────────────────

/// Multiply two amounts that are in the *same* decimal precision, keeping the
/// result in that same precision.
///
/// `mul_same_precision(a, b, scale)` = `(a * b) / scale`
pub fn mul_same_precision(a: i128, b: i128, scale: i128) -> Result<i128, ContractError> {
    if scale == 0 {
        return Err(ContractError::DivisionByZero);
    }
    // Use 256-bit intermediate via u128 to avoid overflow on 18-dec values.
    let a_abs = a.unsigned_abs();
    let b_abs = b.unsigned_abs();
    let s_abs = scale.unsigned_abs();

    let (res_abs, overflow) = mul_div_u128(a_abs, b_abs, s_abs);
    if overflow || res_abs > i128::MAX as u128 {
        return Err(ContractError::Overflow);
    }
    let res = res_abs as i128;
    if (a < 0) ^ (b < 0) {
        Ok(-res)
    } else {
        Ok(res)
    }
}

// ── Internal Helpers ──────────────────────────────────────────────────────────

fn pow10(exp: u32) -> Result<i128, ContractError> {
    let mut result: i128 = 1;
    for _ in 0..exp {
        result = result.checked_mul(10).ok_or(ContractError::Overflow)?;
    }
    Ok(result)
}

fn mul_div_u128(a: u128, b: u128, d: u128) -> (u128, bool) {
    if let Some(prod) = a.checked_mul(b) {
        return (prod / d, false);
    }
    let a_low = a & 0xFFFFFFFFFFFFFFFF;
    let a_high = a >> 64;
    let b_low = b & 0xFFFFFFFFFFFFFFFF;
    let b_high = b >> 64;
    let p0 = a_low * b_low;
    let p1 = a_low * b_high;
    let p2 = a_high * b_low;
    let p3 = a_high * b_high;
    let mid = (p1 & 0xFFFFFFFFFFFFFFFF) + (p2 & 0xFFFFFFFFFFFFFFFF) + (p0 >> 64);
    let high = p3 + (p1 >> 64) + (p2 >> 64) + (mid >> 64);
    let low = (mid << 64) | (p0 & 0xFFFFFFFFFFFFFFFF);
    if high >= d {
        return (0, true);
    }
    let mut quotient = 0u128;
    let mut remainder = high;
    for i in (0..128).rev() {
        remainder = (remainder << 1) | ((low >> i) & 1);
        if remainder >= d {
            remainder -= d;
            quotient |= 1 << i;
        }
    }
    (quotient, false)
}

// ── Soroban Contract Wrapper ──────────────────────────────────────────────────
//
// Expose the normalization functions as a Soroban contract so other contracts
// can call them cross-contract.

#[contract]
pub struct DecimalNorm;

#[contractimpl]
impl DecimalNorm {
    pub fn normalize(
        _e: soroban_sdk::Env,
        amount: i128,
        from_decimals: u32,
        to_decimals: u32,
    ) -> Result<i128, ContractError> {
        normalize_decimals(amount, from_decimals, to_decimals)
    }

    pub fn stellar_to_perigee(_e: soroban_sdk::Env, amount: i128) -> Result<i128, ContractError> {
        stellar_to_perigee(amount)
    }

    pub fn perigee_to_stellar(_e: soroban_sdk::Env, amount: i128) -> Result<i128, ContractError> {
        perigee_to_stellar(amount)
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_up_7_to_18() {
        // 1.0 in 7-decimal = 10_000_000 → 18-decimal = 1_000_000_000_000_000_000
        let result = normalize_decimals(10_000_000, 7, 18).unwrap();
        assert_eq!(result, 1_000_000_000_000_000_000);
    }

    #[test]
    fn test_scale_down_18_to_7() {
        let result = normalize_decimals(1_000_000_000_000_000_000, 18, 7).unwrap();
        assert_eq!(result, 10_000_000);
    }

    #[test]
    fn test_same_decimals_is_noop() {
        let result = normalize_decimals(12345, 7, 7).unwrap();
        assert_eq!(result, 12345);
    }

    #[test]
    fn test_stellar_to_perigee_roundtrip() {
        let original: i128 = 5_000_000; // 0.5 in 7-dec
        let perigee = stellar_to_perigee(original).unwrap();
        let back = perigee_to_stellar(perigee).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn test_overflow_detection() {
        // Scaling i128::MAX up should overflow
        let result = normalize_decimals(i128::MAX, 0, 18);
        assert_eq!(result, Err(ContractError::Overflow));
    }

    #[test]
    fn test_mul_same_precision_basic() {
        // 0.5 * 0.5 = 0.25 in 18-dec
        let half = PERIGEE_SCALE / 2;
        let result = mul_same_precision(half, half, PERIGEE_SCALE).unwrap();
        assert_eq!(result, PERIGEE_SCALE / 4);
    }

    #[test]
    fn test_mul_same_precision_division_by_zero() {
        let result = mul_same_precision(100, 100, 0);
        assert_eq!(result, Err(ContractError::DivisionByZero));
    }
}
