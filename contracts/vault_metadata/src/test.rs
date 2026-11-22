#![cfg(test)]

//! Unit tests for the vault metadata registry.
//!
//! The behaviours worth pinning are the ones that make discovery correct: trait
//! indexes that stay consistent with the record across updates and deregistration,
//! and a version history that an off-chain indexer can order on.
//!
//! Closes OdyxeeeLabs/Perigee#639

use crate::{
    DisplayMetadataUpdate, RiskTier, VaultMetadata, VaultMetadataClient, Error,
    HISTORY_DEPTH, MAX_RISK_SCORE,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.sequence_number = 1_000);

        let admin = Address::generate(&env);
        let vault = Address::generate(&env);

        let contract_id = env.register(VaultMetadata, ());
        let client = VaultMetadataClient::new(&env, &contract_id);
        client.initialize(&admin);

        (env, contract_id, client, admin, vault)
    }};
}

fn sid(env: &Env, value: &str) -> String {
    String::from_str(env, value)
}

/// Register a vault with the given traits.
macro_rules! register {
    ($env:expr, $client:expr, $admin:expr, $vault:expr, $strategy:expr, $tier:expr, $score:expr) => {{
        $client.register(
            $admin,
            $vault,
            &sid($env, "Perigee USDC Vault"),
            &sid($env, "pUSDC"),
            &sid($env, $strategy),
            $tier,
            $score,
            &sid($env, "ipfs://perigee/vault"),
        );
    }};
}

// ── Registration ─────────────────────────────────────────────────────────────

#[test]
fn registration_records_every_field() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    let metadata = client.get_metadata(&vault);
    assert_eq!(metadata.name, sid(&env, "Perigee USDC Vault"));
    assert_eq!(metadata.symbol, sid(&env, "pUSDC"));
    assert_eq!(metadata.strategy, sid(&env, "lending"));
    assert_eq!(metadata.risk_tier, RiskTier::Low);
    assert_eq!(metadata.risk_score, 20);
    assert_eq!(metadata.version, 1);
    assert_eq!(client.vault_count(), 1);
}

#[test]
fn a_vault_cannot_be_registered_twice() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    assert_eq!(
        client.try_register(
            &admin,
            &vault,
            &sid(&env, "Dup"),
            &sid(&env, "DUP"),
            &sid(&env, "lending"),
            &RiskTier::Low,
            &10,
            &sid(&env, "ipfs://x")
        ),
        Err(Ok(Error::MetadataExists))
    );
    assert_eq!(client.vault_count(), 1);
}

#[test]
fn empty_and_over_long_fields_are_refused() {
    let (env, _cid, client, admin, vault) = setup!();

    assert_eq!(
        client.try_register(
            &admin,
            &vault,
            &sid(&env, ""),
            &sid(&env, "SYM"),
            &sid(&env, "lending"),
            &RiskTier::Low,
            &10,
            &sid(&env, "ipfs://x")
        ),
        Err(Ok(Error::EmptyField))
    );

    assert_eq!(
        client.try_register(
            &admin,
            &vault,
            &sid(&env, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            &sid(&env, "SYM"),
            &sid(&env, "lending"),
            &RiskTier::Low,
            &10,
            &sid(&env, "ipfs://x")
        ),
        Err(Ok(Error::FieldTooLong))
    );
}

#[test]
fn a_risk_score_above_one_hundred_is_refused() {
    let (env, _cid, client, admin, vault) = setup!();

    assert_eq!(
        client.try_register(
            &admin,
            &vault,
            &sid(&env, "Name"),
            &sid(&env, "SYM"),
            &sid(&env, "lending"),
            &RiskTier::High,
            &MAX_RISK_SCORE + 1,
            &sid(&env, "ipfs://x")
        ),
        Err(Ok(Error::InvalidRisk))
    );
}

#[test]
fn only_the_admin_may_register() {
    let (env, _cid, client, _admin, vault) = setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_register(
            &stranger,
            &vault,
            &sid(&env, "Name"),
            &sid(&env, "SYM"),
            &sid(&env, "lending"),
            &RiskTier::Low,
            &10,
            &sid(&env, "ipfs://x")
        ),
        Err(Ok(Error::Unauthorized))
    );
}

// ── Trait discovery ──────────────────────────────────────────────────────────

#[test]
fn vaults_are_discoverable_by_strategy() {
    let (env, _cid, client, admin, first) = setup!();
    let second = Address::generate(&env);

    register!(env, client, admin, first, "lending", RiskTier::Low, 20);
    register!(env, client, admin, second, "lending", RiskTier::Medium, 50);
    register!(
        env,
        client,
        admin,
        &Address::generate(&env),
        "dex-lp",
        RiskTier::High,
        &80,
        &sid(&env, "pDEX")
    );

    let lending = client.list_by_strategy(&sid(&env, "lending"));
    assert_eq!(lending.len(), 2);
    assert!(lending.contains(&first));
    assert!(lending.contains(&second));

    assert_eq!(client.list_by_strategy(&sid(&env, "staking")).len(), 0);
}

#[test]
fn vaults_are_discoverable_by_risk_tier() {
    let (env, _cid, client, admin, first) = setup!();
    let second = Address::generate(&env);

    register!(env, client, admin, first, "lending", RiskTier::High, 90);
    register!(env, client, admin, second, "lending", RiskTier::High, 85);
    register!(
        env,
        client,
        admin,
        &Address::generate(&env),
        "dex-lp",
        RiskTier::Low,
        &10,
        &sid(&env, "pSAFE")
    );

    assert_eq!(client.list_by_risk(&RiskTier::High).len(), 2);
    assert_eq!(client.list_by_risk(&RiskTier::Low).len(), 1);
    assert_eq!(client.list_by_risk(&RiskTier::Medium).len(), 0);
}

#[test]
fn updating_a_vault_moves_it_between_indexes() {
    // The index and the record must never disagree, or discovery silently returns
    // a vault that no longer matches the trait it was queried by.
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    client.update(
        &admin,
        &vault,
        &sid(&env, "Perigee USDC Vault"),
        &sid(&env, "pUSDC"),
        &sid(&env, "staking"),
        &RiskTier::High,
        90,
        &sid(&env, "ipfs://perigee/vault"),
    );

    assert_eq!(client.list_by_strategy(&sid(&env, "lending")).len(), 0);
    assert_eq!(client.list_by_strategy(&sid(&env, "staking")).len(), 1);
    assert_eq!(client.list_by_risk(&RiskTier::Low).len(), 0);
    assert_eq!(client.list_by_risk(&RiskTier::High).len(), 1);

    let listed = client.list_by_strategy(&sid(&env, "staking"));
    assert!(listed.contains(&vault));
}

#[test]
fn deregistering_removes_the_vault_from_every_index() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Medium, 50);

    client.deregister(&admin, &vault);

    assert_eq!(client.list_by_strategy(&sid(&env, "lending")).len(), 0);
    assert_eq!(client.list_by_risk(&RiskTier::Medium).len(), 0);
    assert_eq!(client.vault_count(), 0);
    assert_eq!(
        client.try_get_metadata(&vault),
        Err(Ok(Error::MetadataNotFound))
    );
}

// ── Versioning ───────────────────────────────────────────────────────────────

#[test]
fn an_update_bumps_the_version() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    let version = client.update(
        &admin,
        &vault,
        &sid(&env, "Renamed"),
        &sid(&env, "pUSDC2"),
        &sid(&env, "lending"),
        &RiskTier::Low,
        &25,
        &sid(&env, "ipfs://perigee/vault2"),
    );

    assert_eq!(version, 2);
    assert_eq!(client.get_metadata(&vault).version, 2);
    assert_eq!(client.get_metadata(&vault).name, sid(&env, "Renamed"));
}

#[test]
fn history_records_every_version_in_order() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    client.update(
        &admin,
        &vault,
        &sid(&env, "V2"),
        &sid(&env, "pUSDC"),
        &sid(&env, "lending"),
        &RiskTier::Low,
        &30,
        &sid(&env, "ipfs://x"),
    );
    client.update(
        &admin,
        &vault,
        &sid(&env, "V3"),
        &sid(&env, "pUSDC"),
        &sid(&env, "lending"),
        &RiskTier::Low,
        &40,
        &sid(&env, "ipfs://x"),
    );

    assert_eq!(client.history_len(&vault), 3);
    assert_eq!(client.get_history(&vault, &0).unwrap().version, 1);
    assert_eq!(client.get_history(&vault, &1).unwrap().version, 2);
    assert_eq!(client.get_history(&vault, &2).unwrap().version, 3);
    assert_eq!(client.latest_version(&vault).unwrap().metadata.name, sid(&env, "V3"));
}

#[test]
fn history_is_trimmed_to_its_depth() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    for score in 1..=(HISTORY_DEPTH + 4) {
        client.update(
            &admin,
            &vault,
            &sid(&env, "N"),
            &sid(&env, "pUSDC"),
            &sid(&env, "lending"),
            &RiskTier::Low,
            &(score as u32 % 100),
            &sid(&env, "ipfs://x"),
        );
    }

    assert_eq!(client.history_len(&vault), HISTORY_DEPTH);
    // The oldest retained entry is no longer version 1.
    assert!(client.get_history(&vault, &0).unwrap().version > 1);
}

#[test]
fn a_version_outside_the_history_is_reported() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    assert_eq!(
        client.try_get_history(&vault, &99),
        Err(Ok(Error::HistoryOutOfRange))
    );
}

#[test]
fn deregistering_keeps_the_history() {
    // The record is gone but the history survives, so an indexer can still answer
    // what a vault used to be.
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);
    client.deregister(&admin, &vault);

    assert_eq!(client.history_len(&vault), 1);
    assert!(client.latest_version(&vault).is_ok());
}

// ── Queries and administration ───────────────────────────────────────────────

#[test]
fn an_unregistered_vault_has_no_metadata() {
    let (env, _cid, client, _admin, _vault) = setup!();

    assert_eq!(
        client.try_get_metadata(&Address::generate(&env)),
        Err(Ok(Error::MetadataNotFound))
    );
}

#[test]
fn queries_before_initialization_report_not_initialized() {
    let env = Env::default();
    let contract_id = env.register(VaultMetadata, ());
    let client = VaultMetadataClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_vault_count(), Err(Ok(Error::NotInitialized)));
}

#[test]
fn the_registry_can_only_be_initialized_once() {
    let (env, _cid, client, admin, _vault) = setup!();

    assert_eq!(client.try_initialize(&admin), Err(Ok(Error::AlreadyInitialized)));
}

#[test]
fn display_update_preserves_canonical_identity_and_protocol_traits() {
    let (env, contract_id, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);

    let version = client.update_display(
        &admin,
        &vault,
        &DisplayMetadataUpdate {
            name: sid(&env, "Renamed Vault"),
            symbol: sid(&env, "rUSDC"),
            metadata_uri: sid(&env, "ipfs://renamed"),
        },
    );

    let metadata = client.get_metadata(&vault);
    assert_eq!(version, 2);
    assert_eq!(metadata.vault, vault);
    assert_eq!(metadata.name, sid(&env, "Renamed Vault"));
    assert_eq!(metadata.symbol, sid(&env, "rUSDC"));
    assert_eq!(metadata.strategy, sid(&env, "lending"));
    assert_eq!(metadata.risk_tier, RiskTier::Low);
    assert_eq!(metadata.risk_score, 20);

    let events = env.events().all();
    let mut found = false;
    for (_, topics, data) in events.iter() {
        if topics.len() != 3 {
            continue;
        }
        let topic_vault: Result<Address, _> = topics.get(1).unwrap().try_into_val(&env);
        let topic_action: Result<_, _> = topics.get(2).unwrap().try_into_val(&env);
        let action = topic_action.ok().map(|s: soroban_sdk::Symbol| s);
        let event_data: Result<(u32, u32), _> = data.try_into_val(&env);
        if topic_vault.ok() == Some(vault.clone())
            && action == Some(soroban_sdk::Symbol::new(&env, "up"))
            && event_data.ok() == Some((1, 2))
        {
            found = true;
        }
    }
    assert!(found);

    assert_eq!(client.list_by_strategy(&sid(&env, "lending")).len(), 1);
    assert_eq!(client.list_by_risk(&RiskTier::Low).len(), 1);
}

#[test]
fn only_the_admin_may_update_display() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_update_display(
            &stranger,
            &vault,
            &DisplayMetadataUpdate {
                name: sid(&env, "X"),
                symbol: sid(&env, "X"),
                metadata_uri: sid(&env, "ipfs://x"),
            }
        ),
        Err(Ok(Error::Unauthorized))
    );

    let metadata = client.get_metadata(&vault);
    assert_eq!(metadata.vault, vault);
    assert_eq!(metadata.version, 1);
}

#[test]
fn only_the_admin_may_update_or_deregister() {
    let (env, _cid, client, admin, vault) = setup!();
    register!(env, client, admin, vault, "lending", RiskTier::Low, 20);
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_update(
            &stranger,
            &vault,
            &sid(&env, "X"),
            &sid(&env, "X"),
            &sid(&env, "lending"),
            &RiskTier::Low,
            &10,
            &sid(&env, "ipfs://x")
        ),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_deregister(&stranger, &vault),
        Err(Ok(Error::Unauthorized))
    );
    // Nothing changed.
    assert_eq!(client.get_metadata(&vault).version, 1);
}