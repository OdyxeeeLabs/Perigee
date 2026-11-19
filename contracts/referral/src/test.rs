#![cfg(test)]

use crate::{Error, Referral, ReferralClient, ReferralConfig};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, BytesN, Env,
};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(Referral, ());
    let client = ReferralClient::new(&env, &id);
    client.initialize(
        &admin,
        &ReferralConfig {
            reward_bps: 100, // 1%
            max_referees: 3,
            min_referee_age_ledgers: 5,
            min_volume_for_reward: 100,
        },
    );
    (env, id, admin)
}

#[test]
fn bind_and_lookup_referrer() {
    let (env, id, _) = setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);
    client.bind_referrer(&referee, &referrer);
    assert_eq!(client.get_referrer(&referee), Some(referrer.clone()));
    let stats = client.get_stats(&referrer);
    assert_eq!(stats.referee_count, 1);
}

#[test]
fn rejects_self_referral_and_double_bind() {
    let (env, id, _) = setup();
    let client = ReferralClient::new(&env, &id);
    let user = Address::generate(&env);
    assert_eq!(
        client.try_bind_referrer(&user, &user),
        Err(Ok(Error::SelfReferral))
    );
    let referrer = Address::generate(&env);
    client.bind_referrer(&user, &referrer);
    assert_eq!(
        client.try_bind_referrer(&user, &Address::generate(&env)),
        Err(Ok(Error::AlreadyReferred))
    );
}

#[test]
fn enforces_referee_cap() {
    let (env, id, _) = setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    for _ in 0..3 {
        client.bind_referrer(&Address::generate(&env), &referrer);
    }
    assert_eq!(
        client.try_bind_referrer(&Address::generate(&env), &referrer),
        Err(Ok(Error::RefereeLimit))
    );
}

#[test]
fn volume_requires_age_and_minimum() {
    let (env, id, admin) = setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);
    client.bind_referrer(&referee, &referrer);

    assert_eq!(
        client.try_attribute_volume(&referee, &50),
        Err(Ok(Error::VolumeTooLow))
    );
    assert_eq!(
        client.try_attribute_volume(&referee, &1_000),
        Err(Ok(Error::RefereeTooNew))
    );

    env.ledger().with_mut(|l| l.sequence_number += 10);
    let reward = client.attribute_volume(&referee, &1_000);
    assert_eq!(reward, 10); // 1% of 1000

    client.fund_pool(&admin, &100);
    let claimed = client.claim_rewards(&referrer);
    assert_eq!(claimed, 10);
    assert_eq!(client.reward_pool(), 90);
}

// --- Caps, duplicate attribution and budget (#868) ---

fn aged_setup() -> (Env, Address, Address) {
    let (env, id, admin) = setup();
    env.ledger().with_mut(|l| l.sequence_number = 100);
    (env, id, admin)
}

fn bind_aged(env: &Env, client: &ReferralClient, referee: &Address, referrer: &Address) {
    client.bind_referrer(referee, referrer);
    env.ledger().with_mut(|l| l.sequence_number += 10);
}

fn action(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::from_array(env, &[byte; 32])
}

#[test]
fn competing_referrers_cannot_both_claim_the_same_user() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let first = Address::generate(&env);
    let second = Address::generate(&env);
    let user = Address::generate(&env);

    client.bind_referrer(&user, &first);
    assert_eq!(
        client.try_bind_referrer(&user, &second),
        Err(Ok(Error::AlreadyReferred))
    );

    // The first binding stands and the loser gained nothing.
    assert_eq!(client.get_referrer(&user), Some(first.clone()));
    assert_eq!(client.get_stats(&first).referee_count, 1);
    assert_eq!(client.get_stats(&second).referee_count, 0);

    // Rebinding to the same referrer is also refused, so it cannot be used to
    // reset the bound ledger.
    assert_eq!(
        client.try_bind_referrer(&user, &first),
        Err(Ok(Error::AlreadyReferred))
    );
}

#[test]
fn volume_only_accrues_to_the_bound_referrer() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let first = Address::generate(&env);
    let second = Address::generate(&env);
    let user = Address::generate(&env);
    bind_aged(&env, &client, &user, &first);
    let _ = client.try_bind_referrer(&user, &second);

    assert_eq!(client.attribute_volume(&user, &1_000), 10);
    assert_eq!(client.get_stats(&first).rewards_accrued, 10);
    assert_eq!(client.get_stats(&second).rewards_accrued, 0);
    assert_eq!(client.get_stats(&second).qualified_volume, 0);
}

#[test]
fn self_referral_is_rejected_without_touching_state() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let user = Address::generate(&env);
    assert_eq!(
        client.try_bind_referrer(&user, &user),
        Err(Ok(Error::SelfReferral))
    );
    assert_eq!(client.get_referrer(&user), None);
    assert_eq!(client.get_stats(&user).referee_count, 0);
    // A failed self-referral does not burn the user's one-time binding.
    client.bind_referrer(&user, &Address::generate(&env));
    assert!(client.get_referrer(&user).is_some());
}

#[test]
fn referee_cap_boundary_and_failed_bind_does_not_count() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    // max_referees = 3: the third bind (the boundary) succeeds...
    for n in 1..=3u32 {
        client.bind_referrer(&Address::generate(&env), &referrer);
        assert_eq!(client.get_stats(&referrer).referee_count, n);
    }
    // ...the fourth is rejected and leaves the count at the cap.
    let late = Address::generate(&env);
    assert_eq!(
        client.try_bind_referrer(&late, &referrer),
        Err(Ok(Error::RefereeLimit))
    );
    assert_eq!(client.get_stats(&referrer).referee_count, 3);
    // The rejected user is not bound and may still pick another referrer.
    assert_eq!(client.get_referrer(&late), None);
    client.bind_referrer(&late, &Address::generate(&env));
}

#[test]
fn zero_cap_blocks_all_binding() {
    let (env, id, admin) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    client.set_config(
        &admin,
        &ReferralConfig {
            reward_bps: 100,
            max_referees: 0,
            min_referee_age_ledgers: 5,
            min_volume_for_reward: 100,
        },
    );
    assert_eq!(
        client.try_bind_referrer(&Address::generate(&env), &Address::generate(&env)),
        Err(Ok(Error::RefereeLimit))
    );
}

#[test]
fn rejected_attribution_changes_no_accounting() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);
    client.bind_referrer(&referee, &referrer);
    let before = client.get_stats(&referrer);

    // Too small, too new, non-positive and unknown referee: all rejected.
    assert_eq!(client.try_attribute_volume(&referee, &99), Err(Ok(Error::VolumeTooLow)));
    assert_eq!(client.try_attribute_volume(&referee, &1_000), Err(Ok(Error::RefereeTooNew)));
    assert_eq!(client.try_attribute_volume(&referee, &0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        client.try_attribute_volume(&Address::generate(&env), &1_000),
        Err(Ok(Error::ReferrerNotFound))
    );
    assert_eq!(client.get_stats(&referrer), before);
}

#[test]
fn attribute_action_credits_each_action_at_most_once() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);
    bind_aged(&env, &client, &referee, &referrer);

    assert_eq!(client.attribute_action(&referee, &action(&env, 1), &1_000), 10);
    assert_eq!(
        client.try_attribute_action(&referee, &action(&env, 1), &1_000),
        Err(Ok(Error::DuplicateAction))
    );
    let stats = client.get_stats(&referrer);
    assert_eq!(stats.rewards_accrued, 10);
    assert_eq!(stats.qualified_volume, 1_000);

    // A different action id is a different action.
    assert_eq!(client.attribute_action(&referee, &action(&env, 2), &1_000), 10);
    assert_eq!(client.get_stats(&referrer).rewards_accrued, 20);
}

#[test]
fn a_rejected_action_does_not_burn_its_id() {
    let (env, id, _) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);
    client.bind_referrer(&referee, &referrer);

    // Referee is too new, so the action is rejected and nothing is recorded...
    assert_eq!(
        client.try_attribute_action(&referee, &action(&env, 9), &1_000),
        Err(Ok(Error::RefereeTooNew))
    );
    // ...and once the referee qualifies the same action id goes through once.
    env.ledger().with_mut(|l| l.sequence_number += 10);
    assert_eq!(client.attribute_action(&referee, &action(&env, 9), &1_000), 10);
    assert_eq!(
        client.try_attribute_action(&referee, &action(&env, 9), &1_000),
        Err(Ok(Error::DuplicateAction))
    );
}

#[test]
fn rejected_claims_do_not_consume_the_reward_budget() {
    let (env, id, admin) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    let referrer = Address::generate(&env);
    let referee = Address::generate(&env);
    bind_aged(&env, &client, &referee, &referrer);

    // Nothing accrued yet.
    client.fund_pool(&admin, &5);
    assert_eq!(client.try_claim_rewards(&referrer), Err(Ok(Error::NothingToClaim)));
    assert_eq!(client.reward_pool(), 5);

    // Accrue 10 against a pool of 5: claim is rejected, pool and ledger untouched.
    client.attribute_volume(&referee, &1_000);
    assert_eq!(client.try_claim_rewards(&referrer), Err(Ok(Error::InsufficientPool)));
    assert_eq!(client.reward_pool(), 5);
    let stats = client.get_stats(&referrer);
    assert_eq!((stats.rewards_accrued, stats.rewards_claimed), (10, 0));

    // Topping up the pool lets the same claim succeed exactly once.
    client.fund_pool(&admin, &5);
    assert_eq!(client.claim_rewards(&referrer), 10);
    assert_eq!(client.reward_pool(), 0);
    assert_eq!(client.try_claim_rewards(&referrer), Err(Ok(Error::NothingToClaim)));
}

#[test]
fn total_claims_never_exceed_the_funded_budget() {
    let (env, id, admin) = aged_setup();
    let client = ReferralClient::new(&env, &id);
    client.fund_pool(&admin, &25);

    let mut claimed_total = 0i128;
    for _ in 0..3 {
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);
        bind_aged(&env, &client, &referee, &referrer);
        client.attribute_volume(&referee, &1_000); // accrues 10
        if let Ok(Ok(amount)) = client.try_claim_rewards(&referrer) {
            claimed_total += amount;
        }
    }
    // 25 funded, 10 per claim: two claims fit, the third is rejected.
    assert_eq!(claimed_total, 20);
    assert_eq!(client.reward_pool(), 5);
}
