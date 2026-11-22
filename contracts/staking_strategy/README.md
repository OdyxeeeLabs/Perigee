# Perigee Staking Strategy

Closes [#653](https://github.com/OdyxeeeLabs/Perigee/issues/653).

Liquid staking through a normalized staking-pool adapter, with reward claiming
and a configurable commission.

## Protocol boundary

Lido and Rocket Pool are EVM protocols. A Soroban contract cannot call them
directly, so this package defines a `StakingPool` adapter interface for
protocol-specific Soroban contracts. An adapter must implement token movement
and normalize protocol data into `StakingPosition`:

- `shares` — liquid staking receipt balance held for the strategy;
- `principal` — staked asset value at deposit cost;
- `accrued_rewards` — gross claimable rewards before commission.

A Lido or Rocket Pool adapter can be used only where a Soroban deployment,
compatible bridge, or other chain-specific execution layer exposes this
interface. Native EVM deployments require an EVM-side implementation.

## Operations

- The admin configures the operator, protocol label, pool adapter, staked
  asset, and commission in basis points. Commission is capped at `5_000`
  (50%) of claimed rewards.
- `stake(amount)` deposits through the adapter and returns the resulting
  position. Non-positive amounts are rejected.
- `unstake(shares)` redeems receipt shares and returns `(released, position)`.
  Unstaking more shares than the position holds is rejected.
- `claim()` claims gross rewards from the adapter, applies the configured
  commission, and returns `(gross, commission, position)`. The adapter moves
  gross rewards into the strategy account; the commission stays there for the
  treasury and the net amount is the profit the vault can account for. Calling
  `claim` with no accrued rewards is a no-op returning zeros.
- `set_commission_bps` and `set_operator` are admin-only.

Adapters must report non-negative shares, principal, and rewards. A negative
claim result is rejected with `InvalidRewards`.

Run focused tests with:

```powershell
cargo test -p perigee-staking-strategy
```
