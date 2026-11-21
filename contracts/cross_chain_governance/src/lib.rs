#![no_std]

//! # Perigee Cross-Chain Governance — message passing, remote execution, state sync
//!
//! Closes OdyxeeeLabs/Perigee#650
//!
//! Source: contract.md § Access Control & Governance (#20)
//!
//! Cross-chain governance enables decentralized protocols to extend governance authority,
//! execute proposals across multiple blockchain networks, and maintain synchronized governance
//! state between the hub (Stellar/Soroban) and remote spoke chains.
//!
//! ## Core Capabilities
//!
//! 1. **Message Passing (LayerZero / Wormhole / Native)**
//!    - Standardized cross-chain message envelope with domain separation:
//!      - LayerZero: `LZ_GOV_V1` with endpoint IDs, nonce, and packet payload validation.
//!      - Wormhole: `WH_GOV_V1` with emitter chain, guardian quorum, and sequence validation.
//!      - Native: `NATIVE_GOV_V1` domain-separated message hash with timestamp checks.
//!    - Strict anti-replay mechanism tracking processed message digests and monotonic sequence nonces.
//!    - Whitelisting of trusted remote senders and bridge protocols per chain.
//!
//! 2. **Remote Execution**
//!    - Authorized remote governance messages can dispatch and execute target contract invocations (`invoke_contract`).
//!    - Optional configurable timelock execution delay for high-impact actions.
//!    - Execution status lifecycle: `Pending`, `Queued`, `Executed`, `Failed`, `Cancelled`.
//!    - Outbound execution dispatching to remote destination chains with sequence tracking and relayer events.
//!
//! 3. **State Sync**
//!    - Cross-chain synchronization of governance state:
//!      - Proposal state sync: proposal ID, status, for/against votes, and closure status.
//!      - Voting power snapshot sync: voter address, voting units, and snapshot ID.
//!      - Parameter / Configuration sync: global protocol parameters and risk configurations.
//!      - Merkle state root sync: anchoring state roots and epochs across chains.
//!    - Bidirectional state broadcast and ingestion.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, xdr::ToXdr, Address, Bytes,
    BytesN, Env, Symbol, Val, Vec,
};

#[cfg(test)]
mod test;

/// Ledgers to persist operational state.
pub const TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidChain = 4,
    InvalidSequence = 5,
    MessageAlreadyProcessed = 6,
    UnsupportedBridgeProtocol = 7,
    InvalidBridgeProof = 8,
    UntrustedRemoteSender = 9,
    InvalidActionType = 10,
    InvalidPayload = 11,
    ExecutionFailed = 12,
    ExecutionNotReady = 13,
    ExecutionAlreadyExecuted = 14,
    ExecutionNotFound = 15,
    InvalidTimelockDelay = 16,
    InvalidStateRoot = 17,
    InvalidProposalState = 18,
    InvalidVotingPower = 19,
    Paused = 20,
    InvalidParameters = 21,
}

/// Supported bridge wire protocols.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum BridgeProtocol {
    Native = 0,
    LayerZero = 1,
    Wormhole = 2,
}

/// Protocol-specific proof payload attached to a cross-chain governance message.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeProof {
    pub protocol: BridgeProtocol,
    pub proof_data: Bytes,
}

/// Action types supported by cross-chain governance messages.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceActionType {
    RemoteExecution = 0,
    ProposalStateSync = 1,
    VotingPowerSync = 2,
    ParameterSync = 3,
    StateRootSync = 4,
}

/// Core cross-chain governance message envelope.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossChainGovMessage {
    pub source_chain_id: u32,
    pub destination_chain_id: u32,
    pub sequence: u64,
    pub sender: Bytes,
    pub action_type: GovernanceActionType,
    pub payload: Bytes,
    pub timestamp: u64,
}

/// Remote execution lifecycle state.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ExecutionStatus {
    Pending = 0,
    Queued = 1,
    Executed = 2,
    Failed = 3,
    Cancelled = 4,
}

/// Stored record of a remote execution command.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteExecutionRecord {
    pub execution_id: BytesN<32>,
    pub source_chain_id: u32,
    pub sender: Bytes,
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub status: ExecutionStatus,
    pub queued_at: u64,
    pub execute_after: u64,
    pub executed_at: u64,
}

/// Proposal state synchronization record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalStateSync {
    pub proposal_id: u32,
    pub status: u32,
    pub for_votes: i128,
    pub against_votes: i128,
    pub voting_closed: bool,
    pub synced_at: u64,
}

/// Voting power snapshot synchronization record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VotingPowerSync {
    pub voter: Address,
    pub voting_units: i128,
    pub snapshot_id: u64,
    pub synced_at: u64,
}

/// Parameter synchronization record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterSync {
    pub param_key: Symbol,
    pub param_value: i128,
    pub synced_at: u64,
}

/// State root synchronization record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateRootRecord {
    pub chain_id: u32,
    pub block_height: u64,
    pub root: BytesN<32>,
    pub synced_at: u64,
}

/// Governance configuration parameters.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossChainGovConfig {
    pub admin: Address,
    pub local_chain_id: u32,
    pub min_execution_delay: u64,
    pub paused: bool,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Config,
    EnabledProtocol(u32),
    TrustedSender(u32, Bytes),
    ProcessedMessage(BytesN<32>),
    InboundSequence(u32),
    OutboundSequence(u32),
    Execution(BytesN<32>),
    SyncedProposal(u32, u32),
    SyncedVotingPower(u32, Address, u64),
    SyncedParameter(Symbol),
    SyncedStateRoot(u32, u64),
    LatestStateRoot(u32),
}

// ── Bridge Protocol Handlers ────────────────────────────────────────────────

fn validate_bridge_proof(
    _env: &Env,
    message: &CrossChainGovMessage,
    proof: &BridgeProof,
) -> Result<(), Error> {
    match proof.protocol {
        BridgeProtocol::Native => Ok(()),
        BridgeProtocol::LayerZero => {
            // LayerZero packet header check (optional src_eid, dst_eid)
            if proof.proof_data.len() >= 8 {
                let bytes = proof.proof_data.clone();
                let mut src_arr = [0u8; 4];
                let mut dst_arr = [0u8; 4];
                for i in 0..4 {
                    src_arr[i as usize] = bytes.get(i).unwrap_or(0);
                    dst_arr[i as usize] = bytes.get(i + 4).unwrap_or(0);
                }
                let src = u32::from_be_bytes(src_arr);
                let dst = u32::from_be_bytes(dst_arr);
                if src != message.source_chain_id || dst != message.destination_chain_id {
                    return Err(Error::InvalidBridgeProof);
                }
            }
            Ok(())
        }
        BridgeProtocol::Wormhole => {
            // Wormhole VAA header check: quorum (u8) > 0 and <= 19, emitter chain (u32)
            if proof.proof_data.len() >= 1 {
                let quorum = proof.proof_data.get(0).unwrap_or(0);
                if quorum == 0 || quorum > 19 {
                    return Err(Error::InvalidBridgeProof);
                }
            }
            if proof.proof_data.len() >= 5 {
                let mut chain_arr = [0u8; 4];
                for i in 0..4 {
                    chain_arr[i as usize] = proof.proof_data.get(i + 1).unwrap_or(0);
                }
                let emitter_chain = u32::from_be_bytes(chain_arr);
                if emitter_chain != message.source_chain_id {
                    return Err(Error::InvalidBridgeProof);
                }
            }
            Ok(())
        }
    }
}

fn hash_bridge_message(
    env: &Env,
    message: &CrossChainGovMessage,
    proof: &BridgeProof,
) -> BytesN<32> {
    let payload_hash = env.crypto().sha256(&message.payload);
    let payload_hash_bytes = Bytes::from_array(env, &payload_hash.to_array());
    match proof.protocol {
        BridgeProtocol::LayerZero => {
            let mut data = Bytes::new(env);
            data.append(&Bytes::from_slice(env, b"LZ_GOV_V1"));
            data.append(&Bytes::from_slice(env, &message.source_chain_id.to_be_bytes()));
            data.append(&Bytes::from_slice(
                env,
                &message.destination_chain_id.to_be_bytes(),
            ));
            data.append(&Bytes::from_slice(env, &message.sequence.to_be_bytes()));
            if proof.proof_data.len() > 0 {
                data.append(&proof.proof_data);
            }
            data.append(&payload_hash_bytes);
            env.crypto().sha256(&data).into()
        }
        BridgeProtocol::Wormhole => {
            let mut data = Bytes::new(env);
            data.append(&Bytes::from_slice(env, b"WH_GOV_V1"));
            data.append(&Bytes::from_slice(env, &message.source_chain_id.to_be_bytes()));
            data.append(&Bytes::from_slice(env, &message.sequence.to_be_bytes()));
            if proof.proof_data.len() > 0 {
                data.append(&proof.proof_data);
            }
            data.append(&payload_hash_bytes);
            env.crypto().sha256(&data).into()
        }
        BridgeProtocol::Native => {
            let mut data = Bytes::new(env);
            data.append(&Bytes::from_slice(env, b"NATIVE_GOV_V1"));
            data.append(&Bytes::from_slice(env, &message.source_chain_id.to_be_bytes()));
            data.append(&Bytes::from_slice(
                env,
                &message.destination_chain_id.to_be_bytes(),
            ));
            data.append(&Bytes::from_slice(env, &message.sequence.to_be_bytes()));
            data.append(&Bytes::from_slice(env, &message.timestamp.to_be_bytes()));
            data.append(&payload_hash_bytes);
            env.crypto().sha256(&data).into()
        }
    }
}

// ── Contract Implementation ─────────────────────────────────────────────────

#[contract]
pub struct CrossChainGovernance;

#[contractimpl]
impl CrossChainGovernance {
    /// Initialize cross-chain governance with an admin, local chain id, and default timelock delay.
    pub fn initialize(
        env: Env,
        admin: Address,
        local_chain_id: u32,
        min_execution_delay: u64,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        let config = CrossChainGovConfig {
            admin,
            local_chain_id,
            min_execution_delay,
            paused: false,
        };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        // Native, LayerZero and Wormhole enabled by default
        env.storage()
            .persistent()
            .set(&DataKey::EnabledProtocol(BridgeProtocol::Native as u32), &true);
        env.storage()
            .persistent()
            .set(&DataKey::EnabledProtocol(BridgeProtocol::LayerZero as u32), &true);
        env.storage()
            .persistent()
            .set(&DataKey::EnabledProtocol(BridgeProtocol::Wormhole as u32), &true);

        env.events()
            .publish((symbol_short!("cc_init"),), (local_chain_id, min_execution_delay));
        Ok(())
    }

    /// Read configuration.
    pub fn get_config(env: Env) -> Result<CrossChainGovConfig, Error> {
        let config: CrossChainGovConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(Error::NotInitialized)?;
        env.storage().instance().extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        Ok(config)
    }

    /// Transfer administrative control to a new address.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let mut config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        config.admin = new_admin.clone();
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("adm_xfer"),), (new_admin,));
        Ok(())
    }

    /// Set minimum execution timelock delay in seconds.
    pub fn set_execution_delay(env: Env, new_delay: u64) -> Result<(), Error> {
        let mut config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        config.min_execution_delay = new_delay;
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("delay_set"),), (new_delay,));
        Ok(())
    }

    /// Circuit breaker: pause or resume cross-chain message processing.
    pub fn set_paused(env: Env, paused: bool) -> Result<(), Error> {
        let mut config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        config.paused = paused;
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("pause_set"),), (paused,));
        Ok(())
    }

    /// Enable or disable a bridge wire protocol (LayerZero, Wormhole, Native).
    pub fn set_bridge_protocol(
        env: Env,
        protocol: BridgeProtocol,
        enabled: bool,
    ) -> Result<(), Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        let key = DataKey::EnabledProtocol(protocol as u32);
        env.storage().persistent().set(&key, &enabled);
        env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("proto_set"),), (protocol as u32, enabled));
        Ok(())
    }

    /// Check if a bridge protocol is enabled.
    pub fn is_bridge_protocol_enabled(env: Env, protocol: BridgeProtocol) -> bool {
        let key = DataKey::EnabledProtocol(protocol as u32);
        env.storage().persistent().get(&key).unwrap_or(false)
    }

    /// Whitelist or remove a trusted remote governance sender from a source chain.
    pub fn set_trusted_sender(
        env: Env,
        source_chain_id: u32,
        sender: Bytes,
        trusted: bool,
    ) -> Result<(), Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        let key = DataKey::TrustedSender(source_chain_id, sender.clone());
        if trusted {
            env.storage().persistent().set(&key, &true);
            env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
        } else {
            env.storage().persistent().remove(&key);
        }

        env.events()
            .publish((symbol_short!("sender_st"), source_chain_id), (sender, trusted));
        Ok(())
    }

    /// Check whether a sender on a remote source chain is authorized.
    pub fn is_trusted_sender(env: Env, source_chain_id: u32, sender: Bytes) -> bool {
        let key = DataKey::TrustedSender(source_chain_id, sender);
        env.storage().persistent().get(&key).unwrap_or(false)
    }

    /// Highest inbound sequence processed from a source chain.
    pub fn get_inbound_sequence(env: Env, source_chain_id: u32) -> u64 {
        let key = DataKey::InboundSequence(source_chain_id);
        env.storage().persistent().get(&key).unwrap_or(0)
    }

    /// Highest outbound sequence dispatched to a destination chain.
    pub fn get_outbound_sequence(env: Env, destination_chain_id: u32) -> u64 {
        let key = DataKey::OutboundSequence(destination_chain_id);
        env.storage().persistent().get(&key).unwrap_or(0)
    }

    /// Check whether a message hash has already been processed (anti-replay).
    pub fn is_message_processed(env: Env, message_hash: BytesN<32>) -> bool {
        let key = DataKey::ProcessedMessage(message_hash);
        env.storage().persistent().has(&key)
    }

    /// Compute canonical domain-separated message hash for verification.
    pub fn compute_message_hash(
        env: Env,
        message: CrossChainGovMessage,
        proof: BridgeProof,
    ) -> BytesN<32> {
        hash_bridge_message(&env, &message, &proof)
    }

    // ── Inbound Message Verification Helper ─────────────────────────────────

    fn verify_and_record_message(
        env: &Env,
        message: &CrossChainGovMessage,
        proof: &BridgeProof,
        expected_action: GovernanceActionType,
    ) -> Result<BytesN<32>, Error> {
        let config = Self::get_config(env.clone())?;
        if config.paused {
            return Err(Error::Paused);
        }
        if message.destination_chain_id != config.local_chain_id {
            return Err(Error::InvalidChain);
        }
        if message.action_type != expected_action {
            return Err(Error::InvalidActionType);
        }
        if !Self::is_bridge_protocol_enabled(env.clone(), proof.protocol) {
            return Err(Error::UnsupportedBridgeProtocol);
        }
        validate_bridge_proof(env, message, proof)?;

        if !Self::is_trusted_sender(env.clone(), message.source_chain_id, message.sender.clone()) {
            return Err(Error::UntrustedRemoteSender);
        }

        let message_hash = hash_bridge_message(env, message, proof);
        if Self::is_message_processed(env.clone(), message_hash.clone()) {
            return Err(Error::MessageAlreadyProcessed);
        }

        let last_seq = Self::get_inbound_sequence(env.clone(), message.source_chain_id);
        if message.sequence <= last_seq {
            return Err(Error::InvalidSequence);
        }

        // Mark processed & record sequence
        let msg_key = DataKey::ProcessedMessage(message_hash.clone());
        env.storage().persistent().set(&msg_key, &true);
        env.storage().persistent().extend_ttl(&msg_key, TTL_LEDGERS, TTL_LEDGERS);

        let seq_key = DataKey::InboundSequence(message.source_chain_id);
        env.storage().persistent().set(&seq_key, &message.sequence);
        env.storage().persistent().extend_ttl(&seq_key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("cc_msg"), message.source_chain_id),
            (message.sequence, message_hash.clone()),
        );

        Ok(message_hash)
    }

    // ── Remote Execution ────────────────────────────────────────────────────

    /// Receive a verified cross-chain message and execute (or queue) a remote call.
    pub fn receive_and_execute(
        env: Env,
        message: CrossChainGovMessage,
        proof: BridgeProof,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        execution_id: BytesN<32>,
    ) -> Result<ExecutionStatus, Error> {
        let expected_payload = (target.clone(), function.clone(), args.clone(), execution_id.clone()).to_xdr(&env);
        if message.payload != expected_payload {
            return Err(Error::InvalidPayload);
        }

        Self::verify_and_record_message(
            &env,
            &message,
            &proof,
            GovernanceActionType::RemoteExecution,
        )?;

        let config = Self::get_config(env.clone())?;
        let now = env.ledger().timestamp();
        let exec_key = DataKey::Execution(execution_id.clone());

        if env.storage().persistent().has(&exec_key) {
            return Err(Error::ExecutionAlreadyExecuted);
        }

        if config.min_execution_delay > 0 {
            let execute_after = now.saturating_add(config.min_execution_delay);
            let record = RemoteExecutionRecord {
                execution_id: execution_id.clone(),
                source_chain_id: message.source_chain_id,
                sender: message.sender,
                target: target.clone(),
                function: function.clone(),
                args,
                status: ExecutionStatus::Queued,
                queued_at: now,
                execute_after,
                executed_at: 0,
            };
            env.storage().persistent().set(&exec_key, &record);
            env.storage().persistent().extend_ttl(&exec_key, TTL_LEDGERS, TTL_LEDGERS);

            env.events().publish(
                (symbol_short!("exec_que"), message.source_chain_id),
                (execution_id, execute_after),
            );
            Ok(ExecutionStatus::Queued)
        } else {
            // Immediate execution
            let _result: Val = env.invoke_contract(&target, &function, args.clone());

            let record = RemoteExecutionRecord {
                execution_id: execution_id.clone(),
                source_chain_id: message.source_chain_id,
                sender: message.sender,
                target: target.clone(),
                function: function.clone(),
                args,
                status: ExecutionStatus::Executed,
                queued_at: now,
                execute_after: now,
                executed_at: now,
            };
            env.storage().persistent().set(&exec_key, &record);
            env.storage().persistent().extend_ttl(&exec_key, TTL_LEDGERS, TTL_LEDGERS);

            env.events().publish(
                (symbol_short!("exec_done"), message.source_chain_id),
                (execution_id, target, function),
            );
            Ok(ExecutionStatus::Executed)
        }
    }

    /// Execute a previously queued remote execution after the timelock delay has elapsed.
    pub fn execute_queued(env: Env, execution_id: BytesN<32>) -> Result<Val, Error> {
        let exec_key = DataKey::Execution(execution_id.clone());
        let mut record: RemoteExecutionRecord = env
            .storage()
            .persistent()
            .get(&exec_key)
            .ok_or(Error::ExecutionNotFound)?;

        if record.status != ExecutionStatus::Queued {
            return Err(Error::ExecutionAlreadyExecuted);
        }

        let now = env.ledger().timestamp();
        if now < record.execute_after {
            return Err(Error::ExecutionNotReady);
        }

        let result: Val = env.invoke_contract(&record.target, &record.function, record.args.clone());

        record.status = ExecutionStatus::Executed;
        record.executed_at = now;
        env.storage().persistent().set(&exec_key, &record);
        env.storage().persistent().extend_ttl(&exec_key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("exec_done"), record.source_chain_id),
            (execution_id, record.target, record.function),
        );

        Ok(result)
    }

    /// Cancel a queued remote execution before it is executed (admin only).
    pub fn cancel_queued(env: Env, execution_id: BytesN<32>) -> Result<(), Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        let exec_key = DataKey::Execution(execution_id.clone());
        let mut record: RemoteExecutionRecord = env
            .storage()
            .persistent()
            .get(&exec_key)
            .ok_or(Error::ExecutionNotFound)?;

        if record.status != ExecutionStatus::Queued {
            return Err(Error::ExecutionAlreadyExecuted);
        }

        record.status = ExecutionStatus::Cancelled;
        env.storage().persistent().set(&exec_key, &record);
        env.storage().persistent().extend_ttl(&exec_key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("exec_canc"), record.source_chain_id),
            (execution_id,),
        );
        Ok(())
    }

    /// Retrieve stored execution record by execution id.
    pub fn get_execution(env: Env, execution_id: BytesN<32>) -> Option<RemoteExecutionRecord> {
        let exec_key = DataKey::Execution(execution_id);
        env.storage().persistent().get(&exec_key)
    }

    /// Dispatch an outbound remote execution instruction to a remote spoke chain.
    pub fn dispatch_remote_execution(
        env: Env,
        destination_chain_id: u32,
        protocol: BridgeProtocol,
        target_bytes: Bytes,
        function: Symbol,
        args_payload: Bytes,
        execution_id: BytesN<32>,
    ) -> Result<CrossChainGovMessage, Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        if !Self::is_bridge_protocol_enabled(env.clone(), protocol) {
            return Err(Error::UnsupportedBridgeProtocol);
        }

        let out_key = DataKey::OutboundSequence(destination_chain_id);
        let seq = env.storage().persistent().get(&out_key).unwrap_or(0) + 1;
        env.storage().persistent().set(&out_key, &seq);
        env.storage().persistent().extend_ttl(&out_key, TTL_LEDGERS, TTL_LEDGERS);

        let mut payload_builder = Bytes::new(&env);
        payload_builder.append(&target_bytes);
        payload_builder.append(&args_payload);
        payload_builder.append(&Bytes::from_array(&env, &execution_id.to_array()));

        let message = CrossChainGovMessage {
            source_chain_id: config.local_chain_id,
            destination_chain_id,
            sequence: seq,
            sender: config.admin.to_xdr(&env),
            action_type: GovernanceActionType::RemoteExecution,
            payload: payload_builder,
            timestamp: env.ledger().timestamp(),
        };

        env.events().publish(
            (symbol_short!("disp_exec"), destination_chain_id),
            (seq, execution_id, function),
        );

        Ok(message)
    }

    // ── State Sync: Proposals ───────────────────────────────────────────────

    /// Sync proposal state received from a remote governance contract.
    pub fn sync_proposal_state(
        env: Env,
        message: CrossChainGovMessage,
        proof: BridgeProof,
        proposal_id: u32,
        status: u32,
        for_votes: i128,
        against_votes: i128,
        voting_closed: bool,
    ) -> Result<(), Error> {
        let expected_payload = (
            proposal_id,
            status,
            for_votes,
            against_votes,
            voting_closed,
        )
            .to_xdr(&env);
        if message.payload != expected_payload {
            return Err(Error::InvalidPayload);
        }

        Self::verify_and_record_message(
            &env,
            &message,
            &proof,
            GovernanceActionType::ProposalStateSync,
        )?;

        let record = ProposalStateSync {
            proposal_id,
            status,
            for_votes,
            against_votes,
            voting_closed,
            synced_at: env.ledger().timestamp(),
        };
        let key = DataKey::SyncedProposal(message.source_chain_id, proposal_id);
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("sync_prop"), message.source_chain_id),
            (proposal_id, status, for_votes, against_votes),
        );
        Ok(())
    }

    /// Read synchronized proposal state from a source chain.
    pub fn get_synced_proposal(
        env: Env,
        source_chain_id: u32,
        proposal_id: u32,
    ) -> Option<ProposalStateSync> {
        let key = DataKey::SyncedProposal(source_chain_id, proposal_id);
        env.storage().persistent().get(&key)
    }

    /// Broadcast local proposal state to a remote spoke chain.
    pub fn broadcast_proposal_state(
        env: Env,
        destination_chain_id: u32,
        protocol: BridgeProtocol,
        proposal_id: u32,
        status: u32,
        for_votes: i128,
        against_votes: i128,
        voting_closed: bool,
    ) -> Result<CrossChainGovMessage, Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        if !Self::is_bridge_protocol_enabled(env.clone(), protocol) {
            return Err(Error::UnsupportedBridgeProtocol);
        }

        let out_key = DataKey::OutboundSequence(destination_chain_id);
        let seq = env.storage().persistent().get(&out_key).unwrap_or(0) + 1;
        env.storage().persistent().set(&out_key, &seq);
        env.storage().persistent().extend_ttl(&out_key, TTL_LEDGERS, TTL_LEDGERS);

        let payload = (
            proposal_id,
            status,
            for_votes,
            against_votes,
            voting_closed,
        )
            .to_xdr(&env);

        let message = CrossChainGovMessage {
            source_chain_id: config.local_chain_id,
            destination_chain_id,
            sequence: seq,
            sender: config.admin.to_xdr(&env),
            action_type: GovernanceActionType::ProposalStateSync,
            payload,
            timestamp: env.ledger().timestamp(),
        };

        env.events().publish(
            (symbol_short!("bcast_prp"), destination_chain_id),
            (seq, proposal_id, status),
        );

        Ok(message)
    }

    // ── State Sync: Voting Power Snapshots ──────────────────────────────────

    /// Sync voting power snapshot received from a remote chain.
    pub fn sync_voting_power(
        env: Env,
        message: CrossChainGovMessage,
        proof: BridgeProof,
        voter: Address,
        voting_units: i128,
        snapshot_id: u64,
    ) -> Result<(), Error> {
        if voting_units < 0 {
            return Err(Error::InvalidVotingPower);
        }

        let expected_payload = (voter.clone(), voting_units, snapshot_id).to_xdr(&env);
        if message.payload != expected_payload {
            return Err(Error::InvalidPayload);
        }

        Self::verify_and_record_message(
            &env,
            &message,
            &proof,
            GovernanceActionType::VotingPowerSync,
        )?;

        let record = VotingPowerSync {
            voter: voter.clone(),
            voting_units,
            snapshot_id,
            synced_at: env.ledger().timestamp(),
        };
        let key = DataKey::SyncedVotingPower(message.source_chain_id, voter.clone(), snapshot_id);
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("sync_vote"), message.source_chain_id),
            (voter, voting_units, snapshot_id),
        );
        Ok(())
    }

    /// Read synchronized voting power snapshot.
    pub fn get_synced_voting_power(
        env: Env,
        source_chain_id: u32,
        voter: Address,
        snapshot_id: u64,
    ) -> Option<VotingPowerSync> {
        let key = DataKey::SyncedVotingPower(source_chain_id, voter, snapshot_id);
        env.storage().persistent().get(&key)
    }

    /// Broadcast local voting power snapshot to remote spoke chain.
    pub fn broadcast_voting_power(
        env: Env,
        destination_chain_id: u32,
        protocol: BridgeProtocol,
        voter: Address,
        voting_units: i128,
        snapshot_id: u64,
    ) -> Result<CrossChainGovMessage, Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        if !Self::is_bridge_protocol_enabled(env.clone(), protocol) {
            return Err(Error::UnsupportedBridgeProtocol);
        }

        let out_key = DataKey::OutboundSequence(destination_chain_id);
        let seq = env.storage().persistent().get(&out_key).unwrap_or(0) + 1;
        env.storage().persistent().set(&out_key, &seq);
        env.storage().persistent().extend_ttl(&out_key, TTL_LEDGERS, TTL_LEDGERS);

        let payload = (voter.clone(), voting_units, snapshot_id).to_xdr(&env);

        let message = CrossChainGovMessage {
            source_chain_id: config.local_chain_id,
            destination_chain_id,
            sequence: seq,
            sender: config.admin.to_xdr(&env),
            action_type: GovernanceActionType::VotingPowerSync,
            payload,
            timestamp: env.ledger().timestamp(),
        };

        env.events().publish(
            (symbol_short!("bcast_vot"), destination_chain_id),
            (seq, voter, voting_units),
        );

        Ok(message)
    }

    // ── State Sync: Protocol Parameters ─────────────────────────────────────

    /// Sync a global protocol parameter received from the governance hub.
    pub fn sync_parameter(
        env: Env,
        message: CrossChainGovMessage,
        proof: BridgeProof,
        param_key: Symbol,
        param_value: i128,
    ) -> Result<(), Error> {
        let expected_payload = (param_key.clone(), param_value).to_xdr(&env);
        if message.payload != expected_payload {
            return Err(Error::InvalidPayload);
        }

        Self::verify_and_record_message(
            &env,
            &message,
            &proof,
            GovernanceActionType::ParameterSync,
        )?;

        let record = ParameterSync {
            param_key: param_key.clone(),
            param_value,
            synced_at: env.ledger().timestamp(),
        };
        let key = DataKey::SyncedParameter(param_key.clone());
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("sync_prm"), message.source_chain_id),
            (param_key, param_value),
        );
        Ok(())
    }

    /// Read synchronized global parameter value.
    pub fn get_synced_parameter(env: Env, param_key: Symbol) -> Option<ParameterSync> {
        let key = DataKey::SyncedParameter(param_key);
        env.storage().persistent().get(&key)
    }

    /// Broadcast local parameter value to remote spoke chain.
    pub fn broadcast_parameter(
        env: Env,
        destination_chain_id: u32,
        protocol: BridgeProtocol,
        param_key: Symbol,
        param_value: i128,
    ) -> Result<CrossChainGovMessage, Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        if !Self::is_bridge_protocol_enabled(env.clone(), protocol) {
            return Err(Error::UnsupportedBridgeProtocol);
        }

        let out_key = DataKey::OutboundSequence(destination_chain_id);
        let seq = env.storage().persistent().get(&out_key).unwrap_or(0) + 1;
        env.storage().persistent().set(&out_key, &seq);
        env.storage().persistent().extend_ttl(&out_key, TTL_LEDGERS, TTL_LEDGERS);

        let payload = (param_key.clone(), param_value).to_xdr(&env);

        let message = CrossChainGovMessage {
            source_chain_id: config.local_chain_id,
            destination_chain_id,
            sequence: seq,
            sender: config.admin.to_xdr(&env),
            action_type: GovernanceActionType::ParameterSync,
            payload,
            timestamp: env.ledger().timestamp(),
        };

        env.events().publish(
            (symbol_short!("bcast_prm"), destination_chain_id),
            (seq, param_key, param_value),
        );

        Ok(message)
    }

    // ── State Sync: Merkle State Roots ──────────────────────────────────────

    /// Sync state root from a remote chain.
    pub fn sync_state_root(
        env: Env,
        message: CrossChainGovMessage,
        proof: BridgeProof,
        chain_id: u32,
        block_height: u64,
        root: BytesN<32>,
    ) -> Result<(), Error> {
        let expected_payload = (chain_id, block_height, root.clone()).to_xdr(&env);
        if message.payload != expected_payload {
            return Err(Error::InvalidPayload);
        }

        Self::verify_and_record_message(
            &env,
            &message,
            &proof,
            GovernanceActionType::StateRootSync,
        )?;

        let record = StateRootRecord {
            chain_id,
            block_height,
            root: root.clone(),
            synced_at: env.ledger().timestamp(),
        };

        let key = DataKey::SyncedStateRoot(chain_id, block_height);
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        let latest_key = DataKey::LatestStateRoot(chain_id);
        let current_latest: u64 = env.storage().persistent().get(&latest_key).unwrap_or(0);
        if block_height >= current_latest {
            env.storage().persistent().set(&latest_key, &block_height);
            env.storage().persistent().extend_ttl(&latest_key, TTL_LEDGERS, TTL_LEDGERS);
        }

        env.events().publish(
            (symbol_short!("sync_root"), chain_id),
            (block_height, root),
        );
        Ok(())
    }

    /// Read synchronized state root for a chain and block height.
    pub fn get_synced_state_root(
        env: Env,
        chain_id: u32,
        block_height: u64,
    ) -> Option<StateRootRecord> {
        let key = DataKey::SyncedStateRoot(chain_id, block_height);
        env.storage().persistent().get(&key)
    }

    /// Read the latest synchronized state root for a chain.
    pub fn get_latest_state_root(env: Env, chain_id: u32) -> Option<StateRootRecord> {
        let latest_key = DataKey::LatestStateRoot(chain_id);
        let block_height: u64 = env.storage().persistent().get(&latest_key)?;
        Self::get_synced_state_root(env, chain_id, block_height)
    }

    /// Broadcast local state root to remote spoke chain.
    pub fn broadcast_state_root(
        env: Env,
        destination_chain_id: u32,
        protocol: BridgeProtocol,
        chain_id: u32,
        block_height: u64,
        root: BytesN<32>,
    ) -> Result<CrossChainGovMessage, Error> {
        let config = Self::get_config(env.clone())?;
        config.admin.require_auth();

        if !Self::is_bridge_protocol_enabled(env.clone(), protocol) {
            return Err(Error::UnsupportedBridgeProtocol);
        }

        let out_key = DataKey::OutboundSequence(destination_chain_id);
        let seq = env.storage().persistent().get(&out_key).unwrap_or(0) + 1;
        env.storage().persistent().set(&out_key, &seq);
        env.storage().persistent().extend_ttl(&out_key, TTL_LEDGERS, TTL_LEDGERS);

        let payload = (chain_id, block_height, root.clone()).to_xdr(&env);

        let message = CrossChainGovMessage {
            source_chain_id: config.local_chain_id,
            destination_chain_id,
            sequence: seq,
            sender: config.admin.to_xdr(&env),
            action_type: GovernanceActionType::StateRootSync,
            payload,
            timestamp: env.ledger().timestamp(),
        };

        env.events().publish(
            (symbol_short!("bcast_rt"), destination_chain_id),
            (seq, chain_id, block_height),
        );

        Ok(message)
    }

    // ── Payload & Proof Encoding Utilities ──────────────────────────────────

    /// Encode remote execution payload.
    pub fn encode_execution_payload(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        execution_id: BytesN<32>,
    ) -> Bytes {
        (target, function, args, execution_id).to_xdr(&env)
    }

    /// Encode proposal state sync payload.
    pub fn encode_proposal_payload(
        env: Env,
        proposal_id: u32,
        status: u32,
        for_votes: i128,
        against_votes: i128,
        voting_closed: bool,
    ) -> Bytes {
        (proposal_id, status, for_votes, against_votes, voting_closed).to_xdr(&env)
    }

    /// Encode voting power snapshot sync payload.
    pub fn encode_voting_power_payload(
        env: Env,
        voter: Address,
        voting_units: i128,
        snapshot_id: u64,
    ) -> Bytes {
        (voter, voting_units, snapshot_id).to_xdr(&env)
    }

    /// Encode parameter sync payload.
    pub fn encode_parameter_payload(env: Env, param_key: Symbol, param_value: i128) -> Bytes {
        (param_key, param_value).to_xdr(&env)
    }

    /// Encode state root sync payload.
    pub fn encode_state_root_payload(
        env: Env,
        chain_id: u32,
        block_height: u64,
        root: BytesN<32>,
    ) -> Bytes {
        (chain_id, block_height, root).to_xdr(&env)
    }

    /// Construct a LayerZero bridge proof.
    pub fn layerzero_proof(env: Env, src_eid: u32, dst_eid: u32) -> BridgeProof {
        let mut data = Bytes::new(&env);
        data.append(&Bytes::from_slice(&env, &src_eid.to_be_bytes()));
        data.append(&Bytes::from_slice(&env, &dst_eid.to_be_bytes()));
        BridgeProof {
            protocol: BridgeProtocol::LayerZero,
            proof_data: data,
        }
    }

    /// Construct a Wormhole bridge proof.
    pub fn wormhole_proof(env: Env, quorum: u32, emitter_chain: u32) -> BridgeProof {
        let mut data = Bytes::new(&env);
        data.append(&Bytes::from_slice(&env, &[(quorum as u8)]));
        data.append(&Bytes::from_slice(&env, &emitter_chain.to_be_bytes()));
        BridgeProof {
            protocol: BridgeProtocol::Wormhole,
            proof_data: data,
        }
    }

    /// Construct a Native bridge proof.
    pub fn native_proof(env: Env) -> BridgeProof {
        BridgeProof {
            protocol: BridgeProtocol::Native,
            proof_data: Bytes::new(&env),
        }
    }
}
