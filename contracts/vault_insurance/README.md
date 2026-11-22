# Perigee Vault Insurance

Coverage, claims and premium accounting for Nexus Mutual, InsurAce or a
self-funded reserve.

Closes [#638](https://github.com/OdyxeeeLabs/Perigee/issues/638).

## Premiums are assessed, then collected on withdrawal

Charging the premium at deposit means a depositor who leaves immediately has paid
for cover they never used, and it makes the premium indistinguishable from a
deposit fee — which is how it escapes scrutiny.

So `assess_premium` accrues by whole epoch, `deposit` banks the accrual, and
`withdraw` collects the part that has actually accrued:

```text
(premium, net) = withdraw(amount)
premium = min(accrued, amount)
net    = amount - premium
```

`a_depositor_who_leaves_at_once_pays_nothing` and
`a_premium_never_exceeds_the_withdrawal` pin both ends of that.

## The reserve must be able to back what it promises

A cover pool that has promised more than it holds is **worse than no cover**,
because depositors believe they are protected. `write` refuses unless:

```text
unclaimed_premium >= outstanding_coverage + requested_coverage
```

and `assess_coverage(insured_value)` returns exactly what a new policy would be
supportable for, so an integrator learns the limit **before** writing rather than
by having `write` revert.

## Claims cite a declared trigger

A claim is only openable against a `Trigger` the policy already declares in its
`triggers` list. That ordering matters: if claims could pick their own trigger,
any vault could claim against its own mispricing by declaring the trigger
afterwards. `a_claim_on_an_undeclared_trigger_is_refused` pins it.

Settlement is bounded twice: `payout` may not exceed `claim.amount`
(`SettlementTooLarge`) **and** may not exceed `unclaimed_premium`, so a run on the
reserve cannot be settled into negative backing.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the registry admin. |
| 4 | `NoPolicy` | No policy exists, or the policy is inactive. |
| 5 | `PolicyExists` | A policy already exists for the vault. |
| 6 | `ZeroAmount` | Amount is zero or negative. |
| 7 | `InsuredValueTooLow` | Coverage exceeds the insured value. |
| 8 | `CoverageExceedsReserve` | The reserve cannot back the new coverage. |
| 9 | `CoverageAboveCap` | Coverage above the per-policy cap. |
| 10 | `PremiumRateTooHigh` | Premium rate above 100%. |
| 11 | `ClaimAlreadyOpen` | A claim is already open for the vault. |
| 12 | `NoOpenClaim` | No open claim, or it is already settled. |
| 13 | `TriggerNotCovered` | The policy does not declare this trigger. |
| 14 | `SettlementTooLarge` | Beyond the claim, the coverage, or the reserve. |
| 15 | `PremiumAlreadyCollected` | Premium already collected for this epoch. |
| 16 | `MathOverflow` | An arithmetic step overflowed. |

## Events

- `policy(vault)` → `(provider, coverage, premium_rate_bps)`
- `cancel(vault)` → `coverage`
- `prem(vault)` → `(premium, net)`
- `fund()` → `amount`
- `claim(vault)` → `(claimant, amount)`
- `settle(vault)` → `payout`
- `claim_x(vault)` → `admin`

## Tests

```bash
cargo test -p perigee-vault-insurance
```

Covers policy terms and rejections, the reserve-backs-outstanding invariant across
multiple policies, premium accrual per epoch and collection on withdrawal, both
dust-like bounds on claims, settlement bounded by claim and reserve, and
authorization on every admin entry point.