use soroban_sdk::{Address, Env, contracttype};
use Perigee_error_codes::ContractError;

/// Snapshot mechanism for staking balances
/// Prevents reward manipulation by recording balances at specific points in time

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StakingSnapshot {
    pub snapshot_id: u64,
    pub ledger_sequence: u32,
    pub staked_amount: i128,
    pub accrued_rewards: i128,
}

#[derive(Clone)]
#[contracttype]
pub enum SnapshotDataKey {
    StakingSnapshot(u64, Address), // snapshot_id, address -> StakingSnapshot
    SnapshotCounter, // counter for snapshot IDs
    EpochSnapshot(u64), // epoch_id -> snapshot_id
}

/// Create a snapshot of staking state for an address
pub fn create_staking_snapshot(
    e: &Env,
    addr: Address,
    snapshot_id: u64,
    staked_amount: i128,
    accrued_rewards: i128,
) -> StakingSnapshot {
    let snapshot = StakingSnapshot {
        snapshot_id,
        ledger_sequence: e.ledger().sequence(),
        staked_amount,
        accrued_rewards,
    };
    
    let key = SnapshotDataKey::StakingSnapshot(snapshot_id, addr);
    e.storage().persistent().set(&key, &snapshot);
    
    snapshot
}

/// Read staking balance from a snapshot
pub fn read_snapshot_staking_balance(
    e: &Env,
    addr: Address,
    snapshot_id: u64,
) -> Option<StakingSnapshot> {
    let key = SnapshotDataKey::StakingSnapshot(snapshot_id, addr);
    e.storage().persistent().get(&key)
}

/// Get the staking balance for reward calculation, using snapshot if specified
pub fn get_staking_balance_for_epoch(
    e: &Env,
    addr: Address,
    epoch_id: Option<u64>,
) -> Result<i128, ContractError> {
    if let Some(epoch) = epoch_id {
        // Check if there's a snapshot for this epoch
        let snapshot_key = SnapshotDataKey::EpochSnapshot(epoch);
        
        if let Some(snapshot_id) = e.storage().persistent().get::<SnapshotDataKey, u64>(&snapshot_key) {
            // Use snapshot staking balance if available
            if let Some(snapshot) = read_snapshot_staking_balance(e, addr.clone(), snapshot_id) {
                return Ok(snapshot.staked_amount);
            }
        }
    }
    
    // Fallback to current staking balance
    Ok(crate::StakingRewards::get_staked_balance(e.clone(), addr))
}

/// Record the snapshot ID for an epoch
pub fn set_epoch_snapshot(e: &Env, epoch_id: u64, snapshot_id: u64) {
    let key = SnapshotDataKey::EpochSnapshot(epoch_id);
    e.storage().persistent().set(&key, &snapshot_id);
}

/// Get the next snapshot ID
pub fn get_next_snapshot_id(e: &Env) -> u64 {
    let key = SnapshotDataKey::SnapshotCounter;
    let current: u64 = e.storage().persistent().get(&key).unwrap_or(0);
    let next = current + 1;
    e.storage().persistent().set(&key, &next);
    next
}

/// Create a snapshot for reward calculation at a specific point in time
pub fn create_reward_epoch_snapshot(
    e: &Env,
    epoch_id: u64,
    addresses: &[Address],
) -> Result<u64, ContractError> {
    let snapshot_id = get_next_snapshot_id(e);
    
    // Create snapshots for all addresses
    for addr in addresses {
        let staked = crate::StakingRewards::get_staked_balance(e.clone(), addr.clone());
        let rewards = crate::StakingRewards::get_accrued_rewards(e.clone(), addr.clone());
        create_staking_snapshot(e, addr.clone(), snapshot_id, staked, rewards);
    }
    
    // Record the snapshot for this epoch
    set_epoch_snapshot(e, epoch_id, snapshot_id);
    
    Ok(snapshot_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_create_and_read_snapshot() {
        let env = Env::default();
        let addr = Address::generate(&env);
        
        // Create snapshot
        let snapshot = create_staking_snapshot(&env, addr.clone(), 1, 100, 10);
        assert_eq!(snapshot.staked_amount, 100);
        assert_eq!(snapshot.accrued_rewards, 10);
        assert_eq!(snapshot.snapshot_id, 1);
        
        // Read snapshot
        let stored = read_snapshot_staking_balance(&env, addr, 1);
        assert!(stored.is_some());
        assert_eq!(stored.unwrap().staked_amount, 100);
    }

    #[test]
    fn test_snapshot_id_increment() {
        let env = Env::default();
        
        let id1 = get_next_snapshot_id(&env);
        let id2 = get_next_snapshot_id(&env);
        let id3 = get_next_snapshot_id(&env);
        
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }
}
