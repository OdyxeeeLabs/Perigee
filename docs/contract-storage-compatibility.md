# Contract storage compatibility

This document records the storage keys that survive a WASM upgrade and the
rules an upgrade must follow. It is the reference for the compatibility tests
in `contracts/vault_factory/src/test.rs` and
`contracts/vault_migration/src/test.rs`.

## Policy

1. Existing keys are never deleted or reinterpreted by an upgrade. New fields
   are appended inside the reserved gap and the layout version is bumped.
2. A contract refuses to read a config whose stored version it does not
   recognise. The refusal is deterministic (`UnsupportedStorageVersion` for
   `VaultProxy`) and happens before any state is written, so a bad upgrade
   cannot partially change a vault.
3. Migrations are explicit: they read the old layout, write the new layout with
   a new version, and preserve balances and authorization state in the same
   invocation. There is no implicit "best guess" decoding.
4. `PROXY_STORAGE_VERSION` is currently `1`. A change to `ProxyConfig` field
   order or width requires a migration and a version bump.

## `VaultFactory` keys

| Key | Scope | Value |
| --- | --- | --- |
| `FactoryKey::Admin` | instance | Factory admin address |
| `FactoryKey::DefaultImplementation` | instance | Default proxy implementation |
| `FactoryKey::VaultCount` | instance | Number of vaults created |
| `FactoryKey::Vault(n)` | persistent | `VaultRecord` for index `n` |
| `FactoryKey::VaultSeed(n)` | persistent | Seed used for vault `n` |
| `FactoryKey::ApprovedWasm(hash)` | instance | Whether a WASM hash may be deployed |

## `VaultProxy` keys

| Key | Scope | Value |
| --- | --- | --- |
| `ProxyKey::Config` | instance | `ProxyConfig` (version, upgrader, implementation, rollback target, ledger, flag) |
| `ProxyKey::Gap(0..STORAGE_GAP_SLOTS)` | instance | Reserved; never written by version 1 |

`ProxyConfig` field order is part of the layout. `STORAGE_GAP_SLOTS` is 16 and
the reserved slots are layout-only (no ledger entry), so claiming one for a new
field does not move existing fields.

## `VaultMigration` keys

| Key | Scope | Value |
| --- | --- | --- |
| `DataKey::Admin` | instance | Registry admin address |
| `DataKey::Vault(vault_id)` | persistent | `VaultConfig` (legacy/current versions) |
| `DataKey::Adapter(vault_id, version)` | persistent | Adapter address for one version |
| `DataKey::UserVersion(vault_id, owner)` | persistent | Last version a user migrated to |
| `DataKey::OptIn(vault_id, owner)` | persistent | Pending migration opt-in |

Balances and share totals remain in the adapter contracts. A migration must
compare the exported snapshot with the destination state before writing the
user's version, and refuses when the destination is already initialized.

## Testing expectation

For every layout change, tests must cover:

- a representative prior-state fixture that reads correctly under the current
  binary, with balances and authorization state preserved;
- an unknown or unsupported version that is rejected before any write; and
- a successful migration (or an intentional, documented rejection) that leaves
  no partially migrated state.
