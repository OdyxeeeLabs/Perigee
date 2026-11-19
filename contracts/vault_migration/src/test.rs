#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

#[contract]
pub struct MockVaultAdapter;

#[contracttype]
#[derive(Clone)]
enum AdapterKey {
    Version,
    State(u32, Address),
}

#[contractimpl]
impl VaultMigrationAdapter for MockVaultAdapter {
    fn export_state(env: Env, vault_id: u32, owner: Address) -> VaultSnapshot {
        env.storage()
            .persistent()
            .get(&AdapterKey::State(vault_id, owner))
            .unwrap()
    }

    fn has_state(env: Env, vault_id: u32, owner: Address) -> bool {
        env.storage()
            .persistent()
            .has(&AdapterKey::State(vault_id, owner))
    }

    fn import_state(env: Env, vault_id: u32, owner: Address, snapshot: VaultSnapshot) {
        let version: u32 = env.storage().instance().get(&AdapterKey::Version).unwrap();
        assert_eq!(snapshot.vault_id, vault_id);
        assert_eq!(snapshot.owner, owner);
        assert_eq!(snapshot.schema_version, version);
        env.storage()
            .persistent()
            .set(&AdapterKey::State(vault_id, owner), &snapshot);
    }
}

#[contractimpl]
impl MockVaultAdapter {
    pub fn initialize(env: Env, version: u32) {
        env.storage().instance().set(&AdapterKey::Version, &version);
    }

    pub fn seed(env: Env, snapshot: VaultSnapshot) {
        env.storage().persistent().set(
            &AdapterKey::State(snapshot.vault_id, snapshot.owner.clone()),
            &snapshot,
        );
    }

    pub fn state(env: Env, vault_id: u32, owner: Address) -> VaultSnapshot {
        env.storage()
            .persistent()
            .get(&AdapterKey::State(vault_id, owner))
            .unwrap()
    }
}

struct Setup {
    env: Env,
    contract: Address,
    admin: Address,
    owner: Address,
    legacy_adapter: Address,
    v2_adapter: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let contract = env.register(VaultMigration, ());
    let client = VaultMigrationClient::new(&env, &contract);
    client.initialize(&admin);

    let legacy_adapter = env.register(MockVaultAdapter, ());
    MockVaultAdapterClient::new(&env, &legacy_adapter).initialize(&1);
    let v2_adapter = env.register(MockVaultAdapter, ());
    MockVaultAdapterClient::new(&env, &v2_adapter).initialize(&2);
    client.register_vault(&admin, &77, &1, &legacy_adapter);
    client.register_version(&admin, &77, &1, &2, &v2_adapter);
    MockVaultAdapterClient::new(&env, &legacy_adapter).seed(&VaultSnapshot {
        vault_id: 77,
        owner: owner.clone(),
        schema_version: 1,
        asset: Address::generate(&env),
        assets: 10_000,
        shares: 9_500,
        strategy: None,
    });

    Setup {
        env,
        contract,
        admin,
        owner,
        legacy_adapter,
        v2_adapter,
    }
}

impl Setup {
    fn client(&self) -> VaultMigrationClient<'_> {
        VaultMigrationClient::new(&self.env, &self.contract)
    }
}

#[test]
fn registration_starts_existing_users_on_legacy_version() {
    let setup = setup();
    let status = setup.client().status(&77, &setup.owner);
    assert_eq!(status.current_version, 1);
    assert_eq!(status.opted_in_version, None);
    assert_eq!(setup.client().latest_version(&77), 2);
}

#[test]
fn opt_in_and_migration_preserve_state_and_advance_user_version() {
    let setup = setup();
    setup.client().opt_in(&77, &setup.owner, &2);
    let opted = setup.client().status(&77, &setup.owner);
    assert_eq!(opted.current_version, 1);
    assert_eq!(opted.opted_in_version, Some(2));

    let migrated = setup.client().migrate(&77, &setup.owner);
    assert_eq!(migrated.schema_version, 2);
    assert_eq!(migrated.assets, 10_000);
    assert_eq!(migrated.shares, 9_500);
    assert_eq!(migrated.strategy, None);

    let status = setup.client().status(&77, &setup.owner);
    assert_eq!(status.current_version, 2);
    assert_eq!(status.opted_in_version, None);
}

#[test]
fn unmigrated_users_keep_their_legacy_position() {
    let setup = setup();
    let old_state =
        MockVaultAdapterClient::new(&setup.env, &setup.legacy_adapter).state(&77, &setup.owner);
    let new_state = MockVaultAdapterClient::new(&setup.env, &setup.v2_adapter)
        .try_state(&77, &setup.owner);
    assert_eq!(old_state.schema_version, 1);
    assert_eq!(old_state.assets, 10_000);
    assert!(new_state.is_err());
    assert_eq!(setup.client().status(&77, &setup.owner).current_version, 1);
}

#[test]
fn migration_requires_explicit_opt_in_and_is_not_replayable() {
    let setup = setup();
    assert_eq!(
        setup.client().try_migrate(&77, &setup.owner),
        Err(Ok(Error::MigrationNotOptedIn))
    );

    setup.client().opt_in(&77, &setup.owner, &2);
    setup.client().migrate(&77, &setup.owner);
    assert_eq!(
        setup.client().try_migrate(&77, &setup.owner),
        Err(Ok(Error::MigrationNotOptedIn))
    );
}

#[test]
fn owner_can_cancel_opt_in_before_migration() {
    let setup = setup();
    setup.client().opt_in(&77, &setup.owner, &2);
    setup.client().cancel_opt_in(&77, &setup.owner);

    let status = setup.client().status(&77, &setup.owner);
    assert_eq!(status.current_version, 1);
    assert_eq!(status.opted_in_version, None);
    assert_eq!(
        setup.client().try_migrate(&77, &setup.owner),
        Err(Ok(Error::MigrationNotOptedIn))
    );
}

#[test]
fn newer_versions_invalidate_stale_opt_ins() {
    let setup = setup();
    let v3_adapter = setup.env.register(MockVaultAdapter, ());
    MockVaultAdapterClient::new(&setup.env, &v3_adapter).initialize(&3);
    setup.client().opt_in(&77, &setup.owner, &2);
    setup
        .client()
        .register_version(&setup.admin, &77, &2, &3, &v3_adapter);

    assert_eq!(
        setup.client().try_migrate(&77, &setup.owner),
        Err(Ok(Error::OptInOutdated))
    );
}

#[test]
fn versions_must_be_added_consecutively_and_against_current_state() {
    let setup = setup();
    let adapter = setup.env.register(MockVaultAdapter, ());
    MockVaultAdapterClient::new(&setup.env, &adapter).initialize(&4);

    assert_eq!(
        setup
            .client()
            .try_register_version(&setup.admin, &77, &1, &3, &adapter),
        Err(Ok(Error::StaleVersion))
    );
    assert_eq!(
        setup
            .client()
            .try_register_version(&setup.admin, &77, &2, &4, &adapter),
        Err(Ok(Error::VersionNotNext))
    );
}

#[test]
fn user_can_opt_into_only_the_current_registered_version() {
    let setup = setup();
    assert_eq!(
        setup.client().try_opt_in(&77, &setup.owner, &1),
        Err(Ok(Error::InvalidTargetVersion))
    );
    assert_eq!(
        setup.client().try_opt_in(&77, &setup.owner, &3),
        Err(Ok(Error::InvalidTargetVersion))
    );
}

#[test]
fn migration_refuses_to_overwrite_a_destination_position() {
    let setup = setup();
    MockVaultAdapterClient::new(&setup.env, &setup.v2_adapter).seed(&VaultSnapshot {
        vault_id: 77,
        owner: setup.owner.clone(),
        schema_version: 2,
        asset: Address::generate(&setup.env),
        assets: 1,
        shares: 1,
        strategy: None,
    });
    setup.client().opt_in(&77, &setup.owner, &2);

    assert_eq!(
        setup.client().try_migrate(&77, &setup.owner),
        Err(Ok(Error::DestinationAlreadyInitialized))
    );
    assert_eq!(setup.client().status(&77, &setup.owner).current_version, 1);
}

#[test]
fn invalid_legacy_state_does_not_advance_the_version() {
    let setup = setup();
    MockVaultAdapterClient::new(&setup.env, &setup.legacy_adapter).seed(&VaultSnapshot {
        vault_id: 77,
        owner: setup.owner.clone(),
        schema_version: 1,
        asset: Address::generate(&setup.env),
        assets: -1,
        shares: 9_500,
        strategy: None,
    });
    setup.client().opt_in(&77, &setup.owner, &2);

    assert_eq!(
        setup.client().try_migrate(&77, &setup.owner),
        Err(Ok(Error::InvalidSnapshot))
    );
    assert_eq!(setup.client().status(&77, &setup.owner).current_version, 1);
}

// ── #849 — initialization state ─────────────────────────────────────────────

#[test]
fn registry_initialization_sets_admin_once() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(VaultMigration, ());
    let client = VaultMigrationClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));

    client.initialize(&admin);

    assert_eq!(client.admin(), admin);
    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn calls_before_initialization_are_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let adapter = Address::generate(&env);
    let contract_id = env.register(VaultMigration, ());
    let client = VaultMigrationClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(
        client.try_latest_version(&0),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_register_vault(&admin, &0, &1, &adapter),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_register_version(&admin, &0, &1, &2, &adapter),
        Err(Ok(Error::NotInitialized))
    );
}
