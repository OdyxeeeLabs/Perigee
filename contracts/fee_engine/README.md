# Perigee Fee Engine

Performance fees, management fees and fee discount tokens.

Closes [#661](https://github.com/OdyxeeeLabs/Perigee/issues/661),
[#662](https://github.com/OdyxeeeLabs/Perigee/issues/662) and
[#663](https://github.com/OdyxeeeLabs/Perigee/issues/663).

## The problem this solves

A vault's fee schedule is where "who is paid, out of what, and when" is decided,
and getting it wrong is how a protocol either over-charges depositors or
under-pays itself. Three mechanisms belong in one contract because they answer
to the same question and because splitting them across contracts is how fee
logic ends up inconsistent.

## Performance fees (#661)

Charged on **realised profit above a high-water mark**, never on the depositors'
principal. The mark is the highest NAV at which a fee was crystallised:

- `crystallize(vault, nav)` — if the NAV is above the mark, the profit is
  `nav − mark`, the fee is charged on it, and the mark rises to the NAV.
- If the NAV has fallen below the last crystallised value, the fee charged on
  that loss is **clawed back** from `fees_paid`, so a strategy that gives back
  its gains gives back the fee it was paid on them.

Fees are **tiered**: `perf_tiers` maps a profit size to a rate, so a large
profit is charged at a higher rate than a small one.

## Management fees (#662)

Accrue **continuously** on assets under management, not in periodic lumps:

```text
fee = aum × management_fee_bps × elapsed / (10_000 × SECONDS_PER_YEAR)
```

`accrue_management_fee` charges for the time since the last accrual and advances
`last_accrual`, so the next call charges only for the time since this one. A
vault that is deposited into and withdrawn from within a single ledger pays for
exactly the time the capital was deployed.

The fee is split between the vault's own fee recipient and the protocol treasury
by `protocol_share_bps`. The split is computed so the two shares always sum to
the fee exactly: the protocol share is rounded **down** and the remainder goes
to the vault.

## Fee discount tokens (#663)

A vault that stakes the governance token earns a **rebate** on its fees.
`discount_tiers` maps a staked amount to a rebate rate. The rebate is applied to
the total fee *before* the split, so the discount reduces what the vault pays,
not what the treasury is owed. Staking is real: `stake` pulls the governance
token into the contract and `unstake` returns it.

## What is never done

- Fees are never charged on principal. Performance is charged on profit above
  the mark; management is charged on assets under management.
- The fee split never loses or invents value: `vault_share + protocol_share ==
  net_total` at every split point.
- No fee exceeds `MAX_FEE_BPS` (20%).
- Clawing back never drives `fees_paid` negative — it saturates at zero.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called twice. |
| 3 | `Unauthorized` | Caller is not the admin. |
| 4 | `ConfigNotSet` | No fee configuration has been set. |
| 5 | `VaultNotFound` | The vault has no state record. |
| 6 | `InvalidAmount` | A non-positive amount, or a withdrawal above principal. |
| 7 | `InvalidTier` | A tier list is malformed. |
| 8 | `TooManyTiers` | More than `MAX_TIERS`. |
| 9 | `InsufficientStake` | An unstake exceeds the staked balance. |
| 10 | `MathOverflow` | An `i128` step overflowed. |
| 11 | `InvalidFeeSplit` | Protocol share above 100%. |
| 12 | `FeeTooHigh` | A fee above `MAX_FEE_BPS`. |

## Events

- `init()` → `staking_token`
- `fee_cfg()` → `(management_fee_bps, performance_fee_bps)`
- `disc_tiers()` → `count`
- `perf_tiers()` → `count`
- `stake(staker)` → `amount`
- `unstake(staker)` → `amount`

## Tests

```bash
cargo test -p perigee-fee-engine
```

Covers configuration validation, continuous management-fee accrual (including
scaling with time), high-water-mark crystallisation, fee clawback, tiered
performance rates, governance staking and tier qualification, the full
`compute_fees` breakdown (accrue → crystallise → discount → split), and the
split-conservation invariant across the fee range.
