# Policy Vault — vault provisioning with per-manager rate limiting

`perigee-policy-vault` is the Soroban contract that provisions policy vaults for
wealth managers. Vault creation is a state-changing, resource-consuming
operation, so unthrottled provisioning is a griefing vector: a single manager
(or a compromised manager key) can spam `create_vault` and exhaust ledger
footprint, storage rent and downstream indexer capacity.

This contract implements **per-manager, per-epoch rate limiting**
([CONTRACT-11 / #505](https://github.com/OdyxeeeLabs/Perigee/issues/505)):

- Each manager has an independent counter.
- The counter is capped by `max_vaults_per_epoch` (default in tests: **10**).
- The counter resets every `epoch_length_ledgers` ledgers (default in tests:
  **1 000**).
- Both dimensions are configurable by the admin at runtime.

## Model

An **epoch** is a fixed-width window of ledgers, derived deterministically from
the ledger sequence:

```text
epoch(ledger) = ledger / epoch_length_ledgers
```

Because the epoch is a pure function of the ledger, every caller computes the
same window — there is no "first caller resets the timer" race. A manager may
create up to `max_vaults_per_epoch` vaults per epoch; once the ledger crosses
the boundary the counter rolls over to zero automatically, with no admin action
required.

`(manager, vault_id)` pairs are unique. A duplicate is rejected *before* the
quota is consumed, so retries are idempotent and never silently burn a
manager's budget.

## Interface

| Function | Auth | Description |
| --- | --- | --- |
| `initialize(admin, max_vaults_per_epoch, epoch_length_ledgers)` | `admin` | One-shot setup. Both config values must be non-zero. |
| `set_rate_limit(admin, max_vaults_per_epoch, epoch_length_ledgers)` | admin | Retune the limit at runtime. |
| `set_admin(admin, new_admin)` | admin | Rotate the admin key without touching counters. |
| `create_vault(manager, vault_id)` | `manager` | Provision a vault, consuming one unit of the manager's epoch quota. |
| `get_admin()` | – | Current admin address. |
| `get_config()` | – | Active `RateLimitConfig`. |
| `current_epoch()` | – | Epoch index for the current ledger. |
| `vaults_this_epoch(manager)` | – | Vaults the manager created in the current epoch. |
| `total_vaults(manager)` | – | Lifetime vaults created by the manager. |
| `remaining_quota(manager)` | – | Vaults the manager may still create this epoch. |
| `get_vault(manager, vault_id)` | – | `VaultRecord` for the pair, if any. |

### Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `AlreadyInitialized` | `initialize` called more than once. |
| 2 | `NotInitialized` | Contract used before `initialize`. |
| 3 | `Unauthorized` | The address passed to an admin entry point is not the stored admin. |
| 4 | `InvalidConfig` | Zero limit or zero-length epoch. |
| 5 | `RateLimitExceeded` | Manager's per-epoch quota is exhausted. |
| 6 | `VaultAlreadyExists` | `(manager, vault_id)` already registered. |
| 7 | `Overflow` | Lifetime counter overflowed. |

### Events

- `init(topics: [init, admin])` → `(max_vaults_per_epoch, epoch_length_ledgers)`
- `rate_cfg(topics: [rate_cfg])` → `(max_vaults_per_epoch, epoch_length_ledgers)`
- `set_admin(topics: [set_admin])` → `new_admin`
- `vlt_new(topics: [vlt_new, manager, vault_id])` → `(ledger, epoch, count)`

## Example

```text
initialize(admin, 10, 1000)          # 10 vaults per manager per 1000 ledgers
create_vault(alice, 1)               # ok   — alice: 1/10
create_vault(alice, 2)               # ok   — alice: 2/10
create_vault(bob,   1)               # ok   — bob:   1/10 (independent)
# ... 8 more vaults by alice in this epoch ...
create_vault(alice, 11)              # Err(RateLimitExceeded)
# 1000 ledgers later ...
create_vault(alice, 11)              # ok   — new epoch, quota refreshed
```

## Tests

```bash
cargo test -p perigee-policy-vault
```

The suite covers per-manager isolation, epoch rollover, runtime reconfiguration,
duplicate rejection without quota spend, access control, event emission and
uninitialized-state behaviour.
