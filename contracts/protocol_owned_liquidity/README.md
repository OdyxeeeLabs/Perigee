# Protocol-Owned Liquidity (#665)

Olympus-style bonding, POL deployment, and yield capture.

- `bond` / `claim_bond` with discount + vesting
- `deploy_pol` — move reserves into LP units
- `capture_yield` / `sweep_yield` — yield accounting to treasury reserves

## Quotes, expiry and slippage (#869)

- `quote_bond(deposit_value, token_price)` returns the tokens a bond would settle
  for under the current config. `bond` settles with the same pricing function, so
  quote and settlement agree for identical inputs and config.
- `bond_with_quote(.., min_tokens_out, expiry_ledger)` adds two bounds, both
  checked **before** any state is written:
  - `QuoteExpired` when `current_ledger > expiry_ledger` (a quote is valid
    *through* its expiry ledger).
  - `SlippageExceeded` when settlement would mint fewer than `min_tokens_out`
    (inclusive: equal is accepted).
- Rounding: both divisions floor, so settlement never mints more than the exact
  value. A deposit that floors to zero tokens is rejected with `InvalidAmount`
  instead of opening an empty claim.
- Bond maturity is inclusive: a bond can be claimed on `mature_ledger` itself.
