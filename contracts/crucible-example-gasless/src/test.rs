#![cfg(test)]

use crate::{Gasless, GaslessClient, MetaTx};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal, Symbol,
};

const AMOUNT: i128 = 1_000_000;
const BASE_TIME: u64 = 1_000_000;
const DEADLINE: u64 = BASE_TIME + 3_600;

struct Ctx {
    env: Env,
    contract_id: Address,
    relayer: Address,
    alice: Address,
    bob: Address,
    token_id: Address,
}

impl Ctx {
    fn setup() -> Self {
        let env = Env::default();
        env.ledger().with_mut(|l| {
            l.timestamp = BASE_TIME;
        });

        let relayer = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        env.mock_all_auths();

        StellarAssetClient::new(&env, &token_id).mint(&alice, &(AMOUNT * 5));

        let contract_id = env.register(Gasless, ());
        GaslessClient::new(&env, &contract_id).initialize(&relayer);

        Ctx {
            env,
            contract_id,
            relayer,
            alice,
            bob,
            token_id,
        }
    }

    fn client(&self) -> GaslessClient<'_> {
        GaslessClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    fn meta_tx(&self, nonce: u64) -> MetaTx {
        MetaTx {
            from: self.alice.clone(),
            to: self.bob.clone(),
            token: self.token_id.clone(),
            amount: AMOUNT,
            nonce,
            deadline: DEADLINE,
        }
    }
}

#[test]
fn test_execute_transfers_tokens() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    assert_eq!(ctx.token().balance(&ctx.alice), AMOUNT * 4);
    assert_eq!(ctx.token().balance(&ctx.bob), AMOUNT);
}

#[test]
fn test_nonce_increments_after_execute() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_eq!(ctx.client().nonce(&ctx.alice), 0);
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    assert_eq!(ctx.client().nonce(&ctx.alice), 1);
}

#[test]
#[should_panic(expected = "invalid nonce")]
fn test_replay_attack_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    // Replay with same nonce must revert.
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
}

#[test]
fn test_sequential_nonces_succeed() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(1));
    assert_eq!(ctx.token().balance(&ctx.bob), AMOUNT * 2);
}

#[test]
#[should_panic(expected = "meta-tx expired")]
fn test_expired_meta_tx_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.env.ledger().with_mut(|l| {
        l.timestamp = DEADLINE + 1;
    });
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
}

#[test]
#[should_panic(expected = "unauthorized relayer")]
fn test_unauthorized_relayer_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.alice, &ctx.meta_tx(0));
}

#[test]
#[should_panic(expected = "invalid nonce")]
fn test_wrong_nonce_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // Nonce 1 is wrong when 0 is expected.
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(1));
}

#[test]
fn test_nonce_starts_at_zero() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client().nonce(&ctx.alice), 0);
    assert_eq!(ctx.client().nonce(&ctx.bob), 0);
}

#[test]
fn test_relayer_returns_correct_address() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client().relayer(), ctx.relayer.clone());
}

#[test]
#[should_panic(expected = "already initialized")]
fn test_double_initialize_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().initialize(&ctx.relayer);
}

#[test]
fn test_multiple_users_have_independent_nonces() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&ctx.bob, &(AMOUNT * 5));

    // alice uses nonce 0.
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));

    // bob's nonce is still 0 and is independent.
    let bob_tx = MetaTx {
        from: ctx.bob.clone(),
        to: ctx.alice.clone(),
        token: ctx.token_id.clone(),
        amount: AMOUNT,
        nonce: 0,
        deadline: DEADLINE,
    };
    ctx.client().execute(&ctx.relayer, &bob_tx);

    assert_eq!(ctx.client().nonce(&ctx.alice), 1);
    assert_eq!(ctx.client().nonce(&ctx.bob), 1);
}

#[test]
#[should_panic(expected = "invalid nonce")]
fn test_skipped_nonce_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // Skipping from nonce 0 to 2 must revert.
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(2));
}

#[test]
fn test_many_sequential_nonces() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    // Mint enough tokens for 10 transfers.
    StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&ctx.alice, &(AMOUNT * 10));

    for nonce in 0..10u64 {
        ctx.client().execute(&ctx.relayer, &ctx.meta_tx(nonce));
    }

    assert_eq!(ctx.client().nonce(&ctx.alice), 10);
    assert_eq!(ctx.token().balance(&ctx.bob), AMOUNT * 10);
}

#[test]
fn test_nonce_persists_in_persistent_storage() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();

    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));

    // Simulate ledger advancement — persistent storage survives.
    ctx.env.ledger().with_mut(|l| {
        l.sequence_number += 1_000;
        l.timestamp += 10_000;
    });

    // Nonce must still be 1 after ledger advancement.
    assert_eq!(ctx.client().nonce(&ctx.alice), 1);

    // Replay of nonce 0 must still revert.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));
    }));
    assert!(result.is_err(), "replay with consumed nonce must revert");
}

#[test]
fn test_execute_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.client().execute(&ctx.relayer, &ctx.meta_tx(0));

    let events = ctx.env.events().all();
    let has_executed = events.iter().any(|(_, topics, _)| {
        topics.len() == 1
            && topics.get(0) == Some(Symbol::new(&ctx.env, "executed").into_val(&ctx.env))
    });
    assert!(has_executed, "expected 'executed' event");
}
