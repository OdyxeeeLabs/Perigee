/// Standalone staking rewards calculation module.
///
/// The reward calculation logic has been extracted from `lib.rs` into this
/// dedicated module so it can be:
///
///  - Tested independently without spinning up a full contract environment.
///  - Reused by other contracts (e.g. `multi_yield_vault`) without copying code.
///  - Reasoned about in isolation from distribution / transfer logic.
///
/// ## Formula
///
/// For a staker with `staked_amount` S and last-update ledger `t1`, the
/// reward accrued up to ledger `t2` is:
///
/// ```text
/// R_new = (S + R_old) * multiplier(t1, t2)  −  S
/// ```
///
/// where `multiplier` is the compounding growth factor over `[t1, t2]`:
///
/// ```text
/// multiplier = exp( integral of reward_rate(t) dt from t1 to t2 )
/// ```
///
/// The reward rate decays geometrically: `reward_rate(k) = r0 * alpha^k`
/// where `alpha = 1 − decay_rate` and `k = t − start_block`.
///
/// Closes OdyxeeeLabs/Perigee#514

use soroban_sdk::Env;
pub use Perigee_error_codes::ContractError;
use Perigee_math::Fixed;

pub const SCALE: i128 = 1_000_000_000_000_000_000; // 18 decimals

// ── Ledger Validation ─────────────────────────────────────────────────────────
//
// Time-based operations use Stellar ledger sequence numbers as the time proxy.
// These helpers ensure ledger inputs are within expected ranges and guard
// against clock-manipulation attacks (closes #513).

/// Maximum ledger number that we consider valid for time-based inputs.
/// Stellar produces ~1 ledger per 5 s; at that rate 2^31 ≈ 340 years away.
pub const MAX_LEDGER_SEQUENCE: u32 = u32::MAX / 2; // ~2.1 billion

/// Validate a ledger sequence number provided as an external argument.
/// Returns `Err(ContractError::InvalidInput)` if the value is clearly
/// out-of-range (zero, or implausibly far in the future).
///
/// Closes OdyxeeeLabs/Perigee#513
pub fn validate_ledger_sequence(seq: u32) -> Result<(), ContractError> {
    if seq == 0 {
        return Err(ContractError::InvalidInput);
    }
    if seq > MAX_LEDGER_SEQUENCE {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

/// Assert that `expiry > now`.  Used for timeouts, cooldowns and TTL fields.
pub fn require_not_expired(now: u32, expiry: u32) -> Result<(), ContractError> {
    if expiry <= now {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

/// Assert that `start <= now < end`, i.e. we are currently inside the
/// given ledger window.  Used for voting periods and time-locked operations.
pub fn require_in_window(now: u32, start: u32, end: u32) -> Result<(), ContractError> {
    if start > end {
        return Err(ContractError::InvalidInput);
    }
    if now < start || now >= end {
        return Err(ContractError::InvalidInput);
    }
    Ok(())
}

// ── Core Reward Calculation ───────────────────────────────────────────────────

/// Inputs to the reward calculation that are independent of the Soroban `Env`.
/// Decoupling from `Env` makes these functions unit-testable in pure Rust.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardParams {
    /// The fixed-point initial reward rate `r0` (18-decimal Fixed).
    pub initial_rate: Fixed,
    /// The fixed-point decay rate `d` where `alpha = 1 − d`.
    pub decay_rate: Fixed,
    /// The ledger at which rewards started accruing.
    pub start_block: u32,
}

/// Compute the compounding reward multiplier over `[t1, t2]`.
///
/// Returns `Fixed::ONE` (no growth) when `t2 <= t1`.
pub fn calculate_multiplier(
    params: &RewardParams,
    t1: u32,
    t2: u32,
) -> Result<Fixed, ContractError> {
    if t2 <= t1 {
        return Ok(Fixed::ONE);
    }

    let t_start = params.start_block;
    let t1_eff = t1.max(t_start);
    let t2_eff = t2.max(t_start);

    if t2_eff <= t1_eff {
        return Ok(Fixed::ONE);
    }

    let k1 = t1_eff - t_start;
    let k2 = t2_eff - t_start;

    if params.decay_rate.0 == 0 {
        // No decay — alpha = 1
        let elapsed = (k2 - k1) as i128;
        let elapsed_fixed = Fixed::from_int(elapsed).map_err(|_| ContractError::Overflow)?;
        let exponent = params
            .initial_rate
            .mul(elapsed_fixed)
            .map_err(|_| ContractError::Overflow)?;
        exponent.exp().map_err(|_| ContractError::Overflow)
    } else {
        // Decaying rate — alpha = 1 − d
        let alpha = Fixed::ONE
            .sub(params.decay_rate)
            .map_err(|_| ContractError::Overflow)?;

        if alpha.0 < 0 || alpha.0 > SCALE {
            return Err(ContractError::InvalidInput);
        }

        let a1 = fixed_pow_int(alpha, k1)?;
        let a2 = fixed_pow_int(alpha, k2)?;
        let diff = a1.sub(a2).map_err(|_| ContractError::Overflow)?;

        let term = params
            .initial_rate
            .mul(diff)
            .map_err(|_| ContractError::Overflow)?;

        let exponent = term
            .div(params.decay_rate)
            .map_err(|_| ContractError::Overflow)?;

        exponent.exp().map_err(|_| ContractError::Overflow)
    }
}

/// Compute new accrued rewards given the previous virtual balance and the
/// compounding multiplier over the elapsed window.
///
/// `virtual_balance_old = staked + accrued_rewards`
/// `accrued_new         = virtual_balance_old * multiplier − staked`
pub fn apply_multiplier(
    staked: i128,
    accrued: i128,
    multiplier: Fixed,
) -> Result<i128, ContractError> {
    let v_old = staked
        .checked_add(accrued)
        .ok_or(ContractError::Overflow)?;
    let v_new = mul_div(v_old, multiplier.0, SCALE).ok_or(ContractError::Overflow)?;
    v_new.checked_sub(staked).ok_or(ContractError::Overflow)
}

// ── Internal Helpers ──────────────────────────────────────────────────────────

fn fixed_pow_int(base: Fixed, mut exp: u32) -> Result<Fixed, ContractError> {
    let mut temp = base;
    let mut ans = Fixed::ONE;
    while exp > 0 {
        if exp & 1 == 1 {
            ans = ans.mul(temp).map_err(|_| ContractError::Overflow)?;
        }
        temp = temp.mul(temp).map_err(|_| ContractError::Overflow)?;
        exp >>= 1;
    }
    Ok(ans)
}

fn mul_div(a: i128, b: i128, d: i128) -> Option<i128> {
    if d == 0 {
        return None;
    }
    let a_abs = a.unsigned_abs();
    let b_abs = b.unsigned_abs();
    let d_abs = d.unsigned_abs();
    let (res_abs, overflow) = mul_div_u128(a_abs, b_abs, d_abs);
    if overflow || res_abs > (i128::MAX as u128) {
        return None;
    }
    let res = res_abs as i128;
    if (a < 0) ^ (b < 0) ^ (d < 0) {
        Some(-res)
    } else {
        Some(res)
    }
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

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_ledger_sequence_rejects_zero() {
        assert_eq!(
            validate_ledger_sequence(0),
            Err(ContractError::InvalidInput)
        );
    }

    #[test]
    fn test_validate_ledger_sequence_rejects_overflow() {
        assert_eq!(
            validate_ledger_sequence(u32::MAX),
            Err(ContractError::InvalidInput)
        );
    }

    #[test]
    fn test_validate_ledger_sequence_accepts_valid() {
        assert!(validate_ledger_sequence(1_000_000).is_ok());
        assert!(validate_ledger_sequence(MAX_LEDGER_SEQUENCE).is_ok());
    }

    #[test]
    fn test_require_not_expired_ok() {
        assert!(require_not_expired(100, 200).is_ok());
    }

    #[test]
    fn test_require_not_expired_err() {
        assert!(require_not_expired(200, 100).is_err());
        assert!(require_not_expired(200, 200).is_err());
    }

    #[test]
    fn test_calculate_multiplier_no_elapsed() {
        let params = RewardParams {
            initial_rate: Fixed(SCALE / 10),
            decay_rate: Fixed(0),
            start_block: 0,
        };
        let m = calculate_multiplier(&params, 100, 100).unwrap();
        assert_eq!(m, Fixed::ONE);
    }

    #[test]
    fn test_apply_multiplier_no_growth() {
        let m = Fixed::ONE;
        let r = apply_multiplier(1000, 0, m).unwrap();
        assert_eq!(r, 0);
    }
}
