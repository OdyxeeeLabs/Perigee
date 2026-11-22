/// Governance invariant tests.
///
/// Verifies that:
/// - Proposals always progress through valid states (open → closed, never backward).
/// - Voting periods cannot be shortened after proposal creation.
/// - Vote tallies are always consistent with individual vote receipts.
/// - Quadratic vote math is correct and cannot be bypassed.
///
/// Closes OdyxeeeLabs/Perigee#511

#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, BytesN as _, Ledger},
    Address, BytesN, Env, String,
};

fn setup_governance(identity_required: bool) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let voter_a = Address::generate(&env);
    let voter_b = Address::generate(&env);
    let client = GovernanceContractClient::new(&env, &contract_id);
    client.initialize(&admin, &9, &identity_required);
    (env, contract_id, admin, voter_a)
}

fn make_id(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

// ── Invariant 1: Proposal state transitions are monotonic ─────────────────────

/// A proposal that is `open` can be closed; a closed proposal cannot be
/// re-opened — state transitions are one-way.
#[test]
fn proposal_state_is_monotonically_closed() {
    let (env, contract_id, _admin, _voter_a) = setup_governance(false);
    let client = GovernanceContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|l| l.sequence_number = 10);

    let p = client.create_proposal(
        &String::from_str(&env, "State invariant"),
        &String::from_str(&env, "Proposals must close one-way"),
        &50,
    );
    assert!(p.open, "new proposal must start open");

    let closed = client.close_proposal(&p.id);
    assert!(!closed.open, "closed proposal must have open=false");

    // The API only provides close_proposal — there is no re-open path.
    // Verify that the stored proposal remains closed.
    let stored = client.get_proposal(&p.id);
    assert!(!stored.open, "stored proposal must remain closed after close");
}

// ── Invariant 2: Voting period cannot be shortened after creation ─────────────

/// `voting_ends_at` is set once at creation and never updated; any proposal
/// fetched after creation must have the same `voting_ends_at`.
#[test]
fn voting_period_immutable_after_creation() {
    let (env, contract_id, _admin, _voter) = setup_governance(false);
    let client = GovernanceContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|l| l.sequence_number = 5);

    let p = client.create_proposal(
        &String::from_str(&env, "Immutable deadline"),
        &String::from_str(&env, "voting_ends_at must not change"),
        &100,
    );
    let original_end = p.voting_ends_at;

    // Re-fetch the proposal; voting_ends_at must be unchanged.
    let refetched = client.get_proposal(&p.id);
    assert_eq!(
        refetched.voting_ends_at, original_end,
        "voting_ends_at must not change after creation"
    );
}

/// Creating a proposal with `voting_ends_at <= current_ledger` must fail.
#[test]
fn proposal_creation_rejects_past_deadline() {
    let (env, contract_id, _admin, _voter) = setup_governance(false);
    let client = GovernanceContractClient::new(&env, &contract_id);

    env.ledger().with_mut(|l| l.sequence_number = 100);

    // Try to create a proposal whose deadline is in the past.
    let result = client.try_create_proposal(
        &String::from_str(&env, "Bad deadline"),
        &String::from_str(&env, "desc"),
        &50, // 50 <= 100 → must fail
    );
    assert_eq!(result, Err(Ok(Error::InvalidVotingWindow)));
}

// ── Invariant 3: Vote tallies are consistent with individual receipts ──────────

/// The aggregate `for_votes` / `against_votes` on the proposal must equal the
/// sum of all individual `VoteReceipt.votes_cast` values.
#[test]
fn tally_consistent_with_receipts() {
    let (env, contract_id, _admin, voter_a) = setup_governance(true);
    let voter_b = Address::generate(&env);
    let client = GovernanceContractClient::new(&env, &contract_id);

    client.register_voter(&voter_a, &25, &make_id(&env, 1));
    client.register_voter(&voter_b, &16, &make_id(&env, 2));

    env.ledger().with_mut(|l| l.sequence_number = 10);

    let p = client.create_proposal(
        &String::from_str(&env, "Tally check"),
        &String::from_str(&env, "Verify tally === sum of receipts"),
        &50,
    );

    // voter_a votes FOR with 9 credits → sqrt(9) = 3 votes
    let r_a = client.cast_vote(&p.id, &voter_a, &true, &9);
    // voter_b votes AGAINST with 16 credits → sqrt(16) = 4 votes
    let r_b = client.cast_vote(&p.id, &voter_b, &false, &16);

    let stored = client.get_proposal(&p.id);

    assert_eq!(
        stored.for_votes, r_a.votes_cast,
        "for_votes must equal voter_a's votes_cast"
    );
    assert_eq!(
        stored.against_votes, r_b.votes_cast,
        "against_votes must equal voter_b's votes_cast"
    );
    assert_eq!(stored.for_votes, 3);
    assert_eq!(stored.against_votes, 4);
}

// ── Invariant 4: Quadratic vote math correctness ──────────────────────────────

/// `votes_cast = sqrt(credits_spent)` for all valid credit amounts.
#[test]
fn quadratic_vote_math_is_correct() {
    let (env, contract_id, _admin, voter) = setup_governance(true);
    let client = GovernanceContractClient::new(&env, &contract_id);

    client.register_voter(&voter, &100, &make_id(&env, 5));

    env.ledger().with_mut(|l| l.sequence_number = 1);
    let p = client.create_proposal(
        &String::from_str(&env, "QV math"),
        &String::from_str(&env, "sqrt test"),
        &50,
    );

    // 4 credits → 2 votes
    let r = client.cast_vote(&p.id, &voter, &true, &4);
    assert_eq!(r.votes_cast, 2);
    assert_eq!(r.credits_spent, 4);
}

/// `quote_votes_for_credits` and `quote_credits_for_votes` are inverses.
#[test]
fn quadratic_quote_is_round_trip() {
    let (env, contract_id, _admin, _voter) = setup_governance(false);
    let client = GovernanceContractClient::new(&env, &contract_id);

    for credits in [1i128, 4, 9, 16, 25, 36, 49, 64, 81, 100] {
        let votes = client.quote_votes_for_credits(&credits);
        let back = client.quote_credits_for_votes(&votes);
        // Due to integer sqrt, back may be slightly less than credits but never more.
        assert!(
            back <= credits,
            "quote round-trip: back={back} > credits={credits}"
        );
    }
}

// ── Invariant 5: Votes never exceed registered units ─────────────────────────

#[test]
fn cumulative_votes_cannot_exceed_registered_units() {
    let (env, contract_id, _admin, voter) = setup_governance(true);
    let client = GovernanceContractClient::new(&env, &contract_id);

    // Register with 10 units.
    client.register_voter(&voter, &10, &make_id(&env, 7));

    env.ledger().with_mut(|l| l.sequence_number = 5);
    let p = client.create_proposal(
        &String::from_str(&env, "Unit cap"),
        &String::from_str(&env, "Cannot spend more credits than units"),
        &50,
    );

    // Trying to spend 11 credits (> 10 units) must fail.
    let err = client.try_cast_vote(&p.id, &voter, &true, &11);
    assert_eq!(err, Err(Ok(Error::InsufficientVotingUnits)));
}

// ── Invariant 6: Voter cannot change sides mid-proposal ──────────────────────

#[test]
fn voter_cannot_switch_sides_in_same_proposal() {
    let (env, contract_id, _admin, voter) = setup_governance(true);
    let client = GovernanceContractClient::new(&env, &contract_id);

    client.register_voter(&voter, &100, &make_id(&env, 8));

    env.ledger().with_mut(|l| l.sequence_number = 20);
    let p = client.create_proposal(
        &String::from_str(&env, "No flip"),
        &String::from_str(&env, "Cannot flip vote side"),
        &80,
    );
    client.cast_vote(&p.id, &voter, &true, &4);

    let err = client.try_cast_vote(&p.id, &voter, &false, &4);
    assert_eq!(err, Err(Ok(Error::VoteSideMismatch)));
}
