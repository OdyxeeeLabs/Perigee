# Perigee Basket Vault

Multi-asset baskets with composite share pricing, weight discipline, and
deadbanded rebalancing.

Closes [#637](https://github.com/OdyxeeeLabs/Perigee/issues/637).

A single-asset vault prices one token. A basket prices several, which raises three
problems a single-asset vault never has to answer.

## Composite share price

```text
price_share = Σ(weight_i × price_i) / 10 000
```

Each component contributes, and because the weights sum to 10 000 the result is
always expressed in the same unit regardless of how the components happen to be
priced. **That is what makes a basket share comparable across time** — a share is
worth a bundle, and the bundle's unit does not shift with its contents.

`components_for(assets)` returns the per-leg payout, each leg rounded **down**, so
the legs always sum to at most the requested amount and the rounding stays in the
basket.

## Weight discipline

- `max_weight_bps` — ceiling on any single component. Also stops a basket from
  being three wrappers of the same asset while claiming to be diversified.
- `max_assets` — ceiling on component count, at or below `MAX_BASKET_ASSETS` (16).
- A duplicate asset is rejected, so a retry cannot inflate the component count.

## Rebalancing and the deadband

`rebalance_to` is refused while every component is inside
`drift_tolerance_bps`. Without a deadband, **every** price movement of any size
would trigger a rebalance: gas burned continuously, and a small mover able to
arbitrage against the large movers through the basket.

`drift(vault, index)` reports how far a component has moved from its target weight
in basis points, and `needs_rebalance` answers the whole-basket question in one
call.

## Settlement safety

`rebalance_to` decides **every** leg before it writes **any** of them, then checks
the total movement against `Basket.total_held`. A rebalance that would move more
than the basket holds is refused with `InsufficientBasketBalance`, so it cannot
over-commit and leave later redeemers unable to exit.

`record_holding` is what makes that check meaningful: without a recorded holding,
the check compares a rebalance against zero and always fails.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the basket manager or the registry admin. |
| 4 | `BasketNotFound` | The basket does not exist. |
| 5 | `BasketExists` | Already registered. |
| 6 | `BasketFull` | Already at `max_assets`. |
| 7 | `AssetAlreadyInBasket` | The asset is already a component. |
| 8 | `EmptyBasket` | The basket has no components. |
| 9 | `ZeroAmount` | Amount, weight or price is zero or negative. |
| 10 | `WeightsDoNotSum` | Weights do not total 10 000 bps. |
| 11 | `WeightTooLarge` | A weight exceeds `max_weight_bps` or is negative. |
| 12 | `TooManyAssets` | `max_assets` exceeds `MAX_BASKET_ASSETS`. |
| 13 | `InsufficientBasketBalance` | A rebalance would move more than is held. |
| 14 | `WithinDeadband` | Drift is inside the tolerance. |
| 15 | `MathOverflow` | An arithmetic step overflowed. |
| 16 | `DivideByZero` | A division had a zero denominator. |

## Events

- `bkt(id)` → `manager`
- `comp(basket)` → `(asset, weight_bps, index)`
- `price(basket)` → `(index, price)`
- `rebal(basket)` → `rebalance_count`

## Tests

```bash
cargo test -p perigee-basket-vault
```

Covers composite pricing and its response to component price moves, share
valuation, per-leg payouts never over-committing, the weight ceiling, duplicate
rejection, drift detection and the deadband, and every rebalance rejection path
including the settlement check.