# Dynamic Fees (#667)

Utilization-based, volatility-adjusted, competitor-aware fee quotes.

- `quote(signals)` → `FeeQuote` clamped to `[min_fee_bps, max_fee_bps]`
- Competitor clamp respects `max_undercut_bps` / `max_premium_bps`

## Units and rounding (#867)

- All rates and signals are **basis points** (1 bp = 0.01%). `utilization_bps` is
  `0..=10_000`; larger values are rejected with `InvalidInput`.
- `utilization_component = utilization_weight_bps * utilization_bps / 10_000` and
  `volatility_component = volatility_weight_bps * volatility_bps / 10_000`, both
  using **integer division that truncates toward zero** (no rounding up).
- Intermediate sums are `u64` with saturating operations and are saturated to
  `u32::MAX` before any narrowing cast, so extreme weights or inputs cannot wrap
  to a small fee. The final fee is always within `[min_fee_bps, max_fee_bps]`.
- Competitor clamp: with a competitor quote `c`, the fee is held within
  `[c - max_undercut_bps, c + max_premium_bps]` (floor saturating at 0) before
  the final min/max clamp. The bounds are inclusive.
- Configuration is rejected with `InvalidConfig` when `min_fee_bps > max_fee_bps`,
  `base_fee_bps > max_fee_bps`, or `max_fee_bps > 10_000`.
