# Perigee Lending Strategy

Health-factor-guarded lending operations through a normalized Soroban market
adapter.

## Protocol boundary

Aave V3 and Compound V3 are EVM protocols. A Soroban contract cannot call those
EVM contracts directly, so this package does not claim direct interoperability.
Instead, it defines a `LendingMarket` adapter interface for protocol-specific
Soroban contracts. An adapter must implement token movement and normalize
protocol position data into `LendingPosition`.

The adapter reports:

- supplied collateral and outstanding debt in their respective token units;
- net asset value, denominated in the strategy collateral asset and including
  accrued interest;
- health factor with six decimal places (`1.0` = `1_000_000`). A position with
  no debt uses `i128::MAX`.

An Aave or Compound adapter can be used only where a Soroban deployment,
compatible bridge, or other chain-specific execution layer exposes this
interface. Native EVM market deployments require an EVM-side implementation.

## Operations

- The admin configures the operator, protocol label, adapter, collateral/debt
  assets, and minimum health factor.
- Only the configured operator can supply, withdraw, borrow, or repay.
- Borrow checks current health before calling the market and checks the
  resulting position before returning. Withdrawals and supplies are checked
  after the market call; repayments are also checked for consistent health.
- Any post-operation health-factor failure reverts the full transaction,
  including the adapter call.
- The minimum health factor must remain strictly above `1.0`.

Run focused tests with:

```powershell
cargo test -p perigee-lending-strategy
```
