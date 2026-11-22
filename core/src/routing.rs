//! Pure routing algorithms for picking an RPC provider by measured RTT.
//!
//! The registry (`crate::rpc_provider`) owns the actual state — this
//! module hosts the small algorithmic pieces (least-EMA selection,
//! inverse-RTT weight computation) as free functions operating on
//! caller-supplied slices, which makes them exhaustively testable
//! without spinning up a full registry.
//!
//! The production dispatch path is
//! [`crate::rpc_provider::ProviderRegistry::providers_by_latency`]; this
//! module is the algorithmic core of that method plus extra helpers
//! (weighted round-robin) that the registry may adopt later.

use crate::rpc_provider::MIN_SAMPLES_FOR_EMA;
use thiserror::Error;

/// View of one provider that the routing algorithms care about. All
/// pure functions in this module take slices of `ProviderView`, which
/// lets tests synthesise scenarios without any real RPC state.
#[derive(Debug, Clone, Copy)]
pub struct ProviderView<'a> {
    pub name: &'a str,
    pub is_healthy: bool,
    pub ema_rtt_us: u64,
    pub sample_count: u64,
}

/// Return the index (into `providers`) of the best provider to send the
/// next request to, or `None` when no provider is healthy.
///
/// - Primary strategy: pick the healthy provider with the **lowest**
///   EMA RTT. Requires every healthy provider to have reached
///   [`MIN_SAMPLES_FOR_EMA`] samples so we don't bias against providers
///   with short EMAs purely because they're new.
/// - Fallback: while any healthy provider is below the threshold, use
///   round-robin over the healthy set. `round_robin_cursor` is the
///   caller's monotonically-advancing counter (typically an
///   `AtomicUsize::fetch_add(1, …)` from the registry).
pub fn select_provider_index(
    providers: &[ProviderView<'_>],
    round_robin_cursor: usize,
) -> Option<usize> {
    let healthy: Vec<(usize, ProviderView<'_>)> = providers
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_healthy)
        .map(|(i, p)| (i, *p))
        .collect();

    if healthy.is_empty() {
        return None;
    }

    let all_warmed = healthy
        .iter()
        .all(|(_, p)| p.sample_count >= MIN_SAMPLES_FOR_EMA);

    if !all_warmed {
        // Round-robin across the healthy subset. Modulo against
        // `healthy.len()` (not `providers.len()`) so unhealthy
        // providers are genuinely skipped rather than producing a
        // no-op tick.
        let pick = round_robin_cursor % healthy.len();
        return Some(healthy[pick].0);
    }

    // Least-EMA. Ties break by original index — the first provider
    // declared in config wins on equal latency.
    healthy
        .into_iter()
        .min_by_key(|(i, p)| (p.ema_rtt_us, *i))
        .map(|(i, _)| i)
}

/// Compute weighted round-robin weights for the healthy subset of
/// `providers`. Returns one weight per healthy provider, in input order.
/// Faster providers receive higher weights: `weight = max_rtt / rtt`.
///
/// Skips unhealthy providers entirely — the output length equals the
/// count of healthy providers, so callers pairing the weights with
/// indices must track the filter themselves.
pub fn compute_inverse_rtt_weights(providers: &[ProviderView<'_>]) -> Vec<u64> {
    let rtts: Vec<u64> = providers
        .iter()
        .filter(|p| p.is_healthy)
        .map(|p| p.ema_rtt_us.max(1))
        .collect();

    if rtts.is_empty() {
        return Vec::new();
    }

    // SAFETY: we just checked that `rtts` is non-empty above, so `max()`
    // is guaranteed to return `Some`. The `unwrap_or(1)` is unreachable but
    // keeps the compiler happy without the clippy::unwrap_used lint firing.
    let max_rtt = *rtts.iter().max().unwrap_or(&1);
    rtts.into_iter().map(|r| max_rtt / r).collect()
}

// ──────────────────────────────────────────────────────────────────────────
// AMM swap-route selection
// ──────────────────────────────────────────────────────────────────────────
//
// Stellar AMM pools expose a constant-product curve.  When the strategy
// agent needs to rotate between assets it must pick the best pool from the
// set of candidate pools that connect the two assets.
//
// A pool is *eligible* only when both reserves are strictly positive.
// Dividing by a zero reserve (the constant-product formula contains
// `reserve_in` in the denominator) would cause an integer overflow or
// division-by-zero panic — this is the root cause of BE-031.
//
// Selection criterion: highest `reserve_out` among eligible pools, which
// is a reasonable proxy for depth / lowest-slippage without requiring the
// caller to supply an exact input amount.

/// Errors that can be returned by the AMM routing layer.
#[derive(Error, Debug, PartialEq)]
pub enum RoutingError {
    /// No eligible pool could be found for the requested swap. Either no
    /// pools were supplied, or every candidate pool has zero liquidity and
    /// was therefore skipped to prevent a division-by-zero panic.
    #[error("no route available: all candidate pools have zero liquidity")]
    NoRouteAvailable,
}

/// A snapshot of one AMM pool as seen by the routing algorithm.
///
/// `reserve_in` and `reserve_out` are the on-chain reserve amounts for the
/// input and output token respectively, expressed in the token's native
/// integer unit (e.g. stroops for XLM-denominated pairs).
///
/// Both values **must** be the raw, non-scaled reserves that appear in the
/// constant-product formula `Δout = reserve_out * Δin / (reserve_in + Δin)`.
#[derive(Debug, Clone, Copy)]
pub struct PoolView {
    /// Unique identifier for the pool (e.g. contract address or a numeric ID
    /// used by the caller to look up the full pool record).
    pub id: u64,
    /// Reserve of the input token held by this pool.
    pub reserve_in: u128,
    /// Reserve of the output token held by this pool.
    pub reserve_out: u128,
}

/// Return the index (into `pools`) of the best pool for a swap, or
/// [`RoutingError::NoRouteAvailable`] when no eligible pool exists.
///
/// **Zero-liquidity guard (BE-031):** any pool whose `reserve_in` **or**
/// `reserve_out` is zero is silently skipped.  Using such a pool would
/// require dividing by zero in the constant-product formula and would
/// produce a meaningless output amount.
///
/// Among the remaining eligible pools the one with the **highest
/// `reserve_out`** is returned, which minimises price impact for the
/// caller's swap.  Ties are broken by the lowest index so that behaviour
/// is deterministic.
pub fn best_pool_for_swap(pools: &[PoolView]) -> Result<usize, RoutingError> {
    let best = pools
        .iter()
        .enumerate()
        // BE-031: skip pools with zero reserves to prevent division by zero.
        .filter(|(_, p)| p.reserve_in > 0 && p.reserve_out > 0)
        .max_by_key(|(i, p)| (p.reserve_out, usize::MAX - i));

    match best {
        Some((idx, _)) => Ok(idx),
        None => Err(RoutingError::NoRouteAvailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper — build a healthy, fully-warmed provider view.
    fn warm(name: &str, ema_us: u64) -> ProviderView<'_> {
        ProviderView {
            name,
            is_healthy: true,
            ema_rtt_us: ema_us,
            sample_count: MIN_SAMPLES_FOR_EMA,
        }
    }

    fn cold(name: &str) -> ProviderView<'_> {
        ProviderView {
            name,
            is_healthy: true,
            ema_rtt_us: 0,
            sample_count: 0,
        }
    }

    fn down(name: &str, ema_us: u64) -> ProviderView<'_> {
        ProviderView {
            name,
            is_healthy: false,
            ema_rtt_us: ema_us,
            sample_count: MIN_SAMPLES_FOR_EMA,
        }
    }

    #[test]
    fn empty_pool_returns_none() {
        assert_eq!(select_provider_index(&[], 0), None);
    }

    #[test]
    fn all_unhealthy_returns_none() {
        let pool = [down("a", 50), down("b", 100)];
        assert_eq!(select_provider_index(&pool, 0), None);
    }

    #[test]
    fn select_provider_picks_fastest_healthy() {
        // slow at 500ms, fast at 50ms; fast must win on every cursor value.
        let pool = [warm("slow", 500_000), warm("fast", 50_000)];
        assert_eq!(select_provider_index(&pool, 0), Some(1));
        assert_eq!(select_provider_index(&pool, 42), Some(1));
    }

    #[test]
    fn select_provider_falls_back_to_round_robin_before_warmup() {
        // `a` has no samples; we must use round-robin across both.
        let pool = [cold("a"), warm("b", 50_000)];
        assert_eq!(select_provider_index(&pool, 0), Some(0));
        assert_eq!(select_provider_index(&pool, 1), Some(1));
        assert_eq!(select_provider_index(&pool, 2), Some(0));
    }

    #[test]
    fn round_robin_skips_unhealthy() {
        // `a` and `c` healthy, `b` unhealthy; cursor should cycle a → c → a → …
        let pool = [cold("a"), down("b", 100), cold("c")];
        assert_eq!(select_provider_index(&pool, 0), Some(0));
        assert_eq!(select_provider_index(&pool, 1), Some(2));
        assert_eq!(select_provider_index(&pool, 2), Some(0));
    }

    #[test]
    fn unhealthy_providers_are_excluded_from_ema_pick() {
        // Fast but unhealthy — must not be picked. Slow but healthy
        // is the only remaining candidate.
        let pool = [down("fast", 10_000), warm("slow", 500_000)];
        assert_eq!(select_provider_index(&pool, 0), Some(1));
    }

    #[test]
    fn ties_broken_by_lowest_index() {
        // Two providers, identical EMA — the first declared wins.
        let pool = [warm("first", 100_000), warm("second", 100_000)];
        assert_eq!(select_provider_index(&pool, 0), Some(0));
    }

    #[test]
    fn inverse_rtt_weights_favour_faster_providers() {
        // fast=50us, slow=500us → max/fast=10, max/slow=1
        let pool = [warm("slow", 500), warm("fast", 50)];
        assert_eq!(compute_inverse_rtt_weights(&pool), vec![1, 10]);
    }

    #[test]
    fn inverse_rtt_weights_skip_unhealthy() {
        let pool = [warm("a", 100), down("b", 10), warm("c", 50)];
        assert_eq!(compute_inverse_rtt_weights(&pool), vec![1, 2]);
    }

    #[test]
    fn inverse_rtt_weights_on_empty_healthy_pool_return_empty() {
        let pool = [down("a", 100), down("b", 50)];
        assert!(compute_inverse_rtt_weights(&pool).is_empty());
    }

    #[test]
    fn zero_rtt_is_clamped_to_one_in_weight_calc() {
        // A provider with an EMA of exactly zero (no samples) must not
        // divide-by-zero the weight formula; it's clamped to 1us so it
        // receives weight = max_rtt.
        let pool = [
            ProviderView {
                name: "zero",
                is_healthy: true,
                ema_rtt_us: 0,
                sample_count: MIN_SAMPLES_FOR_EMA,
            },
            warm("real", 200),
        ];
        let weights = compute_inverse_rtt_weights(&pool);
        assert_eq!(weights.len(), 2);
        assert!(weights[0] >= weights[1]);
    }

    // ── AMM swap-route tests (BE-031) ──────────────────────────────────────

    fn pool(id: u64, reserve_in: u128, reserve_out: u128) -> PoolView {
        PoolView { id, reserve_in, reserve_out }
    }

    /// A slice containing only a single pool with both reserves at zero must
    /// return `NoRouteAvailable` — the core regression for BE-031.
    #[test]
    fn single_zero_liquidity_pool_returns_no_route() {
        let pools = [pool(1, 0, 0)];
        assert_eq!(
            best_pool_for_swap(&pools),
            Err(RoutingError::NoRouteAvailable)
        );
    }

    /// An empty candidate list also has no route.
    #[test]
    fn empty_pool_list_returns_no_route() {
        assert_eq!(
            best_pool_for_swap(&[]),
            Err(RoutingError::NoRouteAvailable)
        );
    }

    /// A pool where only `reserve_in` is zero must be skipped.
    #[test]
    fn pool_with_zero_reserve_in_is_skipped() {
        // Pool 1: reserve_in=0 (ineligible). Pool 2: healthy.
        let pools = [pool(1, 0, 1_000), pool(2, 500, 800)];
        assert_eq!(best_pool_for_swap(&pools), Ok(1));
    }

    /// A pool where only `reserve_out` is zero must be skipped.
    #[test]
    fn pool_with_zero_reserve_out_is_skipped() {
        // Pool 1: reserve_out=0 (ineligible). Pool 2: healthy.
        let pools = [pool(1, 1_000, 0), pool(2, 500, 800)];
        assert_eq!(best_pool_for_swap(&pools), Ok(1));
    }

    /// When all pools have zero liquidity the function returns `NoRouteAvailable`.
    #[test]
    fn all_zero_liquidity_pools_return_no_route() {
        let pools = [pool(1, 0, 0), pool(2, 0, 0), pool(3, 0, 0)];
        assert_eq!(
            best_pool_for_swap(&pools),
            Err(RoutingError::NoRouteAvailable)
        );
    }

    /// Among eligible pools the one with the deepest `reserve_out` is picked
    /// (lower price impact for the swapper).
    #[test]
    fn best_pool_is_the_one_with_highest_reserve_out() {
        let pools = [
            pool(1, 1_000, 500),
            pool(2, 1_000, 2_000), // <-- deepest out
            pool(3, 1_000, 800),
        ];
        assert_eq!(best_pool_for_swap(&pools), Ok(1));
    }

    /// Ties on `reserve_out` are broken by the lowest index so behaviour is
    /// deterministic across repeated calls.
    #[test]
    fn ties_on_reserve_out_broken_by_lowest_index() {
        let pools = [pool(1, 500, 1_000), pool(2, 600, 1_000)];
        assert_eq!(best_pool_for_swap(&pools), Ok(0));
    }

    /// The zero-liquidity pool is skipped and the single healthy pool wins,
    /// even though the healthy pool comes after the bad one.
    #[test]
    fn zero_liquidity_pool_skipped_healthy_pool_wins() {
        let pools = [pool(1, 0, 0), pool(2, 400, 300)];
        assert_eq!(best_pool_for_swap(&pools), Ok(1));
    }
}
