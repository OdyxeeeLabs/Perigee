# Multi-Yield Vault

## Minimum deposit

Deposits must be at least `1,000` units of the configured deposit token. The
amount is expressed in the token's smallest indivisible units, not whole tokens;
integrators should use the token's decimals when converting a user-facing
quantity to this amount. Deposits of zero or any amount below the minimum fail
with `InvalidAmount` before the vault transfers tokens or updates its accounting.

The minimum itself and any larger amount are accepted. The current vault mints
shares 1:1 with the deposit amount, so a valid deposit mints a positive number
of shares.
