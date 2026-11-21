# Perigee DEX LP Strategy

Closes [#652](https://github.com/OdyxeeeLabs/Perigee/issues/652).

Concentrated-liquidity market making with range rebalancing and an
impermanent-loss guard.

## Protocol boundary

Uniswap V3 is an EVM protocol. A Soroban contract cannot call it directly, so
this package defines a `LiquidityPool` adapter interface for protocol-specific
Soroban contracts. An adapter must implement token movement, tick alignment,
and normalize pool data into `LiquidityPosition`:

- `liquidity` and the underlying `amount0` / `amount1` backing the position;
- the active tick range (`tick_lower`, `tick_upper`);
- `net_asset_value` denominated in the strategy's quote asset;
- `impermanent_loss_bps`, the loss relative to holding both tokens, normalized
  to basis points (`10_000` = 100%).

A Uniswap V3 adapter can be used only where a Soroban deployment, compatible
bridge, or other chain-specific execution layer exposes this interface. Native
EVM pool deployments require an EVM-side implementation.

## Operations

- The admin configures the operator, protocol label, pool adapter, token pair,
  fee tier, and maximum tolerated impermanent loss in basis points.
- `deposit(amount_a, amount_b, tick_lower, tick_upper)` mints a concentrated
  position through the adapter. Tick bounds are validated against the Uniswap
  V3 range (`-887_272` to `887_272`) and must be strictly increasing.
- `withdraw(liquidity)` burns liquidity and returns the amounts released by the
  adapter. Withdrawals are never blocked by the loss guard, so providers can
  always exit.
- `rebalance(tick_lower, tick_upper)` moves an existing position to a new
  range. Rebalancing is rejected with `ImpermanentLossTooHigh` once the
  adapter-reported loss exceeds the configured tolerance. This prevents the
  strategy from rotating a position that is already beyond its risk limit; the
  operator must withdraw instead.
- `set_max_impermanent_loss_bps` and `set_operator` are admin-only. The
  tolerance must be within `1..=10_000` basis points.

The fee tier uses the Uniswap convention of hundredths of a bip
(`500` = 0.05%, `3_000` = 0.30%) and must be in `1..=1_000_000`.

## Tests

Run focused tests with:

```powershell
cargo test -p perigee-dex-lp-strategy
```
