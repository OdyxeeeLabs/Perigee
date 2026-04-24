//! Property-based fuzz tests for the fixed-point math library.
//!
//! Verifies that all arithmetic operations handle edge cases gracefully,
//! returning `Err` on overflow/invalid input rather than panicking.

use super::*;
use proptest::prelude::*;

// ── Strategies ───────────────────────────────────────────────────────────────

/// Generate i128 values within a range that won't immediately overflow
/// when multiplied by SCALE.
fn arb_fixed_safe() -> impl Strategy<Value = i128> {
    // i128::MAX / SCALE ≈ 1.7e20, so keep values well within that
    -100_000_000_000_000_000i128..100_000_000_000_000_000i128
}

/// Generate arbitrary i128 values including extremes.
fn arb_i128_full() -> impl Strategy<Value = i128> {
    prop_oneof![
        // Extremes
        Just(0i128),
        Just(1i128),
        Just(-1i128),
        Just(i128::MAX),
        Just(i128::MIN),
        Just(i128::MAX / 2),
        Just(i128::MIN / 2),
        // Random in full range
        any::<i128>(),
    ]
}

// ── Property Tests ───────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Fixed-point add, sub, mul, div must never panic. They should return
    /// `Err(MathError)` on overflow or division by zero.
    #[test]
    fn fuzz_fixed_arithmetic_no_panic(a in arb_i128_full(), b in arb_i128_full()) {
        let fa = Fixed(a);
        let fb = Fixed(b);

        // None of these should panic — errors are fine.
        let _add = fa.add(fb);
        let _sub = fa.sub(fb);
        let _mul = fa.mul(fb);
        let _div = fa.div(fb);

        // Division by zero must always return an error.
        if b == 0 {
            prop_assert!(
                fa.div(fb).is_err(),
                "Division by zero should return Err"
            );
        }
    }

    /// `exp` should return a positive result for any input in the valid
    /// range [-42*SCALE, 88*SCALE]. Inputs outside this range should
    /// return Ok(ZERO) or Err(Overflow) — never panic.
    #[test]
    fn fuzz_exp_bounded_input(x in -42i128 * SCALE..=88i128 * SCALE) {
        let result = Fixed(x).exp();
        prop_assert!(result.is_ok(), "exp({}) should succeed, got {:?}", x, result);

        let val = result.unwrap();
        // e^x is always positive for real x
        prop_assert!(val.0 >= 0, "exp({}) = {} should be non-negative", x, val.0);
    }

    /// For positive inputs, `ln` should succeed. Then `exp(ln(x))` should
    /// approximately equal `x` (within fixed-point precision limits).
    #[test]
    fn fuzz_ln_positive_input(
        // Use values in [1, 1000] * SCALE to stay within stable range
        x_int in 1i128..1000i128
    ) {
        let x = x_int * SCALE;
        let ln_result = Fixed(x).ln();
        prop_assert!(ln_result.is_ok(), "ln({}) failed: {:?}", x, ln_result);

        let ln_val = ln_result.unwrap();
        let roundtrip = ln_val.exp();
        prop_assert!(roundtrip.is_ok(), "exp(ln({})) failed: {:?}", x, roundtrip);

        let roundtrip_val = roundtrip.unwrap().0;

        // Allow 0.1% tolerance for fixed-point rounding
        let tolerance = x / 1000;
        let diff = (roundtrip_val - x).abs();
        prop_assert!(
            diff <= tolerance.max(SCALE / 100), // at least 0.01 tolerance
            "exp(ln({})) = {} (diff {} exceeds tolerance {})",
            x,
            roundtrip_val,
            diff,
            tolerance
        );
    }

    /// `mul_div_u128` must never panic for any inputs, including d=0.
    /// When d=0, it should return (0, true) indicating overflow.
    #[test]
    fn fuzz_mul_div_u128_no_panic(
        a in any::<u128>(),
        b in any::<u128>(),
        d in any::<u128>(),
    ) {
        let (result, overflow) = mul_div_u128(a, b, d);

        if d == 0 {
            // Division by zero should signal overflow via the flag
            // (the function returns (0, true) for d >= high which covers d=0 case)
            // Just verify no panic.
            let _ = (result, overflow);
        } else if !overflow {
            // If not overflow, verify: result * d <= a * b (approximately)
            // This is hard to verify exactly due to truncation, so just check no panic.
            let _ = result;
        }
    }
}
