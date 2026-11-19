# Perigee Vault Migration

Versioned, user-opted migration of vault positions between registered Soroban
vault adapters.

## Migration flow

1. The admin registers a vault's original adapter and legacy schema version.
2. Each subsequent adapter version is registered in order; versions cannot be
   skipped and registration uses an expected-version guard.
3. A vault owner explicitly opts in to the latest version.
4. The owner authorizes `migrate`, which exports their snapshot from the current
   adapter and imports it into the opted-in adapter atomically. The owner may
   cancel a pending opt-in before migration.
5. The coordinator validates the source and destination versions, owner, vault,
   asset, balances, and shares before recording the new user version.

The position adapter interface is deliberately explicit. The coordinator
requires the destination to be empty and verifies the migrated snapshot retains
the owner, vault, asset, balances, shares, and strategy; only the schema version
may change. It also provides version routing, authorization, replay protection,
state identity checks, and migration events.

## Legacy compatibility

Migration is per owner and never forced. Users without an opt-in remain on the
legacy version, and every registered prior adapter remains addressable for
legacy reads and operations. Migrating one owner's state does not erase or
change another owner's legacy state. If a newer version is registered after a
user opts in, that stale opt-in cannot migrate state; the user must opt into the
new latest version.

Adapters must retain existing state for unmigrated owners and should reject
malformed snapshots. The registry admin is trusted to register the correct
adapter for each version.

## Validation

```powershell
cargo test -p perigee-vault-migration
```
