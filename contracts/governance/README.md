# Governance Contract

A comprehensive DAO governance system for Soroban implementing multi-step proposal workflows with voting, delegation, and timelock execution.

## Overview

This contract provides a full-featured governance system where proposals progress through distinct stages:

- **Draft**: Proposal created but not yet open for voting
- **Voting**: Active voting period with delegation support
- **Queued**: Proposal approved, waiting for timelock delay
- **Executed**: Proposal actions have been executed
- **Cancelled**: Proposal cancelled by admin

## Features

- **Multi-stage Proposals**: Structured proposal lifecycle
- **Voting Delegation**: Users can delegate voting power to others
- **Quorum Requirements**: Minimum participation thresholds
- **Timelock Execution**: Delayed execution for security
- **Admin Controls**: Proposal cancellation and parameter management
- **Threshold Governance**: A configurable signer committee authorizes privileged operations
- **Authorized Modules**: Multisig-approved modules can invoke contract actions

## Multisig Governance

`initialize` remains available for a one-signer deployment. For a threshold
committee, call `initialize_with_signers(admins, signature_threshold,
min_voting_units, identity_required)`. Every signer must authorize
initialization, and the threshold must be between one and the number of unique
signers.

The legacy `register_voter`, `create_proposal`, and `close_proposal` entrypoints
submit only the first signer as an approval, so they remain usable for a
one-signer deployment. For a multisig committee, use the corresponding
`*_with_approvals` entrypoint and supply distinct signer addresses. Signers are
managed with `add_signer`, `remove_signer`, and
`set_signature_threshold`; each change requires the current threshold.

`execute_transaction` executes an arbitrary Soroban contract method only after
the configured threshold has authorized that transaction.

Modules are enabled or disabled with multisig approval. An enabled module must
authorize each `execute_module` call; it can then invoke a Soroban contract
method with Soroban `Val` arguments. This is native Soroban module support and
does not deploy or bridge to an EVM Gnosis Safe.

## Proposal States

```rust
enum ProposalState {
    Draft,      // Created but not voting
    Voting,     // Active voting period
    Queued,     // Approved, waiting for timelock
    Executed,   // Successfully executed
    Cancelled,  // Cancelled by admin
}
```

## Contract Interface

### Initialization
```rust
fn initialize(e: Env, admin: Address, voting_period: u64, timelock_delay: u64, quorum_percentage: u32)
```
Sets up the governance contract with admin and timing parameters.

### Proposal Management
```rust
fn create_proposal(e: Env, title: String, description: String, actions: Vec<ProposalAction>) -> u32
fn start_voting(e: Env, proposal_id: u32)  // Admin only
fn queue_proposal(e: Env, proposal_id: u32)
fn execute_proposal(e: Env, proposal_id: u32)
fn cancel_proposal_admin(e: Env, proposal_id: u32)  // Admin only
```

### Voting
```rust
fn cast_vote(e: Env, proposal_id: u32, support: bool)
fn delegate(e: Env, delegate: Address)
fn set_voting_power(e: Env, user: Address, power: i128)  // Admin only
```

### Queries
```rust
fn get_proposal(e: Env, proposal_id: u32) -> Proposal
fn get_voting_power(e: Env, user: Address) -> i128
fn get_effective_voting_power(e: Env, user: Address) -> i128  // Includes delegated power
fn get_config(e: Env) -> GovernanceConfig
```

## Usage Example

```rust
// Initialize governance
let client = GovernanceClient::new(&env, &contract_id);
client.initialize(&admin, &604800, &172800, &10); // 7d voting, 2d timelock, 10% quorum

// Set up voting power
client.set_voting_power(&user1, &100);

// Create proposal
let actions = Vec::new(&env); // Add contract calls here
let proposal_id = client.create_proposal(
    &"Update Protocol".into(),
    &"Update protocol parameters".into(),
    &actions
);

// Start voting (admin)
client.start_voting(&proposal_id);

// Users vote
client.cast_vote(&proposal_id, &true);

// Or delegate
client.delegate(&trusted_delegate);

// Queue successful proposal
client.queue_proposal(&proposal_id);

// Execute after timelock
client.execute_proposal(&proposal_id);
```

## Security Features

- **Timelock**: Prevents flash loan attacks on governance
- **Quorum Requirements**: Ensures sufficient participation
- **Admin Oversight**: Emergency cancellation capabilities
- **Delegation Safety**: Secure voting power delegation
- **State Validation**: Strict state transitions

## Building

```bash
# Build the contract
cargo build --package soroban-governance-contract

# Build for WASM
cargo build --target wasm32-unknown-unknown --release --package soroban-governance-contract
```

## Testing

```bash
# Run tests
cargo test --package soroban-governance-contract
```

## Parameters

- **voting_period**: Duration of voting in seconds (e.g., 604800 = 7 days)
- **timelock_delay**: Delay before execution in seconds (e.g., 172800 = 2 days)
- **quorum_percentage**: Minimum participation as percentage of total voting power

## Future Enhancements

- Quadratic voting
- Proposal templates
- Multi-signature execution
- Cross-chain governance
- Snapshot voting integration

## License

MIT Stellar Wave