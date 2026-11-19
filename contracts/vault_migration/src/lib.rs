#![no_std]

//! User-opted migration of vault positions between versioned vault adapters.
//!
//! Migration adapters expose a common state snapshot interface. The coordinator
//! copies one owner's position from a retained legacy/current adapter into the
//! next adapter in one atomic invocation. It never forces a migration: users
//! who do not opt in continue to use their existing vault version.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

/// Ledgers that a persistent migration record remains live after an access.
pub const TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    VaultAlreadyRegistered = 4,
    VaultNotFound = 5,
    VersionAlreadyRegistered = 6,
    VersionNotNext = 7,
    StaleVersion = 8,
    InvalidSnapshot = 9,
    MigrationNotOptedIn = 10,
    NoMigrationRequired = 11,
    OptInOutdated = 12,
    InvalidTargetVersion = 13,
    DestinationAlreadyInitialized = 14,
}

/// State needed to migrate one owner's position in one vault.
///
/// A migration must preserve all position values; only the schema version may
/// change as the state is copied between adapters.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultSnapshot {
    pub vault_id: u32,
    pub owner: Address,
    pub schema_version: u32,
    pub asset: Address,
    pub assets: i128,
    pub shares: i128,
    pub strategy: Option<Address>,
}

/// Version and opt-in status for a particular owner in a vault.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationStatus {
    pub current_version: u32,
    pub opted_in_version: Option<u32>,
}

/// Common interface implemented by a versioned vault or a legacy adapter.
///
/// The coordinator only calls adapters explicitly registered by the admin.
/// `import_state` must validate and store the snapshot for the supplied owner.
pub trait VaultMigrationAdapter {
    fn export_state(env: Env, vault_id: u32, owner: Address) -> VaultSnapshot;
    fn has_state(env: Env, vault_id: u32, owner: Address) -> bool;
    fn import_state(
        env: Env,
        vault_id: u32,
        owner: Address,
        snapshot: VaultSnapshot,
    );
}

soroban_sdk::contractclient!(name = "VaultMigrationAdapterClient", trait = VaultMigrationAdapter);

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultConfig {
    pub legacy_version: u32,
    pub latest_version: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationOptIn {
    pub target_version: u32,
    pub opted_in_ledger: u32,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Vault(u32),
    Adapter(u32, u32),
    UserVersion(u32, Address),
    OptIn(u32, Address),
}

fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    let stored: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if *admin != stored {
        return Err(Error::Unauthorized);
    }
    admin.require_auth();
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    Ok(())
}

fn touch_instance(env: &Env) -> Result<(), Error> {
    if !env.storage().instance().has(&DataKey::Admin) {
        return Err(Error::NotInitialized);
    }
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    Ok(())
}

fn load_vault(env: &Env, vault_id: u32) -> Result<VaultConfig, Error> {
    touch_instance(env)?;
    let key = DataKey::Vault(vault_id);
    let config: VaultConfig = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(Error::VaultNotFound)?;
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
    Ok(config)
}

fn load_adapter(env: &Env, vault_id: u32, version: u32) -> Result<Address, Error> {
    let key = DataKey::Adapter(vault_id, version);
    let address: Address = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(Error::InvalidTargetVersion)?;
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
    Ok(address)
}

fn user_version(env: &Env, vault_id: u32, owner: &Address, legacy: u32) -> u32 {
    let key = DataKey::UserVersion(vault_id, owner.clone());
    let version = env.storage().persistent().get(&key).unwrap_or(legacy);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
    }
    version
}

fn validate_snapshot(
    snapshot: &VaultSnapshot,
    vault_id: u32,
    owner: &Address,
    version: u32,
) -> Result<(), Error> {
    if snapshot.vault_id != vault_id
        || snapshot.owner != *owner
        || snapshot.schema_version != version
        || snapshot.assets < 0
        || snapshot.shares < 0
    {
        return Err(Error::InvalidSnapshot);
    }
    Ok(())
}

#[contract]
pub struct VaultMigration;

#[contractimpl]
impl VaultMigration {
    /// Initialize the migration registry.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        Ok(())
    }

    /// Register an existing vault and its original adapter.
    ///
    /// Existing users default to `legacy_version` until they explicitly migrate.
    /// The legacy adapter remains registered and available after upgrades.
    pub fn register_vault(
        env: Env,
        admin: Address,
        vault_id: u32,
        legacy_version: u32,
        legacy_adapter: Address,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if legacy_version == 0 {
            return Err(Error::InvalidTargetVersion);
        }
        let vault_key = DataKey::Vault(vault_id);
        if env.storage().persistent().has(&vault_key) {
            return Err(Error::VaultAlreadyRegistered);
        }
        env.storage().persistent().set(
            &vault_key,
            &VaultConfig {
                legacy_version,
                latest_version: legacy_version,
            },
        );
        env.storage()
            .persistent()
            .extend_ttl(&vault_key, TTL_LEDGERS, TTL_LEDGERS);
        let adapter_key = DataKey::Adapter(vault_id, legacy_version);
        env.storage()
            .persistent()
            .set(&adapter_key, &legacy_adapter);
        env.storage()
            .persistent()
            .extend_ttl(&adapter_key, TTL_LEDGERS, TTL_LEDGERS);
        env.events().publish(
            (symbol_short!("register"), vault_id),
            (legacy_version, legacy_adapter),
        );
        Ok(())
    }

    /// Add the next version adapter. Versions are consecutive to prevent
    /// migration paths that skip a required intermediate state transformation.
    pub fn register_version(
        env: Env,
        admin: Address,
        vault_id: u32,
        expected_latest_version: u32,
        version: u32,
        adapter: Address,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        let mut config = load_vault(&env, vault_id)?;
        if config.latest_version != expected_latest_version {
            return Err(Error::StaleVersion);
        }
        let next_version = config
            .latest_version
            .checked_add(1)
            .ok_or(Error::InvalidTargetVersion)?;
        if version != next_version {
            return Err(Error::VersionNotNext);
        }

        let key = DataKey::Adapter(vault_id, version);
        if env.storage().persistent().has(&key) {
            return Err(Error::VersionAlreadyRegistered);
        }
        env.storage().persistent().set(&key, &adapter);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        config.latest_version = version;
        let vault_key = DataKey::Vault(vault_id);
        env.storage().persistent().set(&vault_key, &config);
        env.storage()
            .persistent()
            .extend_ttl(&vault_key, TTL_LEDGERS, TTL_LEDGERS);
        env.events().publish(
            (symbol_short!("version"), vault_id),
            (expected_latest_version, version, adapter),
        );
        Ok(())
    }

    /// Record an owner's explicit consent to migrate to the current version.
    pub fn opt_in(
        env: Env,
        vault_id: u32,
        owner: Address,
        target_version: u32,
    ) -> Result<(), Error> {
        owner.require_auth();
        let config = load_vault(&env, vault_id)?;
        if target_version != config.latest_version {
            return Err(Error::InvalidTargetVersion);
        }
        let current = user_version(&env, vault_id, &owner, config.legacy_version);
        if current >= target_version {
            return Err(Error::NoMigrationRequired);
        }
        load_adapter(&env, vault_id, target_version)?;

        let key = DataKey::OptIn(vault_id, owner.clone());
        let opt_in = MigrationOptIn {
            target_version,
            opted_in_ledger: env.ledger().sequence(),
        };
        env.storage().persistent().set(&key, &opt_in);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("opt_in"), vault_id), (owner, target_version));
        Ok(())
    }

    /// Revoke a pending opt-in before the migration is executed.
    pub fn cancel_opt_in(env: Env, vault_id: u32, owner: Address) -> Result<(), Error> {
        owner.require_auth();
        load_vault(&env, vault_id)?;
        let key = DataKey::OptIn(vault_id, owner.clone());
        if !env.storage().persistent().has(&key) {
            return Err(Error::MigrationNotOptedIn);
        }
        env.storage().persistent().remove(&key);
        env.events()
            .publish((symbol_short!("cancel"), vault_id), owner);
        Ok(())
    }

    /// Migrate one owner's state atomically from their current adapter to the
    /// opted-in target version. The owner authorizes both opt-in and execution.
    pub fn migrate(
        env: Env,
        vault_id: u32,
        owner: Address,
    ) -> Result<VaultSnapshot, Error> {
        owner.require_auth();
        let config = load_vault(&env, vault_id)?;
        let opt_key = DataKey::OptIn(vault_id, owner.clone());
        let opt_in: MigrationOptIn = env
            .storage()
            .persistent()
            .get(&opt_key)
            .ok_or(Error::MigrationNotOptedIn)?;
        env.storage()
            .persistent()
            .extend_ttl(&opt_key, TTL_LEDGERS, TTL_LEDGERS);
        if opt_in.target_version != config.latest_version {
            return Err(Error::OptInOutdated);
        }

        let from_version = user_version(&env, vault_id, &owner, config.legacy_version);
        if from_version >= opt_in.target_version {
            return Err(Error::NoMigrationRequired);
        }
        let source = load_adapter(&env, vault_id, from_version)?;
        let target = load_adapter(&env, vault_id, opt_in.target_version)?;
        let source_client = VaultMigrationAdapterClient::new(&env, &source);
        let mut snapshot = source_client.export_state(&vault_id, &owner);
        validate_snapshot(&snapshot, vault_id, &owner, from_version)?;

        let target_client = VaultMigrationAdapterClient::new(&env, &target);
        if target_client.has_state(&vault_id, &owner) {
            return Err(Error::DestinationAlreadyInitialized);
        }
        snapshot.schema_version = opt_in.target_version;
        target_client.import_state(&vault_id, &owner, &snapshot);

        let migrated = target_client.export_state(&vault_id, &owner);
        validate_snapshot(&migrated, vault_id, &owner, opt_in.target_version)?;
        if migrated.asset != snapshot.asset
            || migrated.assets != snapshot.assets
            || migrated.shares != snapshot.shares
            || migrated.strategy != snapshot.strategy
        {
            return Err(Error::InvalidSnapshot);
        }

        let version_key = DataKey::UserVersion(vault_id, owner.clone());
        env.storage()
            .persistent()
            .set(&version_key, &opt_in.target_version);
        env.storage()
            .persistent()
            .extend_ttl(&version_key, TTL_LEDGERS, TTL_LEDGERS);
        env.storage().persistent().remove(&opt_key);
        env.events().publish(
            (symbol_short!("migrate"), vault_id),
            (owner, from_version, opt_in.target_version),
        );
        Ok(migrated)
    }

    /// Report the effective current and opted-in versions for an owner.
    pub fn status(
        env: Env,
        vault_id: u32,
        owner: Address,
    ) -> Result<MigrationStatus, Error> {
        let config = load_vault(&env, vault_id)?;
        let current_version = user_version(&env, vault_id, &owner, config.legacy_version);
        let key = DataKey::OptIn(vault_id, owner);
        let opted_in: Option<MigrationOptIn> = env.storage().persistent().get(&key);
        if opted_in.is_some() {
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
        }
        Ok(MigrationStatus {
            current_version,
            opted_in_version: opted_in.map(|entry| entry.target_version),
        })
    }

    /// Latest registered version for a vault.
    pub fn latest_version(env: Env, vault_id: u32) -> Result<u32, Error> {
        Ok(load_vault(&env, vault_id)?.latest_version)
    }

    /// Registered adapter address for a specific version.
    pub fn adapter(env: Env, vault_id: u32, version: u32) -> Result<Address, Error> {
        load_vault(&env, vault_id)?;
        load_adapter(&env, vault_id, version)
    }

    /// Admin address.
    pub fn admin(env: Env) -> Result<Address, Error> {
        touch_instance(&env)?;
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}
