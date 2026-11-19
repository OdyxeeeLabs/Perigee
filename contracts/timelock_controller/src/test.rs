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

    assert!(matches!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotReady))
    ));
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 0);

    advance_to(&setup.env, operation.ready_at);
    setup.client().execute(&setup.executor, &operation.id);
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 7);
    assert_eq!(
        setup.client().operation(&operation.id).status,
        OperationStatus::Done
    );
    assert!(matches!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotPending))
    ));
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
    assert!(matches!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotPending))
    ));

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
    assert!(matches!(
        setup.client().try_execute(&stranger, &operation.id),
        Err(Ok(Error::Unauthorized))
    ));
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

// ---------------------------------------------------------------------------
// Replay policy: a completed or cancelled operation cannot be replayed, and
// the operation's caller, target, payload, and timing are represented per the
// documented scheme (identifier = hash of target, function, args, salt; the
// record additionally stores proposer, queued_at, and ready_at).
// ---------------------------------------------------------------------------

#[test]
fn operation_id_binds_to_target_function_args_and_salt() {
    let setup = setup(30);
    let target = setup.target.clone();
    let other_target = setup.env.register(Target, ());
    let add = Symbol::new(&setup.env, "add");
    let other_function = Symbol::new(&setup.env, "count");
    let one = args(&setup.env, 1);
    let two = args(&setup.env, 2);
    let salt_a = salt(&setup.env, 3);
    let salt_b = salt(&setup.env, 4);

    let base = setup
        .client()
        .hash_operation(&target, &add, &one, &salt_a);

    // Every field that defines the action changes the identifier.
    assert_ne!(
        setup
            .client()
            .hash_operation(&other_target, &add, &one, &salt_a),
        base
    );
    assert_ne!(
        setup
            .client()
            .hash_operation(&target, &other_function, &one, &salt_a),
        base
    );
    assert_ne!(
        setup
            .client()
            .hash_operation(&target, &add, &two, &salt_a),
        base
    );
    assert_ne!(
        setup
            .client()
            .hash_operation(&target, &add, &one, &salt_b),
        base
    );

    // The identifier queue stores matches the documented derivation.
    let queued = setup
        .client()
        .queue(&setup.proposer, &target, &add, &one, &salt_a);
    assert_eq!(queued.id, base);
}

#[test]
fn queued_record_represents_caller_target_payload_and_timing() {
    let setup = setup(60);
    let operation = setup.client().queue(
        &setup.proposer,
        &setup.target,
        &Symbol::new(&setup.env, "add"),
        &args(&setup.env, 7),
        &salt(&setup.env, 9),
    );
    let record = setup.client().operation(&operation.id);

    // Caller.
    assert_eq!(record.proposer, setup.proposer);
    // Target.
    assert_eq!(record.target, setup.target);
    // Payload: function, arguments, and replay salt.
    assert_eq!(record.function, Symbol::new(&setup.env, "add"));
    assert_eq!(record.args, args(&setup.env, 7));
    assert_eq!(record.salt, salt(&setup.env, 9));
    // Timing: queued at the ledger timestamp and ready after the delay.
    assert_eq!(record.queued_at, 1_000);
    assert_eq!(record.ready_at, 1_060);
    assert_eq!(record.status, OperationStatus::Pending);
    assert_eq!(record.id, operation.id);
}

#[test]
fn identical_payloads_need_distinct_salt_to_coexist() {
    let setup = setup(10);
    let first = queue_add(&setup, 5, 10);
    // Same target/function/args with a different salt is a separate operation.
    let second = queue_add(&setup, 5, 11);
    assert_ne!(first.id, second.id);

    advance_to(&setup.env, first.ready_at);
    setup.client().execute(&setup.executor, &first.id);
    setup.client().execute(&setup.executor, &second.id);
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 10);
    assert_eq!(
        setup.client().operation(&first.id).status,
        OperationStatus::Done
    );
    assert_eq!(
        setup.client().operation(&second.id).status,
        OperationStatus::Done
    );
}

#[test]
fn completed_operation_cannot_be_replayed() {
    let setup = setup(1);
    let operation = queue_add(&setup, 3, 12);
    advance_to(&setup.env, operation.ready_at);
    setup.client().execute(&setup.executor, &operation.id);
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 3);

    // Re-executing the stale identifier does not repeat the call.
    assert!(matches!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotPending))
    ));
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 3);

    // A completed operation stays terminal: it cannot be re-queued under the
    // same identifier, so the prior execution cannot be revived.
    assert!(matches!(
        setup.client().try_queue(
            &setup.proposer,
            &setup.target,
            &Symbol::new(&setup.env, "add"),
            &args(&setup.env, 3),
            &salt(&setup.env, 12),
        ),
        Err(Ok(Error::OperationAlreadyExists))
    ));
}

#[test]
fn cancelled_operation_cannot_be_replayed_even_past_its_ready_time() {
    let setup = setup(1);
    let operation = queue_add(&setup, 8, 13);
    setup.client().cancel(&setup.canceller, &operation.id);
    assert_eq!(
        setup.client().operation(&operation.id).status,
        OperationStatus::Canceled
    );

    // Reaching the stale ready_at does not let the canceled call execute.
    advance_to(&setup.env, operation.ready_at);
    assert!(matches!(
        setup
            .client()
            .try_execute(&setup.executor, &operation.id),
        Err(Ok(Error::OperationNotPending))
    ));
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 0);

    // Cancellation is terminal: a canceled op cannot be canceled again.
    assert!(matches!(
        setup
            .client()
            .try_cancel(&setup.canceller, &operation.id),
        Err(Ok(Error::OperationNotPending))
    ));
}

#[test]
fn completed_operations_cannot_be_canceled() {
    let setup = setup(1);
    let operation = queue_add(&setup, 2, 14);
    advance_to(&setup.env, operation.ready_at);
    setup.client().execute(&setup.executor, &operation.id);

    // Cancel requires Pending, so a completed op stays Done and cannot be
    // flipped to Canceled (which would reopen re-queueing under its id).
    assert!(matches!(
        setup
            .client()
            .try_cancel(&setup.canceller, &operation.id),
        Err(Ok(Error::OperationNotPending))
    ));
    assert_eq!(
        setup.client().operation(&operation.id).status,
        OperationStatus::Done
    );
}

#[test]
fn requeued_operation_replaces_canceled_record_with_fresh_timing() {
    let setup = setup(60);
    let original = queue_add(&setup, 6, 15);
    assert_eq!(original.ready_at, 1_060);
    setup.client().cancel(&setup.canceller, &original.id);

    // The stale identifier now refers to a canceled record; even past the old
    // ready time it cannot be executed.
    advance_to(&setup.env, original.ready_at);
    assert!(matches!(
        setup
            .client()
            .try_execute(&setup.executor, &original.id),
        Err(Ok(Error::OperationNotPending))
    ));

    // Re-queueing the same payload/salt reuses the identifier but binds it to
    // fresh timing, replacing the canceled record entirely.
    advance_to(&setup.env, 5_000);
    let requeued = queue_add(&setup, 6, 15);
    assert_eq!(requeued.id, original.id);
    assert_eq!(requeued.queued_at, 5_000);
    assert_eq!(requeued.ready_at, 5_060);
    assert_eq!(requeued.status, OperationStatus::Pending);

    let record = setup.client().operation(&requeued.id);
    assert_eq!(record.ready_at, 5_060);
    assert_eq!(record.queued_at, 5_000);
    assert_eq!(record.status, OperationStatus::Pending);

    advance_to(&setup.env, requeued.ready_at);
    setup.client().execute(&setup.executor, &requeued.id);
    assert_eq!(TargetClient::new(&setup.env, &setup.target).count(), 6);
    assert_eq!(
        setup.client().operation(&requeued.id).status,
        OperationStatus::Done
    );
}
