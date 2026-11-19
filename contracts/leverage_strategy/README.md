# Perigee Leverage Strategy

Closes [#654](https://github.com/OdyxeeeLabs/Perigee/issues/654).

Leveraged looping through a normalized credit-market adapter, with loan-to-value
enforcement and liquidation protection.

## Protocol boundary

Gearbox and Notional are EVM protocols. A Soroban contract cannot call them
directly, so this package defines a `LeverageMarket` adapter interface for
protocol-specific Soroban contracts. An adapter must implement token movement
and normalize credit-account data into `CreditAccount`:

- `collateral` and `debt` in their respective token units;
- `net_asset_value` — collateral minus debt;
- `ltv_bps` — debt-to-collateral-value ratio in basis points (`10_000` = 100%);
- `liquidation_threshold_bps` — the protocol's liquidation threshold in basis
  points, or `0` if the adapter does not expose one.

A Gearbox or Notional adapter can be used only where a Soroban deployment,
compatible bridge, or other chain-specific execution layer exposes this
interface. Native EVM deployments require an EVM-side implementation.

## Operations

- The admin configures the operator, protocol label, market adapter,
  collateral/debt assets, and maximum LTV. The maximum must be within
  `1..=9_999` basis points, leaving room below liquidation.
- `loop_position(collateral_amount, borrow_amount)` deposits collateral and
  borrows in one adapter call. The resulting LTV is checked after the call and
  the transaction reverts if it exceeds the maximum or reaches the reported
  liquidation threshold.
- `deleverage(repay_amount)` repays debt to improve position health and
  re-checks the risk limits.
- `withdraw_collateral(amount)` withdraws collateral and reverts with
  `MaxLtvExceeded` if the withdrawal would push the position over the limit.
- `set_max_ltv_bps` and `set_operator` are admin-only.

Adapters must report non-negative collateral and debt, an LTV within
`0..=10_000` basis points, and a liquidation threshold within the same range.
A position with negative net value (or zero equity while carrying debt) is
rejected as `Insolvent`.

## Tests

Run focused tests with:

```powershell
cargo test -p perigee-leverage-strategy
```
