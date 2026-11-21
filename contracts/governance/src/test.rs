#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, BytesN as _, Ledger},
    Address, BytesN, Env, String, Symbol, TryFromVal, Val, Vec,
};

#[contract]
pub struct ModuleTarget;

#[contractimpl]
impl ModuleTarget {
    pub fn ping(_env: Env) -> u32 {
        42
    }
}

fn setup(identity_required: bool) -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(GovernanceContract, ());
    let admin = Address::generate(&env);
    let voter_a = Address::generate(&env);
    let voter_b = Address::generate(&env);

    let client = GovernanceContractClient::new(&env, &contract_id);
    client.initialize(&admin, &9, &identity_required);
    (env, contract_id, admin, voter_a, voter_b)
}

fn make_identity(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

#[test]
fn quadratic_votes_follow_square_root_curve() {
    let (env, contract_id, admin, voter, _) = setup(true);
    let client = GovernanceContractClient::new(&env, &contract_id);
    client.register_voter(&voter, &25, &make_identity(&env, 7));

    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 25;
    });

    let title = String::from_str(&env, "Ship quadratic voting");
    let description = String::from_str(&env, "Reduce whale influence in governance");
    let proposal = client.create_proposal(&title, &description, &60);

    let first = client.cast_vote(&proposal.id, &voter, &true, &9);
    assert_eq!(first.credits_spent, 9);
    assert_eq!(first.votes_cast, 3);

    let second = client.cast_vote(&proposal.id, &voter, &true, &7);
    assert_eq!(second.credits_spent, 16);
    assert_eq!(second.votes_cast, 4);

    let stored = client.get_proposal(&proposal.id);
    assert_eq!(stored.for_votes, 4);
    assert_eq!(stored.against_votes, 0);

    let quote = client.quote_votes_for_credits(&25);
    assert_eq!(quote, 5);

    let cost = client.quote_credits_for_votes(&5);
    assert_eq!(cost, 25);

    let _ = admin;
}

#[test]
fn split_accounts_below_threshold_are_rejected() {
    let (env, contract_id, _admin, voter, _) = setup(false);
    let client = GovernanceContractClient::new(&env, &contract_id);

    let err = client.try_register_voter(&voter, &4, &make_identity(&env, 1));
    assert_eq!(err, Err(Ok(Error::InsufficientVotingUnits)));
}

#[test]
fn duplicate_identity_commitments_are_blocked() {
    let (env, contract_id, _admin, voter_a, voter_b) = setup(true);
    let client = GovernanceContractClient::new(&env, &contract_id);
    let identity = make_identity(&env, 9);

    client.register_voter(&voter_a, &16, &identity);
    let err = client.try_register_voter(&voter_b, &16, &identity);

    assert_eq!(err, Err(Ok(Error::IdentityAlreadyClaimed)));
}

#[test]
fn voter_cannot_flip_sides_on_same_proposal() {
    let (env, contract_id, _admin, voter, _) = setup(true);
    let client = GovernanceContractClient::new(&env, &contract_id);
    client.register_voter(&voter, &16, &make_identity(&env, 2));

    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 50;
    });

    let proposal = client.create_proposal(
        &String::from_str(&env, "Treasury reallocation"),
        &String::from_str(&env, "Test proposal"),
        &80,
    );
    // Voting 'for' first, then trying 'against' on the same proposal must fail
    client.cast_vote(&proposal.id, &voter, &true, &4);

    let err = client.try_cast_vote(&proposal.id, &voter, &false, &5);
    assert_eq!(err, Err(Ok(Error::VoteSideMismatch)));
}

#[test]
fn voter_cannot_vote_against_then_for_on_same_proposal() {
    let (env, contract_id, _admin, voter, _) = setup(true);
    let client = GovernanceContractClient::new(&env, &contract_id);
    client.register_voter(&voter, &16, &make_identity(&env, 3));

    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 50;
    });

    let proposal = client.create_proposal(
        &String::from_str(&env, "Treasury reallocation"),
        &String::from_str(&env, "Test proposal"),
        &80,
    );
    // Voting 'against' first, then trying 'for' on the same proposal must fail
    client.cast_vote(&proposal.id, &voter, &false, &4);

    let err = client.try_cast_vote(&proposal.id, &voter, &true, &5);
    assert_eq!(err, Err(Ok(Error::VoteSideMismatch)));
}

#[test]
fn votes_cannot_exceed_registered_units() {
    let (env, contract_id, _admin, voter, _) = setup(true);
    let client = GovernanceContractClient::new(&env, &contract_id);
    client.register_voter(&voter, &10, &make_identity(&env, 5));

    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 100;
    });

    let proposal = client.create_proposal(
        &String::from_str(&env, "Cap credits"),
        &String::from_str(&env, "Voting units snapshot should cap spend"),
        &150,
    );

    let err = client.try_cast_vote(&proposal.id, &voter, &true, &11);
    assert_eq!(err, Err(Ok(Error::InsufficientVotingUnits)));
}

#[test]
fn multisig_requires_distinct_authorized_signers_and_executes_modules() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(GovernanceContract, ());
    let signer_a = Address::generate(&env);
    let signer_b = Address::generate(&env);
    let outsider = Address::generate(&env);
    let mut admins = Vec::new(&env);
    admins.push_back(signer_a.clone());
    admins.push_back(signer_b.clone());

    let client = GovernanceContractClient::new(&env, &contract_id);
    client.initialize_with_signers(&admins, &2, &1, &false);

    let voter = Address::generate(&env);
    let identity = make_identity(&env, 12);
    let mut one_approval = Vec::new(&env);
    one_approval.push_back(signer_a.clone());
    assert_eq!(
        client.try_register_voter_with_approvals(&voter, &5, &identity, &one_approval),
        Err(Ok(Error::InsufficientSignatures))
    );

    let mut unauthorized_approvals = Vec::new(&env);
    unauthorized_approvals.push_back(signer_a.clone());
    unauthorized_approvals.push_back(outsider);
    assert_eq!(
        client.try_register_voter_with_approvals(&voter, &5, &identity, &unauthorized_approvals),
        Err(Ok(Error::Unauthorized))
    );
    let mut duplicate_approvals = Vec::new(&env);
    duplicate_approvals.push_back(signer_a.clone());
    duplicate_approvals.push_back(signer_a.clone());
    assert_eq!(
        client.try_register_voter_with_approvals(&voter, &5, &identity, &duplicate_approvals),
        Err(Ok(Error::InsufficientSignatures))
    );

    let mut approvals = Vec::new(&env);
    approvals.push_back(signer_a);
    approvals.push_back(signer_b);
    client.register_voter_with_approvals(&voter, &5, &identity, &approvals);
    assert_eq!(client.get_config().signature_threshold, 2);
    assert_eq!(client.get_config().admins.len(), 2);

    let target = env.register(ModuleTarget, ());
    assert_eq!(
        client.try_execute_transaction(
            &one_approval,
            &target,
            &Symbol::new(&env, "ping"),
            &Vec::<Val>::new(&env)
        ),
        Err(Ok(Error::InsufficientSignatures))
    );
    assert_eq!(
        client.try_execute_transaction(
            &unauthorized_approvals,
            &target,
            &Symbol::new(&env, "ping"),
            &Vec::<Val>::new(&env)
        ),
        Err(Ok(Error::Unauthorized))
    );
    let execution_result = client.execute_transaction(
        &approvals,
        &target,
        &Symbol::new(&env, "ping"),
        &Vec::<Val>::new(&env),
    );
    assert_eq!(u32::try_from_val(&env, &execution_result).unwrap(), 42);

    let module = Address::generate(&env);
    client.enable_module(&approvals, &module);
    assert!(client.is_module_enabled(&module));

    let result = client.execute_module(
        &module,
        &target,
        &Symbol::new(&env, "ping"),
        &Vec::<Val>::new(&env),
    );
    assert_eq!(u32::try_from_val(&env, &result).unwrap(), 42);

    client.disable_module(&approvals, &module);
    assert!(!client.is_module_enabled(&module));
    assert_eq!(
        client.try_execute_module(
            &module,
            &target,
            &Symbol::new(&env, "ping"),
            &Vec::<Val>::new(&env)
        ),
        Err(Ok(Error::ModuleNotEnabled))
    );
}

#[test]
fn multisig_threshold_changes_require_current_threshold() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(GovernanceContract, ());
    let signer_a = Address::generate(&env);
    let signer_b = Address::generate(&env);
    let signer_c = Address::generate(&env);
    let mut admins = Vec::new(&env);
    admins.push_back(signer_a.clone());
    admins.push_back(signer_b.clone());

    let client = GovernanceContractClient::new(&env, &contract_id);
    client.initialize_with_signers(&admins, &2, &1, &false);

    let mut approvals = Vec::new(&env);
    approvals.push_back(signer_a.clone());
    approvals.push_back(signer_b.clone());
    client.add_signer(&approvals, &signer_c);
    client.set_signature_threshold(&approvals, &3);
    assert_eq!(client.get_config().signature_threshold, 3);

    let voter = Address::generate(&env);
    assert_eq!(
        client.try_register_voter_with_approvals(&voter, &5, &make_identity(&env, 13), &approvals),
        Err(Ok(Error::InsufficientSignatures))
    );

    approvals.push_back(signer_c);
    client.register_voter_with_approvals(&voter, &5, &make_identity(&env, 13), &approvals);
}
