# Snapshot Mechanism for Voting and Staking Rewards

## Overview

This document describes the snapshot mechanism implemented for the Perigee governance and staking rewards contracts. The snapshot mechanism prevents vote and reward manipulation by recording balances at specific points in time.

Closes: #537 [CONTRACT-43]

## Problem Statement

Without snapshots, users could manipulate voting power or staking rewards by:
- **Voting Manipulation**: Transferring tokens or staking power after a proposal is created but before voting ends
- **Reward Manipulation**: Staking large amounts just before reward distribution snapshots and unstaking immediately after
- **Flash Loan Attacks**: Using borrowed funds temporarily to gain voting power or staking rewards

## Solution: Periodic Snapshots

The snapshot mechanism records user balances at specific points in time (snapshots) and uses these historical balances for:
1. **Voting Power**: Determined at proposal creation time
2. **Staking Rewards**: Determined at the start of each reward epoch

### Key Benefits
- **Prevents Manipulation**: Balances are locked in at snapshot time
- **Fair Distribution**: Users cannot game the system by timing their deposits/withdrawals
- **Transparent**: All snapshots are recorded on-chain with timestamps
- **Efficient**: Snapshots are created only when needed

## Architecture

### Governance Contract

#### Data Structures

```rust
pub struct VotingSnapshot {
    pub snapshot_id: u64,
    pub ledger_sequence: u32,
    pub voting_power: i128,
}

pub enum DataKey {
    VotingSnapshot(u64, Address),    // snapshot_id, address -> VotingSnapshot
    ProposalSnapshot(u32),            // proposal_id -> snapshot_id
    SnapshotCounter,                  // Global snapshot ID counter
}
```

#### Key Functions

**`create_voting_snapshot(env, addr, snapshot_id) -> VotingSnapshot`**
- Creates a snapshot of voting power for an address
- Records current ledger sequence and voting power
- Stores snapshot in persistent storage

**`get_voting_power_for_proposal(env, addr, proposal_id) -> i128`**
- Retrieves voting power from proposal's snapshot
- Falls back to current voting power if snapshot doesn't exist

**`set_proposal_snapshot(env, proposal_id, snapshot_id)`**
- Associates a snapshot ID with a proposal
- Called automatically when a proposal is created

### Staking Rewards Contract

#### Data Structures

```rust
pub struct StakingSnapshot {
    pub snapshot_id: u64,
    pub ledger_sequence: u32,
    pub staked_amount: i128,
    pub accrued_rewards: i128,
}

pub enum SnapshotDataKey {
    StakingSnapshot(u64, Address),  // snapshot_id, address -> StakingSnapshot
    EpochSnapshot(u64),              // epoch_id -> snapshot_id
    SnapshotCounter,                 // Global snapshot ID counter
}
```

#### Key Functions

**`create_staking_snapshot(env, addr, snapshot_id, staked, rewards) -> StakingSnapshot`**
- Creates a snapshot of staking state for an address
- Records staked amount and accrued rewards
- Stores snapshot in persistent storage

**`create_reward_epoch_snapshot(env, epoch_id, addresses) -> Result<u64>`**
- Creates snapshots for all stakers at the start of a reward epoch
- Prevents reward manipulation
- Returns the snapshot ID

**`get_staking_balance_for_epoch(env, addr, epoch_id) -> Result<i128>`**
- Retrieves staking balance from epoch snapshot
- Falls back to current balance if snapshot doesn't exist

## Usage Examples

### Governance: Creating a Proposal with Snapshot

```rust
// When creating a proposal, a snapshot is automatically created
let proposal = GovernanceContract::create_proposal(
    env,
    String::from_str(&env, "Proposal Title"),
    String::from_str(&env, "Proposal Description"),
    voting_ends_at,
)?;

// Voting power for all voters is now locked at proposal creation time
```

### Governance: Voting with Snapshot Power

```rust
// When casting a vote, the snapshot voting power is used
let receipt = GovernanceContract::cast_vote(
    env,
    proposal_id,
    voter_address,
    true,  // support
    100,   // credits_to_spend
)?;

// Even if the voter's actual voting power changes, the snapshot value is used
```

### Staking: Creating an Epoch Snapshot

```rust
// At the start of each reward period, create snapshots for all stakers
let staker_addresses = vec![&env, addr1, addr2, addr3];
let snapshot_id = StakingRewards::create_epoch_snapshot(
    env,
    epoch_id,
    staker_addresses,
)?;

// Rewards will be calculated based on these snapshot balances
```

### Staking: Querying Snapshot Balance

```rust
// Retrieve staking balance from a specific snapshot
let snapshot = StakingRewards::get_snapshot_staking_balance(
    env,
    user_address,
    snapshot_id,
);

if let Some(snap) = snapshot {
    // Use snap.staked_amount for reward calculation
    let rewards = calculate_rewards(snap.staked_amount);
}
```

## Implementation Details

### Snapshot ID Management

- Each snapshot is assigned a unique, monotonically increasing ID
- The global counter is stored in persistent storage
- Snapshot IDs are shared across all users to maintain temporal ordering

### Storage Considerations

- Snapshots are stored in **persistent storage** for long-term availability
- Each snapshot includes the ledger sequence for timestamp reference
- Storage keys use composite types for efficient lookup

### Timing

**Governance Snapshots:**
- Created: At proposal creation time
- Used: Throughout the voting period
- Prevents: Last-minute voting power manipulation

**Staking Snapshots:**
- Created: At the start of each reward epoch
- Used: For reward calculations during that epoch
- Prevents: Strategic staking/unstaking around distribution times

## Security Considerations

### Snapshot Immutability
- Once created, snapshots cannot be modified
- Historical data is preserved for audit purposes

### Access Control
- Only authorized addresses can create epoch snapshots (owner only)
- All users can query their own snapshot data
- Proposal creation automatically triggers snapshot creation

### Edge Cases Handled
1. **Missing Snapshots**: Falls back to current balance
2. **Zero Balances**: Correctly handles users with no voting power/stake
3. **Multiple Snapshots**: Each proposal/epoch has its own snapshot
4. **Timestamp Recording**: Ledger sequence is recorded for each snapshot

## Testing

### Unit Tests

Both contracts include comprehensive unit tests:

```rust
// Governance contract
#[test]
fn test_create_and_read_snapshot()
#[test]
fn test_snapshot_isolation()

// Staking rewards contract
#[test]
fn test_create_and_read_snapshot()
#[test]
fn test_snapshot_id_increment()
```

### Integration Testing Recommendations

1. **Manipulation Prevention Tests**
   - Create proposal, transfer voting power, verify old power is used
   - Stake, create snapshot, stake more, verify snapshot uses old amount

2. **Multi-User Scenarios**
   - Multiple users voting with different snapshot times
   - Batch snapshot creation for large staker sets

3. **Edge Case Testing**
   - Empty snapshots
   - Maximum voting power scenarios
   - Snapshot ID overflow (unlikely but should be considered)

## Migration Guide

### For Existing Deployments

1. **Deploy New Contract Versions**
   - Deploy updated governance contract with snapshot support
   - Deploy updated staking_rewards contract with snapshot support

2. **Initialize Snapshot System**
   - Snapshot counter starts at 0
   - First snapshots will be created on next proposal/epoch

3. **Backward Compatibility**
   - Falls back to current balance if snapshot doesn't exist
   - Existing proposals continue to work as before

### For New Deployments

- Snapshot mechanism is active from initialization
- No special configuration required
- Automatically enabled for all proposals and reward epochs

## Future Enhancements

### Potential Improvements
1. **Snapshot Pruning**: Remove old snapshots to save storage costs
2. **Batch Snapshot Creation**: Optimize gas costs for large user sets
3. **Snapshot History**: Query historical snapshots for analytics
4. **Configurable Snapshot Timing**: Allow custom snapshot schedules

### Performance Optimizations
- Implement lazy snapshot creation (on-demand)
- Use Merkle trees for efficient multi-user snapshots
- Add snapshot caching for frequently accessed data

## References

- Issue: #537 [CONTRACT-43] Add snapshot mechanism for voting and staking rewards
- Related Contracts:
  - `contracts/governance/`
  - `contracts/staking_rewards/`
- Soroban Documentation: https://soroban.stellar.org/docs

## Conclusion

The snapshot mechanism provides a robust solution to prevent manipulation of voting power and staking rewards. By recording balances at specific points in time, the system ensures fair and transparent governance and reward distribution.

For questions or contributions, please refer to the main project repository.
