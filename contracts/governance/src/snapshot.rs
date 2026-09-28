use crate::storage_types::DataKey;
use soroban_sdk::{Address, Env};

/// Snapshot mechanism for voting power
/// Prevents vote/reward manipulation by recording balances at specific points in time

#[derive(Clone, Debug, Eq, PartialEq)]
#[soroban_sdk::contracttype]
pub struct VotingSnapshot {
    pub snapshot_id: u64,
    pub ledger_sequence: u32,
    pub voting_power: i128,
}

/// Create a snapshot of voting power for an address
pub fn create_voting_snapshot(e: &Env, addr: Address, snapshot_id: u64) -> VotingSnapshot {
    let current_power = crate::voting::read_voting_power(e, addr.clone());
    let snapshot = VotingSnapshot {
        snapshot_id,
        ledger_sequence: e.ledger().sequence(),
        voting_power: current_power,
    };
    
    let key = DataKey::VotingSnapshot(snapshot_id, addr);
    e.storage().persistent().set(&key, &snapshot);
    
    snapshot
}

/// Read voting power from a snapshot
pub fn read_snapshot_voting_power(
    e: &Env,
    addr: Address,
    snapshot_id: u64,
) -> Option<i128> {
    let key = DataKey::VotingSnapshot(snapshot_id, addr);
    e.storage()
        .persistent()
        .get::<DataKey, VotingSnapshot>(&key)
        .map(|snapshot| snapshot.voting_power)
}

/// Get the voting power for a proposal, using snapshot if available
pub fn get_voting_power_for_proposal(
    e: &Env,
    addr: Address,
    proposal_id: u32,
) -> i128 {
    // Check if there's a snapshot for this proposal
    let snapshot_key = DataKey::ProposalSnapshot(proposal_id);
    
    if let Some(snapshot_id) = e.storage().persistent().get::<DataKey, u64>(&snapshot_key) {
        // Use snapshot voting power if available
        if let Some(power) = read_snapshot_voting_power(e, addr, snapshot_id) {
            return power;
        }
    }
    
    // Fallback to current voting power
    crate::voting::read_voting_power(e, addr)
}

/// Record the snapshot ID for a proposal
pub fn set_proposal_snapshot(e: &Env, proposal_id: u32, snapshot_id: u64) {
    let key = DataKey::ProposalSnapshot(proposal_id);
    e.storage().persistent().set(&key, &snapshot_id);
}

/// Get the current snapshot ID counter
pub fn get_next_snapshot_id(e: &Env) -> u64 {
    let key = DataKey::SnapshotCounter;
    let current: u64 = e.storage().persistent().get(&key).unwrap_or(0);
    let next = current + 1;
    e.storage().persistent().set(&key, &next);
    next
}

/// Create snapshots for multiple addresses at once
pub fn create_bulk_snapshots(e: &Env, addresses: &[Address], snapshot_id: u64) {
    for addr in addresses {
        create_voting_snapshot(e, addr.clone(), snapshot_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_create_and_read_snapshot() {
        let env = Env::default();
        let addr = Address::generate(&env);
        
        // Set initial voting power
        crate::voting::write_voting_power(&env, addr.clone(), 100);
        
        // Create snapshot
        let snapshot = create_voting_snapshot(&env, addr.clone(), 1);
        assert_eq!(snapshot.voting_power, 100);
        assert_eq!(snapshot.snapshot_id, 1);
        
        // Read snapshot
        let power = read_snapshot_voting_power(&env, addr, 1);
        assert_eq!(power, Some(100));
    }

    #[test]
    fn test_snapshot_isolation() {
        let env = Env::default();
        let addr = Address::generate(&env);
        
        // Set initial voting power and create snapshot
        crate::voting::write_voting_power(&env, addr.clone(), 100);
        create_voting_snapshot(&env, addr.clone(), 1);
        
        // Change voting power
        crate::voting::write_voting_power(&env, addr.clone(), 200);
        
        // Snapshot should still have old value
        let snapshot_power = read_snapshot_voting_power(&env, addr.clone(), 1);
        assert_eq!(snapshot_power, Some(100));
        
        // Current power should be new value
        let current_power = crate::voting::read_voting_power(&env, addr);
        assert_eq!(current_power, 200);
    }
}
