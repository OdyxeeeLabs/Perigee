# Perigee Vault Strategy

Pluggable yield strategies, atomic rotation, and performance fees.

Closes [#633](https://github.com/OdyxeeeLabs/Perigee/issues/633).

## The strategy interface

A strategy is any deployed contract implementing this trait:

```rust,ignore
pub trait IStrategy {
    fn asset(env: Env) -> Address;         // token it operates on
    fn want(env: Env) -> i128;             // assets to deploy next
    fn deploy(env: Env, amount: i128);     // put assets to work
    fn withdraw(env: Env, amount: i128);   // take assets back
    fn harvest(env: Env) -> i128;          // realise yield, return profit
    fn balance(env: Env) -> i128;          // total assets under management
}
```

`register_strategy` records the interface version, and a strategy declaring a
version other than `SUPPORTED_STRATEGY_VERSION` is rejected **at registration**
rather than failing on the first harvest — a version mismatch found halfway
through a harvest is far more expensive than one found at deployment time.

## Registration is not approval

Two separate steps, deliberately:

- `register_strategy` — records the strategy. Cheap and reversible.
- `approve_strategy` — makes it eligible to be rotated into.

Collapsing them would mean that adding a strategy, which is a routine operation,
also made it live for every vault. Keeping them apart means the set of strategies
that can touch user funds is a separate, reviewable decision.

`approve_strategy` writes a dedicated `Approved` flag that the vault entry points
read, so a rotation check is one ledger read rather than a deserialize-and-inspect.

## Rotation is atomic

`rotate` moves a vault from one strategy to another in a single call:

1. the outgoing strategy is withdrawn and harvested,
2. the incoming strategy is registered for the vault and deployed into,
3. the vault's records are updated in the same invocation.

**Atomicity is the point.** A rotation built from two transactions can withdraw,
fail before deploying, and leave the vault holding idle assets with no record of
where they were — the worst possible state, because the funds exist, the vault
cannot account for them, and nobody can tell whether they were lost.

Realised profit becomes part of the vault's value but **not** its `principal`.
That is what keeps it from being charged a performance fee twice.

## Performance fees are charged on profit, never on principal

`VaultState.principal` tracks the depositors' capital explicitly, and

```text
profit = assets_under_management - principal
```

Performance fees are charged on `profit`. Charging on the balance would take a
percentage of depositors' own money, which is the mistake that makes a fee
schedule predatory rather than merely expensive. `a_loss_is_never_charged` asserts
that a strategy which lost money produces zero fees rather than a negative one.

Two distinct fees are supported because they answer different questions:

| Fee | Charged on | Typical |
| --- | --- | --- |
| Management | assets under management | 0–2%/yr |
| Performance | realised profit | 10–20% of profit |

Both are capped at `MAX_FEE_BPS` (2 000 = 20%).

### The fee split never loses or invents value

```text
(vault, protocol) = split_fee(fee, protocol_share_bps)
```

`split_fee` computes the protocol share rounded **down** and gives the remainder
to the vault, so `vault + protocol == fee` exactly at every split point. A naive
two-sided rounding can lose a unit to nobody, or create one;
`the_fee_split_always_sums_to_the_total` asserts the invariant across the split
range including 0% and 100%.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Registry used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` or `initialize_vault` called twice. |
| 3 | `Unauthorized` | Caller is not the registry admin. |
| 4 | `StrategyNotFound` | The strategy is not registered. |
| 5 | `StrategyAlreadyRegistered` | Already registered, or rotation target is the current strategy. |
| 6 | `IncompatibleStrategyVersion` | Strategy declares an unsupported interface version. |
| 7 | `NoActiveStrategy` | The vault has no strategy assigned. |
| 8 | `StrategyNotApproved` | Target strategy is not approved for rotation. |
| 9 | `FeeTooHigh` | A fee above `MAX_FEE_BPS` was configured. |
| 10 | `InvalidFeeSplit` | Protocol share above 100%. |
| 11 | `StrategyBalanceMismatch` | The strategy reported an inconsistent balance. |
| 12 | `MathOverflow` | An arithmetic step overflowed. |
| 13 | `InsufficientPrincipal` | A withdrawal would take principal below zero. |

## Events

- `init(admin)` → `treasury`
- `strat(strategy)` → `(version, label)`
- `approve(strategy)` → `bool`
- `fee_cfg(vault)` → `(management_fee_bps, performance_fee_bps)`
- `vlt_init(vault)` → `strategy`
- `rotate(vault)` → `(from, to, withdrawn, profit, deployed)`
- `set_admin()` → `new_admin`

## Tests

```bash
cargo test -p perigee-vault-strategy
```

Covers the registry and its version gate, registration-is-not-approval, rotation
(including that realised profit never becomes principal), fee computation at the
profit boundary and the loss boundary, fee-split conservation across the split
range, and fee ceilings.