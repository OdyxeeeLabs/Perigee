# Perigee Options Strategy

Closes [#655](https://github.com/OdyxeeeLabs/Perigee/issues/655).

Covered calls, protective puts, and delta hedging through a normalized
options-market adapter.

## Protocol boundary

Lyra and Dopex are EVM protocols. A Soroban contract cannot call them
directly, so this package defines an `OptionsMarket` adapter interface for
protocol-specific Soroban contracts. An adapter must implement token movement,
collateral, settlement, and normalize option data into `OptionPosition` and
`OptionsAccount`:

- `OptionPosition` — option kind, amount, strike, expiry, premium, and
  position delta in basis points;
- `OptionsAccount` — underlying balance, short-call and long-put exposure, and
  net account delta in basis points (`5_000` = half of the underlying
  notional).

A Lyra or Dopex adapter can be used only where a Soroban deployment, compatible
bridge, or other chain-specific execution layer exposes this interface. Native
EVM deployments require an EVM-side implementation.

## Operations

- The admin configures the operator, protocol label, market adapter,
  underlying/strike assets, and the absolute delta band in basis points.
- `write_covered_call(amount, strike_price, expiry)` writes calls only up to
  the current underlying balance, so every call is fully covered. A call must
  not be expired at execution time.
- `buy_protective_put(amount, strike_price, expiry, premium)` buys downside
  protection. Premiums must be non-negative.
- `settle(kind, amount)` settles expiring options and returns the realized
  profit or loss reported by the adapter.
- `hedge(target_delta_bps)` rebalances the account delta to a target inside
  the configured band. Targets outside the band are rejected with
  `DeltaTooHigh`.
- Every option operation re-reads the account and reverts with
  `DeltaTooHigh` if the net delta leaves `[-max_abs_delta_bps,
  max_abs_delta_bps]`, so the operator cannot leave unhedged exposure behind.
- `set_max_abs_delta_bps` and `set_operator` are admin-only.

Adapters must report non-negative underlying and option exposure, and a delta
within `-10_000..=10_000` basis points; inconsistent adapter data is rejected
with `InvalidDelta` or `InvalidPosition`.

## Tests

Run focused tests with:

```powershell
cargo test -p perigee-options-strategy
```
