# Perigee ERC-4626 Vault

`perigee-erc4626-vault` implements the [ERC-4626](https://eips.ethereum.org/EIPS/eip-4626)
tokenised-vault interface on Soroban. Depositors receive vault **shares**; shares
redeem for the underlying Stellar Asset Contract token at the vault's current
share price.

Closes [#631](https://github.com/OdyxeeeLabs/Perigee/issues/631).

## Interface

Soroban is snake_case, so the camelCase ERC-4626 names map to Rust identifiers.

| ERC-4626 | This contract | Auth |
| --- | --- | --- |
| `asset()` | `asset()` | — |
| `totalAssets()` | `total_assets()` | — |
| `convertToShares` / `convertToAssets` | `convert_to_shares` / `convert_to_assets` | — |
| `previewDeposit` / `previewMint` | `preview_deposit` / `preview_mint` | — |
| `previewWithdraw` / `previewRedeem` | `preview_withdraw` / `preview_redeem` | — |
| `maxDeposit` / `maxMint` | `max_deposit` / `max_mint` | — |
| `maxWithdraw` / `maxRedeem` | `max_withdraw` / `max_redeem` | — |
| — | `balance_of` | — |
| `deposit(assets, receiver)` | `deposit(caller, assets, receiver)` | caller |
| `mint(shares, receiver)` | `mint(caller, shares, receiver)` | caller |
| `withdraw(assets, receiver, owner)` | `withdraw(caller, assets, receiver, owner)` | owner or approved spender |
| `redeem(shares, receiver, owner)` | `redeem(caller, shares, receiver, owner)` | owner or approved spender |
| `permit` | **not implemented** — see below | — |

`caller` is passed explicitly rather than inferred, matching the convention in
`contracts/policy_vault`: passing an address is not sufficient to act as it,
because the entry point still calls `require_auth` on it.

### Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the admin, or lacks authority over the owner's shares. |
| 4 | `Paused` | Deposits and withdrawals are halted. |
| 5 | `ZeroAmount` | Amount is zero, negative, or rounds to zero shares. |
| 6 | `MathOverflow` | An arithmetic step overflowed `i128`. |
| 7 | `DivideByZero` | A division had a zero denominator. |
| 8 | `InsufficientShares` | The account does not hold enough shares. |
| 9 | `InsufficientAllowance` | No allowance, or it does not cover the amount. |
| 10 | `AllowanceExpired` | The allowance's expiry ledger has been reached. |
| 11 | `InvalidFee` | A performance fee above 100% was configured. |

### Events

- `init(admin)` → `(asset, performance_fee_bps)`
- `deposit(receiver)` → `(caller, assets, shares)`
- `mint(receiver)` → `(caller, shares, assets)`
- `withdraw(receiver)` → `(caller, owner, assets, shares)`
- `redeem(receiver)` → `(caller, owner, shares, assets)`
- `approve(spender)` → `(owner, shares, valid_until)`
- `revoke(spender)` → `owner`
- `permit(receiver)` → `(spender, owner, assets, shares, remaining)`
- `harvest()` → `(profit, fee, new_total_assets)`
- `paused()` → `bool`
- `set_admin()`, `fee_to()`, `verifier()`

## `permit` is not implementable on Soroban — and what replaces it

ERC-4626 inherits EIP-2612 `permit`, which requires `ecrecover` on-chain.
**Soroban exposes no signature-recovery primitive to contracts.** Any
implementation here would have to either trust an off-chain relayer with the
owner's key, or accept an unverified blob and call it verified. Both are worse
than not having the function, so this contract does not have it.

What `permit` actually exists to provide is authority that is **bounded in amount
and in time**. That part is expressible on-chain, and it is what this contract
provides:

- `approve(caller, spender, shares, valid_until)` — grants a bounded delegation.
- `withdraw(caller, ...)` / `redeem(caller, ...)` — a spender may act for
  `owner` within the allowance. **The spender's own signature is still
  required**; an allowance grants scope, not identity, so an observer of the
  on-chain allowance cannot exercise it.
- The allowance is **consumed before the transfer**, so a partial failure cannot
  leave the delegation intact and reusable.
- `permit_withdraw(spender, owner, assets, receiver)` — the explicit-deadline form.
- `set_verifier(admin, kind, verifier)` — records a signature scheme the
  deployment chooses to trust. It defaults to `VerifierKind::None`, which
  refuses everything, so a deployer must **opt in** to trusting a scheme rather
  than inherit one. The choice is readable on-chain via `verifier_kind()`, so the
  trust assumption is auditable rather than implied.

There is deliberately **no way to create an allowance with no expiry**. A
permanent approval cannot be revoked if the key leaks, which is the failure mode
that makes approvals dangerous in the first place.

## Rounding

Every conversion rounds **against the caller and in favour of the vault**:

| Direction | Rounding | Effect |
| --- | --- | --- |
| `preview_deposit`, `preview_redeem` | down | caller receives slightly less than the exact ratio |
| `preview_mint`, `preview_withdraw` | up | caller pays or burns slightly more than the exact ratio |

The remainder therefore always accrues to existing holders rather than to whichever
party happened to round in its own favour, and a single depositor cannot extract
it. `previews_match_execution` asserts the previews equal what actually happens.

## First-depositor inflation

A vault with a plain `shares = assets * supply / managed` ratio can be drained by
its first depositor: mint 1 wei, donate a large amount, and the share price is now
so high that a later depositor's shares round down to zero while their assets sit
in the vault with no claim on them.

This contract applies the standard mitigation from OpenZeppelin's ERC4626 — a
virtual share/asset offset of `DECIMALS_OFFSET = 3`, so both sides of the ratio
are offset by 1 000 before it is taken.
`a_donation_cannot_steal_from_a_later_depositor` exercises exactly that attack
and asserts the victim ends up with a redeemable position.

That offset is the baseline, not the whole story. Deeper manipulation resistance,
rounding policy and dust handling are the subject of
[#636](https://github.com/OdyxeeeLabs/Perigee/issues/636).

## Accounting invariant

`total_assets` is updated by every deposit and withdrawal, and reconciled against
the vault's actual token balance by `harvest`, which is the **only** path by which
yield enters the accounting. That is what makes a harvest externally verifiable
rather than self-reported: anyone can compare `total_assets()` with the vault's
SAC balance, and `harvest` is what closes the gap.

The performance fee is charged on the **profit**, not on the balance.

## Example

```rust,ignore
initialize(asset, admin, fee_recipient, 1_000)  // 10% performance fee

admin.deposit(10_000, alice);        // 10_000 shares to alice
admin.mint(77, bob);                 // bob pays for exactly 77 shares
alice.withdraw(4_000, bob, alice);   // exact asset amount out
bob.redeem(2_500, bob, bob);         // exact share count in

alice.approve(bob, 1_000, ledger + 5_000);
bob.permit_withdraw(alice, 400, bob); // bounded, expiring delegation

admin.harvest();                     // reconcile against the real balance
```

## Tests

```bash
cargo test -p perigee-erc4626-vault
```

The suite covers the ERC-4626 acceptance surface (the four conversions, the four
operations, the max and preview helpers), rounding direction, the
first-depositor donation attack, yield and performance-fee accounting, allowance
scope and expiry, pause, admin rotation, and the verifier default.