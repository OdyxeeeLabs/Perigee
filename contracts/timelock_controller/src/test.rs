#![cfg(test)]

use super::*;
use soroban_sdk::{
    contract, contractimpl, testutils::{Address as _, Ledger, LedgerInfo}, Address, Env,
    IntoVal, Symbol,
};

#[contract]
pub struct Target;

#[contracttype]
#[derive(Clone)]
enum TargetKey {
    Count,
}

#[contractimpl]
impl Target {
    pub fn add(env: Env, amount: i128) -> i128 {
        let current: i128 = env.storage().instance().get(&TargetKey::Count).unwrap_or(0);
        let next = current + amount;
        env.storage().instance().set(&TargetKey::Count, &next);
        next
    }

    pub fn count(env: Env) -> i128 {
        env.storage().instance().get(&TargetKey::Count).unwrap_or(0)
    }
}

struct Setup {
    env: Env,
    contract: Address,
    target: Address,
    proposer: Address,
    executor: Address,
    canceller: Address,
}

fn setup(delay: u64) -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp: 1_000,
        protocol_version: 22,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3110400,
    });
    let admin = Address::generate(&env);
    let proposer = Address::generate(&env);
    let executor = Address::generate(&env);
    let canceller = Address::generate(&env);
    let id = env.register(TimelockController, ());
    let client = TimelockControllerClient::new(&env, &id);
    client.initialize(&admin, &proposer, &executor, &canceller, &delay);
    let target = env.register(Target, ());
    Setup {
        env,
        contract: id,
        target,
        proposer,
        executor,
        canceller,
    }
}

impl Setup {
    fn client(&self) -> TimelockControllerClient<'_> {
        TimelockControllerClient::new(&self.env, &self.contract)
    }
}

fn args(env: &Env, amount: i128) -> Vec<Val> {
    Vec::from_array(env, [amount.into_val(env)])
}

fn salt(env: &Env, value: u8) -> BytesN<32> {
    BytesN::from_array(env, &[value; 32])
}

fn advance_to(env: &Env, timestamp: u64) {
    env.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 22,
        sequence_number: 2,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3110400,
    });
}

fn queue_add(setup: &Setup, amount: i128, salt_value: u8) -> Operation {
    setup.client().queue(
        &setup.proposer,
        &setup.target,
        &Symbol::new(&setup.env, "add"),
        &args(&setup.env, amount),
        &salt(&setup.env, salt_value),
    )
}

#[test]
fn queues_and_executes_only_after_minimum_delay() {
    let setup = setup(60);
    let operation = queue_add(&setup, 7, 1);
    assert_eq!(operation.ready_at, 1_060);
    assert_eq!(operation.status, OperationStatus::Pending);

    assert_eq!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotReady))
    );
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 0);

    advance_to(&setup.env, operation.ready_at);
    setup.client().execute(&setup.executor, &operation.id);
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 7);
    assert_eq!(
        setup.client().operation(&operation.id).status,
        OperationStatus::Done
    );
    assert_eq!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotPending))
    );
}

#[test]
fn cancellation_blocks_execution_and_allows_requeue_with_same_salt() {
    let setup = setup(10);
    let operation = queue_add(&setup, 4, 2);
    setup.client().cancel(&setup.canceller, &operation.id);
    assert_eq!(
        setup.client().operation(&operation.id).status,
        OperationStatus::Canceled
    );
    assert_eq!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotPending))
    );

    let requeued = queue_add(&setup, 4, 2);
    assert_eq!(requeued.id, operation.id);
    assert_eq!(requeued.status, OperationStatus::Pending);
}

#[test]
fn operation_hash_commits_to_target_call_arguments_and_salt() {
    let setup = setup(30);
    let first = queue_add(&setup, 1, 3);
    let second = setup.client().queue(
        &setup.proposer,
        &setup.target,
        &Symbol::new(&setup.env, "add"),
        &args(&setup.env, 2),
        &salt(&setup.env, 3),
    );
    assert_ne!(first.id, second.id);
    assert_eq!(
        setup.client().hash_operation(
            &setup.target,
            &Symbol::new(&setup.env, "add"),
            &args(&setup.env, 1),
            &salt(&setup.env, 3),
        ),
        first.id
    );
}

#[test]
fn duplicate_pending_and_completed_operations_cannot_be_replayed() {
    let setup = setup(1);
    let operation = queue_add(&setup, 3, 4);
    assert_eq!(
        setup.client().try_queue(
            &setup.proposer,
            &setup.target,
            &Symbol::new(&setup.env, "add"),
            &args(&setup.env, 3),
            &salt(&setup.env, 4),
        ),
        Err(Ok(Error::OperationAlreadyExists))
    );

    advance_to(&setup.env, operation.ready_at);
    setup.client().execute(&setup.executor, &operation.id);
    assert_eq!(
        setup.client().try_queue(
            &setup.proposer,
            &setup.target,
            &Symbol::new(&setup.env, "add"),
            &args(&setup.env, 3),
            &salt(&setup.env, 4),
        ),
        Err(Ok(Error::OperationAlreadyExists))
    );
}

#[test]
fn delay_and_roles_are_admin_configurable() {
    let setup = setup(10);
    let admin = setup.client().config().admin;
    setup.client().set_min_delay(&admin, &90);
    assert_eq!(setup.client().config().min_delay, 90);
    assert_eq!(
        setup.client().try_set_min_delay(&admin, &0),
        Err(Ok(Error::InvalidDelay))
    );

    let next_executor = Address::generate(&setup.env);
    setup.client().set_roles(
        &admin,
        &setup.proposer,
        &next_executor,
        &setup.canceller,
    );
    assert_eq!(setup.client().config().executor, next_executor);
}

#[test]
fn queue_cancel_and_execute_require_their_designated_roles() {
    let setup = setup(10);
    let stranger = Address::generate(&setup.env);
    assert_eq!(
        setup.client().try_queue(
            &stranger,
            &setup.target,
            &Symbol::new(&setup.env, "add"),
            &args(&setup.env, 1),
            &salt(&setup.env, 5),
        ),
        Err(Ok(Error::Unauthorized))
    );

    let operation = queue_add(&setup, 1, 6);
    assert_eq!(
        setup.client().try_cancel(&stranger, &operation.id),
        Err(Ok(Error::Unauthorized))
    );
    advance_to(&setup.env, operation.ready_at);
    assert_eq!(
        setup.client().try_execute(&stranger, &operation.id),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn initialization_requires_positive_delay_and_is_single_use() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let proposer = Address::generate(&env);
    let executor = Address::generate(&env);
    let canceller = Address::generate(&env);
    let id = env.register(TimelockController, ());
    let client = TimelockControllerClient::new(&env, &id);

    assert_eq!(
        client.try_initialize(&admin, &proposer, &executor, &canceller, &0),
        Err(Ok(Error::InvalidDelay))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &proposer,
            &executor,
            &canceller,
            &(MAX_DELAY_SECONDS + 1),
        ),
        Err(Ok(Error::InvalidDelay))
    );
    client.initialize(&admin, &proposer, &executor, &canceller, &1);
    assert_eq!(
        client.try_initialize(&admin, &proposer, &executor, &canceller, &1),
        Err(Ok(Error::AlreadyInitialized))
    );
}
