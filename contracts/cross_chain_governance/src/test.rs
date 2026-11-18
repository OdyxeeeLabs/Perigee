use super::*;
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    vec, IntoVal,
};

#[contract]
pub struct MockTarget;

#[contractimpl]
impl MockTarget {
    pub fn upd_val(env: Env, val: u32) -> u32 {
        env.storage().instance().set(&symbol_short!("val"), &val);
        val
    }

    pub fn get_val(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("val"))
            .unwrap_or(0)
    }
}

fn setup_test(
    env: &Env,
    local_chain_id: u32,
    min_delay: u64,
) -> (
    Address,
    CrossChainGovernanceClient<'_>,
    Address,
    MockTargetClient<'_>,
) {
    let admin = Address::generate(env);
    let contract_id = env.register(CrossChainGovernance, ());
    let client = CrossChainGovernanceClient::new(env, &contract_id);
    client.initialize(&admin, &local_chain_id, &min_delay);

    let target_id = env.register(MockTarget, ());
    let target_client = MockTargetClient::new(env, &target_id);

    (admin, client, target_id, target_client)
}

#[test]
fn test_initialization_and_config() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, _, _) = setup_test(&env, 1, 0);

    let cfg = client.get_config();
    assert_eq!(cfg.admin, admin);
    assert_eq!(cfg.local_chain_id, 1);
    assert_eq!(cfg.min_execution_delay, 0);
    assert!(!cfg.paused);

    assert!(client.is_bridge_protocol_enabled(&BridgeProtocol::Native));
    assert!(client.is_bridge_protocol_enabled(&BridgeProtocol::LayerZero));
    assert!(client.is_bridge_protocol_enabled(&BridgeProtocol::Wormhole));
}

#[test]
fn test_admin_operations() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, client, _, _) = setup_test(&env, 1, 0);

    let new_admin = Address::generate(&env);
    client.transfer_admin(&new_admin);
    assert_eq!(client.get_config().admin, new_admin);

    client.set_execution_delay(&3600);
    assert_eq!(client.get_config().min_execution_delay, 3600);

    client.set_paused(&true);
    assert!(client.get_config().paused);

    client.set_bridge_protocol(&BridgeProtocol::Native, &false);
    assert!(!client.is_bridge_protocol_enabled(&BridgeProtocol::Native));
}

#[test]
fn test_layerzero_remote_execution_immediate() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);

    let source_chain_id = 200u32;
    let remote_sender = admin.to_xdr(&env);
    client.set_trusted_sender(&source_chain_id, &remote_sender, &true);
    assert!(client.is_trusted_sender(&source_chain_id, &remote_sender));

    let function = symbol_short!("upd_val");
    let args: Vec<Val> = vec![&env, 42u32.into_val(&env)];
    let execution_id = BytesN::from_array(&env, &[7u8; 32]);
    let payload = client.encode_execution_payload(&target_id, &function, &args, &execution_id);

    let message = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 100,
        sequence: 1,
        sender: remote_sender,
        action_type: GovernanceActionType::RemoteExecution,
        payload,
        timestamp: env.ledger().timestamp(),
    };

    let proof = client.layerzero_proof(&source_chain_id, &100);
    let status = client.receive_and_execute(
        &message,
        &proof,
        &target_id,
        &function,
        &args,
        &execution_id,
    );
    assert_eq!(status, ExecutionStatus::Executed);
    assert_eq!(target_client.get_val(), 42);

    let rec = client.get_execution(&execution_id).unwrap();
    assert_eq!(rec.status, ExecutionStatus::Executed);
    assert_eq!(rec.source_chain_id, source_chain_id);
}

#[test]
fn test_wormhole_remote_execution_queued_timelock() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 500);

    let source_chain_id = 300u32;
    let remote_sender = admin.to_xdr(&env);
    client.set_trusted_sender(&source_chain_id, &remote_sender, &true);

    let function = symbol_short!("upd_val");
    let args: Vec<Val> = vec![&env, 99u32.into_val(&env)];
    let execution_id = BytesN::from_array(&env, &[9u8; 32]);
    let payload = client.encode_execution_payload(&target_id, &function, &args, &execution_id);

    let message = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 100,
        sequence: 1,
        sender: remote_sender,
        action_type: GovernanceActionType::RemoteExecution,
        payload,
        timestamp: 1_000,
    };

    let proof = client.wormhole_proof(&13, &source_chain_id);
    let status = client.receive_and_execute(
        &message,
        &proof,
        &target_id,
        &function,
        &args,
        &execution_id,
    );
    assert_eq!(status, ExecutionStatus::Queued);
    assert_eq!(target_client.get_val(), 0); // Not executed yet

    // Execution not ready before delay elapses
    let not_ready = client.try_execute_queued(&execution_id);
    assert!(not_ready.is_err());

    // Advance time past delay
    env.ledger().set_timestamp(1_501);
    client.execute_queued(&execution_id);
    assert_eq!(target_client.get_val(), 99);

    let rec = client.get_execution(&execution_id).unwrap();
    assert_eq!(rec.status, ExecutionStatus::Executed);
}

#[test]
fn test_cancel_queued_execution() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, target_id, _) = setup_test(&env, 100, 500);

    let source_chain_id = 300u32;
    let remote_sender = admin.to_xdr(&env);
    client.set_trusted_sender(&source_chain_id, &remote_sender, &true);

    let function = symbol_short!("upd_val");
    let args: Vec<Val> = vec![&env, 77u32.into_val(&env)];
    let execution_id = BytesN::from_array(&env, &[11u8; 32]);
    let payload = client.encode_execution_payload(&target_id, &function, &args, &execution_id);

    let message = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 100,
        sequence: 1,
        sender: remote_sender,
        action_type: GovernanceActionType::RemoteExecution,
        payload,
        timestamp: 1_000,
    };

    let proof = client.native_proof();
    client.receive_and_execute(
        &message,
        &proof,
        &target_id,
        &function,
        &args,
        &execution_id,
    );

    client.cancel_queued(&execution_id);
    let rec = client.get_execution(&execution_id).unwrap();
    assert_eq!(rec.status, ExecutionStatus::Cancelled);

    // Cannot execute cancelled
    env.ledger().set_timestamp(1_600);
    assert!(client.try_execute_queued(&execution_id).is_err());
}

#[test]
fn test_state_sync_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, _, _) = setup_test(&env, 10, 0);

    let source_chain_id = 20u32;
    let remote_sender = admin.to_xdr(&env);
    client.set_trusted_sender(&source_chain_id, &remote_sender, &true);

    let proposal_id = 42u32;
    let status = 2u32; // Passed
    let for_votes = 100_000i128;
    let against_votes = 5_000i128;
    let voting_closed = true;

    let payload = client.encode_proposal_payload(
        &proposal_id,
        &status,
        &for_votes,
        &against_votes,
        &voting_closed,
    );

    let message = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 10,
        sequence: 1,
        sender: remote_sender,
        action_type: GovernanceActionType::ProposalStateSync,
        payload,
        timestamp: env.ledger().timestamp(),
    };

    let proof = client.layerzero_proof(&source_chain_id, &10);
    client.sync_proposal_state(
        &message,
        &proof,
        &proposal_id,
        &status,
        &for_votes,
        &against_votes,
        &voting_closed,
    );

    let synced = client.get_synced_proposal(&source_chain_id, &proposal_id).unwrap();
    assert_eq!(synced.proposal_id, proposal_id);
    assert_eq!(synced.status, status);
    assert_eq!(synced.for_votes, for_votes);
    assert_eq!(synced.against_votes, against_votes);
    assert!(synced.voting_closed);
}

#[test]
fn test_state_sync_voting_power() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, _, _) = setup_test(&env, 10, 0);

    let source_chain_id = 30u32;
    let remote_sender = admin.to_xdr(&env);
    client.set_trusted_sender(&source_chain_id, &remote_sender, &true);

    let voter = Address::generate(&env);
    let voting_units = 500_000i128;
    let snapshot_id = 101u64;

    let payload = client.encode_voting_power_payload(&voter, &voting_units, &snapshot_id);

    let message = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 10,
        sequence: 1,
        sender: remote_sender,
        action_type: GovernanceActionType::VotingPowerSync,
        payload,
        timestamp: env.ledger().timestamp(),
    };

    let proof = client.wormhole_proof(&15, &source_chain_id);
    client.sync_voting_power(
        &message,
        &proof,
        &voter,
        &voting_units,
        &snapshot_id,
    );

    let synced = client
        .get_synced_voting_power(&source_chain_id, &voter, &snapshot_id)
        .unwrap();
    assert_eq!(synced.voter, voter);
    assert_eq!(synced.voting_units, voting_units);
    assert_eq!(synced.snapshot_id, snapshot_id);
}

#[test]
fn test_state_sync_parameters_and_state_root() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, _, _) = setup_test(&env, 10, 0);

    let source_chain_id = 40u32;
    let remote_sender = admin.to_xdr(&env);
    client.set_trusted_sender(&source_chain_id, &remote_sender, &true);

    // Sync parameter
    let param_key = symbol_short!("fee_bps");
    let param_val = 25i128;
    let p_payload = client.encode_parameter_payload(&param_key, &param_val);
    let p_msg = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 10,
        sequence: 1,
        sender: remote_sender.clone(),
        action_type: GovernanceActionType::ParameterSync,
        payload: p_payload,
        timestamp: env.ledger().timestamp(),
    };
    client.sync_parameter(
        &p_msg,
        &client.native_proof(),
        &param_key,
        &param_val,
    );
    let synced_p = client.get_synced_parameter(&param_key).unwrap();
    assert_eq!(synced_p.param_value, 25);

    // Sync state root
    let root = BytesN::from_array(&env, &[0xab; 32]);
    let block_height = 1_000_000u64;
    let r_payload = client.encode_state_root_payload(&source_chain_id, &block_height, &root);
    let r_msg = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 10,
        sequence: 2,
        sender: remote_sender,
        action_type: GovernanceActionType::StateRootSync,
        payload: r_payload,
        timestamp: env.ledger().timestamp(),
    };
    client.sync_state_root(
        &r_msg,
        &client.native_proof(),
        &source_chain_id,
        &block_height,
        &root,
    );
    let synced_r = client.get_synced_state_root(&source_chain_id, &block_height).unwrap();
    assert_eq!(synced_r.root, root);
    let latest_r = client.get_latest_state_root(&source_chain_id).unwrap();
    assert_eq!(latest_r.block_height, block_height);
}

#[test]
fn test_outbound_dispatch_and_broadcast() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, client, target_id, _) = setup_test(&env, 1, 0);

    let destination_chain = 200u32;
    let exec_id = BytesN::from_array(&env, &[0xcd; 32]);

    // Outbound dispatch execution
    let msg1 = client.dispatch_remote_execution(
        &destination_chain,
        &BridgeProtocol::LayerZero,
        &target_id.to_xdr(&env),
        &symbol_short!("foo"),
        &Bytes::new(&env),
        &exec_id,
    );
    assert_eq!(msg1.sequence, 1);
    assert_eq!(msg1.destination_chain_id, destination_chain);

    // Outbound broadcast proposal
    let msg2 = client.broadcast_proposal_state(
        &destination_chain,
        &BridgeProtocol::Wormhole,
        &12,
        &1,
        &50_000,
        &1_000,
        &false,
    );
    assert_eq!(msg2.sequence, 2);

    assert_eq!(client.get_outbound_sequence(&destination_chain), 2);
}

#[test]
fn test_anti_replay_and_untrusted_sender() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, _) = setup_test(&env, 100, 0);

    let source_chain_id = 200u32;
    let untrusted_sender = admin.to_xdr(&env);

    let function = symbol_short!("upd_val");
    let args: Vec<Val> = vec![&env, 42u32.into_val(&env)];
    let execution_id = BytesN::from_array(&env, &[7u8; 32]);
    let payload = client.encode_execution_payload(&target_id, &function, &args, &execution_id);

    let message = CrossChainGovMessage {
        source_chain_id,
        destination_chain_id: 100,
        sequence: 1,
        sender: untrusted_sender.clone(),
        action_type: GovernanceActionType::RemoteExecution,
        payload,
        timestamp: env.ledger().timestamp(),
    };
    let proof = client.native_proof();

    // Rejects untrusted sender
    let res = client.try_receive_and_execute(
        &message,
        &proof,
        &target_id,
        &function,
        &args,
        &execution_id,
    );
    assert!(res.is_err());

    // Trust sender and succeed
    client.set_trusted_sender(&source_chain_id, &untrusted_sender, &true);
    let ok = client.receive_and_execute(
        &message,
        &proof,
        &target_id,
        &function,
        &args,
        &execution_id,
    );
    assert_eq!(ok, ExecutionStatus::Executed);

    // Replay with same message rejected
    let replay = client.try_receive_and_execute(
        &message,
        &proof,
        &target_id,
        &function,
        &args,
        &execution_id,
    );
    assert!(replay.is_err());
}

// ── #872 — replay protection ────────────────────────────────────────────────
// ── #873 — timeout and retry state model ────────────────────────────────────

#[contract]
pub struct FlakyTarget;

#[contractimpl]
impl FlakyTarget {
    pub fn set_ready(env: Env, ready: bool) {
        env.storage().instance().set(&symbol_short!("ready"), &ready);
    }

    /// Fails until `set_ready(true)` has been called, then records `v`.
    pub fn run(env: Env, v: u32) -> u32 {
        let ready: bool = env
            .storage()
            .instance()
            .get(&symbol_short!("ready"))
            .unwrap_or(false);
        if !ready {
            panic!("remote target not ready");
        }
        env.storage().instance().set(&symbol_short!("val"), &v);
        v
    }

    pub fn get_val(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("val"))
            .unwrap_or(0)
    }
}

struct Call {
    message: CrossChainGovMessage,
    args: Vec<Val>,
    execution_id: BytesN<32>,
}

/// Build a `RemoteExecution` message for `target.function(val)` from `source`.
fn call_for(
    env: &Env,
    client: &CrossChainGovernanceClient,
    target: &Address,
    function: &Symbol,
    source: u32,
    sender: &Bytes,
    sequence: u64,
    exec_byte: u8,
    val: u32,
) -> Call {
    let args: Vec<Val> = vec![env, val.into_val(env)];
    let execution_id = BytesN::from_array(env, &[exec_byte; 32]);
    let payload = client.encode_execution_payload(target, function, &args, &execution_id);
    Call {
        message: CrossChainGovMessage {
            source_chain_id: source,
            destination_chain_id: 100,
            sequence,
            sender: sender.clone(),
            action_type: GovernanceActionType::RemoteExecution,
            payload,
            timestamp: env.ledger().timestamp(),
        },
        args,
        execution_id,
    }
}

fn deliver(
    client: &CrossChainGovernanceClient,
    target: &Address,
    function: &Symbol,
    call: &Call,
    proof: &BridgeProof,
) -> Result<ExecutionStatus, Result<Error, soroban_sdk::InvokeError>> {
    match client.try_receive_and_execute(
        &call.message,
        proof,
        target,
        function,
        &call.args,
        &call.execution_id,
    ) {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(_)) => panic!("unexpected conversion error"),
        Err(e) => Err(e),
    }
}

#[test]
fn a_valid_message_executes_exactly_once_and_replays_change_nothing() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();
    let call = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 7, 42);

    assert_eq!(deliver(&client, &target_id, &f, &call, &proof), Ok(ExecutionStatus::Executed));
    assert_eq!(target_client.get_val(), 42);
    assert_eq!(client.get_inbound_sequence(&200), 1);

    // Replaying the identical message is a duplicate identifier.
    assert_eq!(
        deliver(&client, &target_id, &f, &call, &proof),
        Err(Ok(Error::MessageAlreadyProcessed))
    );
    assert_eq!(target_client.get_val(), 42);
    assert_eq!(client.get_inbound_sequence(&200), 1);
    let rec = client.get_execution(&call.execution_id).unwrap();
    assert_eq!(rec.status, ExecutionStatus::Executed);
}

#[test]
fn reusing_a_sequence_with_a_different_payload_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();

    let first = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 10);
    assert!(deliver(&client, &target_id, &f, &first, &proof).is_ok());

    // Different payload and execution id, same (source, sequence): not a new
    // message, so it must not execute.
    let second = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 2, 99);
    assert_eq!(
        deliver(&client, &target_id, &f, &second, &proof),
        Err(Ok(Error::InvalidSequence))
    );
    assert_eq!(target_client.get_val(), 10);
    assert!(client.get_execution(&second.execution_id).is_none());
}

#[test]
fn out_of_order_delivery_rejects_the_late_message_without_state_change() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();

    let seq1 = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 11);
    let seq2 = call_for(&env, &client, &target_id, &f, 200, &sender, 2, 2, 22);
    let seq3 = call_for(&env, &client, &target_id, &f, 200, &sender, 3, 3, 33);

    // 2 arrives first and is accepted (gaps are allowed)...
    assert!(deliver(&client, &target_id, &f, &seq2, &proof).is_ok());
    assert_eq!(target_client.get_val(), 22);
    // ...so the older 1 is now stale.
    assert_eq!(
        deliver(&client, &target_id, &f, &seq1, &proof),
        Err(Ok(Error::InvalidSequence))
    );
    assert_eq!(target_client.get_val(), 22);
    assert_eq!(client.get_inbound_sequence(&200), 2);
    assert!(client.get_execution(&seq1.execution_id).is_none());
    // A newer message still goes through.
    assert!(deliver(&client, &target_id, &f, &seq3, &proof).is_ok());
    assert_eq!(client.get_inbound_sequence(&200), 3);
}

#[test]
fn sequences_are_tracked_per_source_chain() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, _) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    client.set_trusted_sender(&300, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();

    let a = call_for(&env, &client, &target_id, &f, 200, &sender, 5, 1, 1);
    let b = call_for(&env, &client, &target_id, &f, 300, &sender, 1, 2, 2);
    assert!(deliver(&client, &target_id, &f, &a, &proof).is_ok());
    // Sequence 1 on chain 300 is fresh even though chain 200 is already at 5.
    assert!(deliver(&client, &target_id, &f, &b, &proof).is_ok());
    assert_eq!(client.get_inbound_sequence(&200), 5);
    assert_eq!(client.get_inbound_sequence(&300), 1);
}

#[test]
fn a_sender_trusted_on_one_chain_is_rejected_from_another() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();

    let from_other_chain = call_for(&env, &client, &target_id, &f, 300, &sender, 1, 1, 5);
    assert_eq!(
        deliver(&client, &target_id, &f, &from_other_chain, &proof),
        Err(Ok(Error::UntrustedRemoteSender))
    );
    assert_eq!(client.get_inbound_sequence(&300), 0);
    assert_eq!(target_client.get_val(), 0);
    assert!(client.get_execution(&from_other_chain.execution_id).is_none());

    // The rejection did not poison the legitimate source.
    let valid = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 2, 5);
    assert!(deliver(&client, &target_id, &f, &valid, &proof).is_ok());
}

#[test]
fn bridge_proofs_naming_the_wrong_source_chain_are_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let call = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 5);

    // LayerZero proof claims endpoint 999 as the source.
    assert_eq!(
        deliver(&client, &target_id, &f, &call, &client.layerzero_proof(&999, &100)),
        Err(Ok(Error::InvalidBridgeProof))
    );
    // Wormhole proof names a different emitter chain.
    assert_eq!(
        deliver(&client, &target_id, &f, &call, &client.wormhole_proof(&13, &999)),
        Err(Ok(Error::InvalidBridgeProof))
    );
    assert_eq!(target_client.get_val(), 0);
    assert_eq!(client.get_inbound_sequence(&200), 0);
    assert!(!client.is_message_processed(&client.compute_message_hash(
        &call.message,
        &client.layerzero_proof(&999, &100)
    )));

    // With a matching proof the same message is accepted once.
    assert!(deliver(&client, &target_id, &f, &call, &client.layerzero_proof(&200, &100)).is_ok());
}

#[test]
fn a_message_for_another_destination_chain_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 0);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();

    let mut call = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 5);
    call.message.destination_chain_id = 101;
    assert_eq!(
        deliver(&client, &target_id, &f, &call, &proof),
        Err(Ok(Error::InvalidChain))
    );
    assert_eq!(target_client.get_val(), 0);
    assert_eq!(client.get_inbound_sequence(&200), 0);
}

// ── #873 ─────────────────────────────────────────────────────────────────────

#[test]
fn queued_execution_boundary_is_inclusive_and_measured_in_seconds() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 500);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let call = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 8);

    assert_eq!(
        deliver(&client, &target_id, &f, &call, &client.native_proof()),
        Ok(ExecutionStatus::Queued)
    );
    let rec = client.get_execution(&call.execution_id).unwrap();
    // The delay is in ledger-timestamp seconds: 1_000 + 500.
    assert_eq!(rec.queued_at, 1_000);
    assert_eq!(rec.execute_after, 1_500);

    env.ledger().set_timestamp(1_499);
    assert!(client.try_execute_queued(&call.execution_id).is_err());
    assert_eq!(client.get_execution(&call.execution_id).unwrap().status, ExecutionStatus::Queued);
    assert_eq!(target_client.get_val(), 0);

    env.ledger().set_timestamp(1_500);
    client.execute_queued(&call.execution_id);
    assert_eq!(target_client.get_val(), 8);
    let rec = client.get_execution(&call.execution_id).unwrap();
    assert_eq!(rec.status, ExecutionStatus::Executed);
    assert_eq!(rec.executed_at, 1_500);
}

#[test]
fn a_finished_execution_cannot_be_executed_or_cancelled_again() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 100);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let call = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 8);
    deliver(&client, &target_id, &f, &call, &client.native_proof()).unwrap();

    env.ledger().set_timestamp(1_100);
    client.execute_queued(&call.execution_id);
    let done = client.get_execution(&call.execution_id).unwrap();

    // Duplicate acknowledgement: a second execute / cancel is rejected and the
    // record, including `executed_at`, is untouched.
    env.ledger().set_timestamp(2_000);
    assert!(client.try_execute_queued(&call.execution_id).is_err());
    assert!(client.try_cancel_queued(&call.execution_id).is_err());
    assert_eq!(client.get_execution(&call.execution_id).unwrap(), done);
    assert_eq!(target_client.get_val(), 8);
}

#[test]
fn a_cancelled_execution_is_terminal() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 100);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let call = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 8);
    deliver(&client, &target_id, &f, &call, &client.native_proof()).unwrap();

    client.cancel_queued(&call.execution_id);
    assert!(client.try_cancel_queued(&call.execution_id).is_err());
    env.ledger().set_timestamp(5_000);
    assert!(client.try_execute_queued(&call.execution_id).is_err());
    assert_eq!(client.get_execution(&call.execution_id).unwrap().status, ExecutionStatus::Cancelled);
    assert_eq!(target_client.get_val(), 0);
}

#[test]
fn a_failed_remote_call_stays_queued_and_retries_with_the_same_arguments() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, _target, _) = setup_test(&env, 100, 100);
    let flaky_id = env.register(FlakyTarget, ());
    let flaky = FlakyTargetClient::new(&env, &flaky_id);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("run");
    let call = call_for(&env, &client, &flaky_id, &f, 200, &sender, 1, 1, 61);
    deliver(&client, &flaky_id, &f, &call, &client.native_proof()).unwrap();
    let queued = client.get_execution(&call.execution_id).unwrap();

    // Ready on time, but the remote target fails: the whole call reverts, so the
    // record is still `Queued` and still eligible for retry.
    env.ledger().set_timestamp(1_100);
    assert!(client.try_execute_queued(&call.execution_id).is_err());
    assert_eq!(client.get_execution(&call.execution_id).unwrap(), queued);
    assert_eq!(flaky.get_val(), 0);

    // Retry after the target recovers executes the originally approved call.
    flaky.set_ready(&true);
    client.execute_queued(&call.execution_id);
    assert_eq!(flaky.get_val(), 61);
    let rec = client.get_execution(&call.execution_id).unwrap();
    assert_eq!(rec.status, ExecutionStatus::Executed);
    assert_eq!(rec.args, queued.args);
    assert_eq!(rec.target, queued.target);
    assert_eq!(rec.function, queued.function);
}

#[test]
fn redelivering_an_execution_id_with_new_arguments_cannot_change_the_approved_call() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);
    let (admin, client, target_id, target_client) = setup_test(&env, 100, 100);
    let sender = admin.to_xdr(&env);
    client.set_trusted_sender(&200, &sender, &true);
    let f = symbol_short!("upd_val");
    let proof = client.native_proof();

    let original = call_for(&env, &client, &target_id, &f, 200, &sender, 1, 1, 10);
    deliver(&client, &target_id, &f, &original, &proof).unwrap();
    let before = client.get_execution(&original.execution_id).unwrap();

    // A later message (fresh sequence) reuses the execution id with a different
    // argument. The attempt reverts as a whole: no overwrite, and the new
    // sequence is not consumed.
    let retry = call_for(&env, &client, &target_id, &f, 200, &sender, 2, 1, 999);
    assert_eq!(
        deliver(&client, &target_id, &f, &retry, &proof),
        Err(Ok(Error::ExecutionAlreadyExecuted))
    );
    assert_eq!(client.get_execution(&original.execution_id).unwrap(), before);
    assert_eq!(client.get_inbound_sequence(&200), 1);

    env.ledger().set_timestamp(1_100);
    client.execute_queued(&original.execution_id);
    assert_eq!(target_client.get_val(), 10);
}
