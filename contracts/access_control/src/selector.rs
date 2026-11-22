//! Selector parsing, validation and wildcard matching.
//!
//! A **selector** is the name of a guarded function (`deposit`,
//! `vault.withdraw`, …). Selectors are stored as [`soroban_sdk::String`]
//! rather than `Symbol` for two reasons: contract entry-point names routinely
//! exceed the 9 characters `symbol_short!` allows, and `Symbol` cannot be
//! compared as a byte prefix on Wasm (`ToString for Symbol` is gated behind
//! `not(target_family = "wasm")`), which namespace wildcards need.
//!
//! Three shapes are accepted:
//!
//! | Selector     | Kind        | Matches                                     |
//! |--------------|-------------|---------------------------------------------|
//! | `deposit`    | `Exact`     | only `deposit`                              |
//! | `*`          | `Any`       | every selector                              |
//! | `vault.*`    | `Namespace` | every selector starting with `vault.`       |
//!
//! A trailing `*` is the **only** legal position for a star, and it must be the
//! last byte: `de*`, `vault.*` and the bare `*` are accepted; `de*pos`, `*x` and
//! `a*b*` are rejected with [`Error::InvalidSelector`]. That keeps the match
//! rule to "exact, universal, or starts-with" with no overlapping cases, so
//! matching needs no backtracking and stays cheap enough for a hot path.

use soroban_sdk::{Env, String};

use crate::Error;

/// Longest selector accepted, in bytes. Also bounds the internal byte buffers,
/// so every `copy_into_slice` call below is statically known to fit.
pub const MAX_SELECTOR_LEN: u32 = 64;

/// The universal wildcard.
pub const WILDCARD: &str = "*";

/// The byte that terminates a namespace pattern.
const STAR: u8 = b'*';

/// Buffer big enough for [`MAX_SELECTOR_LEN`] selector bytes.
const MAX_BYTES: usize = MAX_SELECTOR_LEN as usize;

/// How a selector string is interpreted during matching.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectorKind {
    /// A plain function name that matches only itself.
    Exact,
    /// The universal wildcard `*`, which matches every selector.
    Any,
    /// A namespace wildcard such as `vault.*`, carrying the bare prefix
    /// (`vault.`) that candidate selectors are tested against.
    Namespace(String),
}

/// Copy a [`String`] into a fixed buffer, returning the byte count.
///
/// `String::copy_into_slice` panics unless the slice length matches exactly, so
/// this helper always hands it the precise sub-slice. Callers must have already
/// rejected over-long selectors; the `n > MAX_BYTES` branch exists purely so
/// the helper can never panic if a future caller forgets.
fn to_bytes(value: &String) -> ([u8; MAX_BYTES], usize) {
    let n = value.len() as usize;
    let mut buf = [0u8; MAX_BYTES];
    if n > 0 && n <= MAX_BYTES {
        value.copy_into_slice(&mut buf[..n]);
    }
    (buf, n)
}

/// Classify `selector`, rejecting malformed input.
///
/// Length is validated before any byte is copied so an over-long selector can
/// never overflow the internal buffer.
pub fn classify(env: &Env, selector: &String) -> Result<SelectorKind, Error> {
    let len = selector.len();
    if len == 0 {
        return Err(Error::InvalidSelector);
    }
    if len > MAX_SELECTOR_LEN {
        return Err(Error::SelectorTooLong);
    }

    let (bytes, n) = to_bytes(selector);

    // Sole star in the string, and it is the whole string: the universal
    // wildcard. Handled before the generic scan so `*` is never treated as an
    // (empty) namespace prefix.
    if n == 1 && bytes[0] == STAR {
        return Ok(SelectorKind::Any);
    }

    match bytes[..n].iter().position(|b| *b == STAR) {
        // No star at all: an ordinary function name.
        None => Ok(SelectorKind::Exact),
        // The first star is also the last byte: `vault.*` or `de*`. Because
        // `position` finds the *first* star, reaching this arm already proves
        // no second star exists, so there is no separate ambiguity check here.
        Some(last) if last == n - 1 => Ok(SelectorKind::Namespace(String::from_bytes(
            env,
            &bytes[..last],
        ))),
        // Leading (`*x`) or interior (`a*b`) star: the prefix is ambiguous, so
        // refuse rather than guess.
        Some(_) => Err(Error::InvalidSelector),
    }
}

/// Whether every byte of `prefix` is also the corresponding byte of `value`.
///
/// Used to test a candidate selector against a registered namespace prefix.
/// Lengths are compared first so a longer selector can never match a shorter
/// prefix by accident.
pub fn is_prefix(prefix: &String, value: &String) -> bool {
    let p = prefix.len() as usize;
    let v = value.len() as usize;
    if p == 0 || p > v || p > MAX_BYTES || v > MAX_BYTES {
        return false;
    }
    let (pbuf, _) = to_bytes(prefix);
    let (vbuf, _) = to_bytes(value);
    pbuf[..p] == vbuf[..p]
}

/// Rebuild the full grantable pattern (`vault.` → `vault.*`) from a stored
/// namespace prefix, so permission lookups key on the same string that
/// `grant` stored.
pub fn pattern(env: &Env, prefix: &String) -> String {
    let p = prefix.len() as usize;
    debug_assert!(p <= MAX_BYTES);
    let mut buf = [0u8; MAX_BYTES + 1];
    prefix.copy_into_slice(&mut buf[..p]);
    buf[p] = STAR;
    String::from_bytes(env, &buf[..p + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(env: &Env, s: &str) -> String {
        String::from_str(env, s)
    }

    #[test]
    fn classifies_exact_any_and_namespace() {
        let env = Env::default();
        assert_eq!(
            classify(&env, &sel(&env, "deposit")),
            Ok(SelectorKind::Exact)
        );
        assert_eq!(classify(&env, &sel(&env, "*")), Ok(SelectorKind::Any));
        assert_eq!(
            classify(&env, &sel(&env, "vault.*")),
            Ok(SelectorKind::Namespace(sel(&env, "vault.")))
        );
    }

    #[test]
    fn rejects_malformed_selectors() {
        let env = Env::default();
        // A star anywhere but the final byte, and a second star, are ambiguous.
        for bad in ["", "*x", "de*pos", "*.*", "a*b", "**", "vault.**"] {
            assert_eq!(
                classify(&env, &sel(&env, bad)),
                Err(Error::InvalidSelector),
                "expected {bad} to be rejected"
            );
        }
    }

    #[test]
    fn a_trailing_star_anywhere_is_a_namespace_wildcard() {
        let env = Env::default();
        assert_eq!(
            classify(&env, &sel(&env, "de*")),
            Ok(SelectorKind::Namespace(sel(&env, "de")))
        );
        assert_eq!(
            classify(&env, &sel(&env, "vault.*")),
            Ok(SelectorKind::Namespace(sel(&env, "vault.")))
        );
    }

    #[test]
    fn rejects_over_long_selectors() {
        let env = Env::default();
        let long = "a".repeat(MAX_SELECTOR_LEN as usize + 1);
        assert_eq!(
            classify(&env, &sel(&env, &long)),
            Err(Error::SelectorTooLong)
        );
    }

    #[test]
    fn namespace_pattern_round_trips() {
        let env = Env::default();
        let prefix = sel(&env, "vault.");
        assert_eq!(pattern(&env, &prefix), sel(&env, "vault.*"));
    }

    #[test]
    fn prefix_matching_is_anchored_at_the_start() {
        let env = Env::default();
        let prefix = sel(&env, "vault.");
        assert!(is_prefix(&prefix, &sel(&env, "vault.deposit")));
        assert!(is_prefix(&prefix, &sel(&env, "vault.")));
        // Not a prefix relationship.
        assert!(!is_prefix(&prefix, &sel(&env, "deposit")));
        assert!(!is_prefix(&prefix, &sel(&env, "xvault.deposit")));
        // Shorter value than prefix can never match.
        assert!(!is_prefix(
            &sel(&env, "vault.withdraw"),
            &sel(&env, "vault.")
        ));
        // Empty prefix never matches.
        assert!(!is_prefix(&sel(&env, ""), &sel(&env, "deposit")));
    }
}
