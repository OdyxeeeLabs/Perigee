# Perigee Vault Factory and Proxy

Deterministic vault deployment and UUPS-style upgrades.

Closes [#632](https://github.com/OdyxeeeLabs/Perigee/issues/632) (clone factory)
and [#635](https://github.com/OdyxeeeLabs/Perigee/issues/635) (upgradeability).

## What EIP-1167 does and does not transfer to Soroban

EIP-1167 is a 45-byte minimal proxy. The point is **one deployed artifact, many
instances, cheap per-instance creation**. Soroban has no EVM bytecode to embed, so
the literal technique does not port. Two of the three properties do, and this
implementation gets both by different means:

| EIP-1167 property | How it is achieved here |
| --- | --- |
| One artifact, many instances | Every vault is a [`VaultProxy`] running the same tiny WASM; no per-vault custom code exists. |
| Cheap creation | The proxy stores a handful of values and never deploys anything. |
| Deterministic address (EIP-1167 + CREATE2) | `salt = sha256(owner, seed, implementation)`, fed to `env.deployer().with_current_contract(salt).deploy_v2(..)`. |

What does **not** port is transparent call delegation. An EVM minimal proxy
forwards *every* call; a Soroban proxy cannot, because the host resolves the
target contract from the invocation. So `VaultProxy` exposes its own explicit
methods rather than proxying an implementation's method set. The upgrade pointer
therefore governs the **configuration the proxy consults**, not the bytecode
executing the call — and the proxy's own WASM is fixed for the life of the
instance.

That is a real difference, and it is why `VaultProxy` is small: it holds the
upgrade-relevant state and nothing else.

## The admin allowlist

`create_vault` only deploys a WASM hash the admin has already approved via
`approve_implementation`. Without that allowlist the factory would deploy
whatever the caller named, which turns it into a deployer for arbitrary
contracts under the factory's address and reputation.

## Upgrade authorization

`upgrade_to` requires **two** independent conditions:

1. The caller must equal the recorded `upgrader` **and** must authorize the
   call. Passing the upgrader's address is not sufficient.
2. `expected_implementation` must equal the currently stored implementation.

The second is the upgrade equivalent of an optimistic-concurrency guard. If the
caller is acting on a stale view — because someone else upgraded between them
reading state and submitting — the change is refused rather than silently
clobbering theirs.

## Rollback

The outgoing implementation is retained and `rollback_to` can revert to it for
`ROLLBACK_WINDOW_LEDGERS` (17 280 ledgers ≈ 24 h) after the upgrade.

The window is bounded on purpose. Rollback is for "we shipped a bad upgrade", not
for "any past implementation is selectable forever" — an unbounded history turns
the implementation pointer into a way to re-introduce a vulnerability that was
already patched.

A successful rollback closes the window, so a second rollback cannot walk
backwards through history. `extend_rollback_window` re-affirms the current
implementation, which is only legitimate after deliberately rolling forward.

## Storage gap

`STORAGE_GAP_SLOTS` (16) instance-storage slots are reserved in the `ProxyConfig`
layout — the key-slot analogue of an EVM `uint256[N] __gap`. The reservation is
**layout-only**: no ledger entry is written for a gap slot, so it costs no rent,
but a future version can claim slots in the reserved region without moving the
existing ones. That is what keeps a layout change from reinterpreting live
entries.

`the_storage_gap_is_reserved_but_unwritten` asserts both halves: the config
exists, and no gap slot has been written.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the upgrader or the factory admin. |
| 4 | `VaultExists` | The resulting address is already registered. |
| 5 | `VaultNotFound` | No vault at the supplied index. |
| 6 | `StaleImplementation` | `expected_implementation` did not match the stored one. |
| 7 | `SameImplementation` | Already the active implementation. |
| 8 | `RollbackUnavailable` | The rollback window closed, or the target is not a recorded predecessor. |
| 9 | `NoRollbackTarget` | No predecessor has been recorded. |
| 10 | `DeterministicAddressCollision` | The salt resolved to a taken address. |
| 11 | `ImplementationNotApproved` | The WASM hash is not on the admin allowlist. |
| 12 | `Overflow` | A counter would have overflowed. |

## Events

- `init()` → `(implementation, storage_version)` — proxy
- `init(admin)` → `default_implementation` — factory
- `upgrade(new_implementation)` → `(caller, previous_implementation)`
- `rollback(target)` → `caller`
- `trip_reset()` / `paused()` — see the guard crate
- `vlt_new(proxy)` → `(owner, implementation, index)`
- `approve(wasm)` / `revoke(wasm)` → `caller`
- `set_admin()` / `set_impl()`

## Tests

```bash
cargo test -p perigee-vault-factory
```

Covers upgrade authorization, the anti-stale-write guard, the bounded rollback
window in both directions, the storage-gap reservation, the admin allowlist,
salt determinism (including that the implementation is part of the salt), and
admin rotation.

The `create_vault` deployment path needs a real uploaded WASM artifact and is
exercised by the factory's own integration harness; the authorization and registry
behaviour around it is what the unit suite covers.
## Address prediction

`predict_vault_address(owner, seed, implementation)` returns the address
`create_vault` will deploy to. It is a pure function of the factory address and
those three arguments:

- It does **not** depend on the caller, the ledger sequence, the admin, the
  default implementation or the approved-WASM list, so rotating any of those
  never moves a predicted address.
- The proxy WASM hash is **not** part of the salt. Two vaults with the same
  `(owner, seed, implementation)` therefore map to the same address regardless
  of which approved WASM is named.
- Deploying the same triple twice resolves to an address that is already taken
  and `create_vault` fails with `DeterministicAddressCollision`; use a new
  `seed` to create another vault for the same owner and implementation.
