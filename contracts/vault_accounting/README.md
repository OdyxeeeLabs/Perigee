# Perigee Vault Accounting

Share-price manipulation resistance, explicit rounding, and dust handling.

Closes [#636](https://github.com/OdyxeeeLabs/Perigee/issues/636).

A vault's share price is `total_assets / total_supply`. Three things break that
ratio, and this contract closes all three.

## 1. Share-price manipulation

A plain ratio is exploitable by the **first depositor**: mint 1 wei of shares,
then donate a large amount directly to the vault. The share price is now so high
that a later depositor's shares round down to zero — their assets sit in the vault
with no claim on them, and the attacker redeems their donation for everything.

`AccountingConfig::virtual_offset` applies the standard mitigation: both sides of
the ratio are offset by `10 ** virtual_offset` before it is taken, so
`shares_for_assets` cannot return zero for a non-zero deposit once the vault is
past its first entry.

`a_donation_cannot_zero_out_a_later_depositor` runs the attack and asserts the
victim ends up with a redeemable position.
`a_larger_offset_harder_to_exploit` shows the offset is doing the work rather than
the test passing by accident.

## 2. Rounding errors

Every conversion rounds **against the caller and in favour of the vault**, and the
choice is explicit:

| Direction | Rounding | Where |
| --- | --- | --- |
| assets → shares | down | `shares_for_assets` |
| shares → assets | down | `assets_for_shares` |
| shares → assets, exact-out | up | `shares_for_exact_assets` |
| assets → shares, exact-in | up | `assets_for_exact_shares` |

The remainder accrues to existing holders rather than to whichever party happened
to round in its own favour, and the vault is never left short on an exact-out
request.

Critically, the remainder is **banked, not merely asserted**. Every
`record_deposit` / `record_withdrawal` computes the exact value, subtracts what was
actually applied, and adds the difference to `VaultTotals::rounding_remainder`,
emitting a `round` event. `rounding_remainder` makes the accumulated dust
auditable on-chain rather than a comment asserting it is small.

## 3. Dust

Two distinct problems, handled separately:

- **Dust deposits.** An amount too small to mint one whole share would leave the
  depositor's assets in the vault with no claim on them.
  `AccountingConfig::min_shares_per_deposit` makes that an explicit rejection
  (`BelowMinShares`) rather than a silent loss.
- **Dust balances.** A holder whose position has rounded down to zero can never
  redeem it. `sweep_dust` returns those assets to the holder and records the
  sweep.

`sweep_dust` **refuses when the amount is above `dust_threshold`**
(`AboveDustThreshold`). That refusal is the point: a sweep that could move a
meaningful balance is an unbounded-drain primitive, and the threshold is what makes
exposing the entry point safe.

## Reconciliation

`reconcile(vault, reported_assets, reported_supply)` compares what an off-chain
indexer believes against what the contract recorded, returning `AccountingMismatch`
on drift. That is how a divergence between on-chain state and an off-chain mirror
is caught by a check rather than discovered in a user report.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the accounting admin. |
| 4 | `VaultNotFound` | The vault is not registered. |
| 5 | `VaultAlreadyRegistered` | Already registered. |
| 6 | `NotVaultCaller` | Caller is not the vault reporting its own totals. |
| 7 | `ZeroAmount` | Amount is zero or negative. |
| 8 | `BelowMinShares` | The deposit would mint fewer shares than the minimum. |
| 9 | `AboveDustThreshold` | The position is worth more than the dust threshold. |
| 10 | `MathOverflow` | An arithmetic step overflowed. |
| 11 | `DivideByZero` | A division had a zero denominator. |
| 12 | `AccountingMismatch` | Reported totals disagree with the record. |
| 13 | `InvalidConfig` | A configuration value is negative or out of range. |

## Events

- `vlt_reg(vault)` → `virtual_offset`
- `cfg(vault)` → `virtual_offset`
- `round(vault, kind)` → `(exact, applied, remainder, ledger)`
- `dust(vault)` → `(holder, assets)`

## Tests

```bash
cargo test -p perigee-vault-accounting
```

Covers the donation attack, offset strength, rounding direction on both sides,
the banked remainder, both dust problems including that `sweep_dust` refuses above
the threshold, reconciliation drift, and configuration validation.

## Relationship to the ERC-4626 vault

`contracts/erc4626_vault` (#631) applies a virtual offset and one-directional
rounding inline, because shipping a knowingly-inflation-exploitable vault is not
acceptable. This crate is where that logic lives properly: the offset is
configurable per vault, the remainder is banked and auditable, dust is explicit,
and there is a reconciliation check against off-chain state.