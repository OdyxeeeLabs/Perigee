/// Reentrancy guard for all state-modifying entry points.
///
/// Even though Stellar's execution model is single-threaded and cross-contract
/// calls on Soroban do not have the same classic reentrancy vector as EVM, a
/// defense-in-depth guard is warranted for vault operations:
///
/// - A malicious token contract could, on `transfer`, invoke back into
///   `StakingRewards::withdraw` before the first call records the new balance.
/// - The guard uses a boolean flag stored in instance storage so any re-entry
///   within the same transaction is detected and rejected.
///
/// Usage:
/// ```rust
/// let _guard = ReentrancyGuard::acquire(&e)?;
/// // ... state changes ...
/// // _guard dropped: flag automatically cleared
/// ```
///
/// Closes OdyxeeeLabs/Perigee#506

use soroban_sdk::Env;
pub use Perigee_error_codes::ContractError;

const REENTRANCY_KEY: &str = "reen_lock";

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReentrancyKey {
    Lock,
}

/// Acquire the reentrancy lock. Returns `Err(ContractError::Unauthorized)` if
/// a call is already in progress. Automatically releases the lock when the
/// returned `ReentrancyGuard` is dropped.
pub struct ReentrancyGuard<'a> {
    env: &'a Env,
}

impl<'a> ReentrancyGuard<'a> {
    pub fn acquire(env: &'a Env) -> Result<Self, ContractError> {
        let locked: bool = env
            .storage()
            .instance()
            .get(&REENTRANCY_KEY)
            .unwrap_or(false);
        if locked {
            return Err(ContractError::Unauthorized);
        }
        env.storage().instance().set(&REENTRANCY_KEY, &true);
        Ok(ReentrancyGuard { env })
    }
}

impl<'a> Drop for ReentrancyGuard<'a> {
    fn drop(&mut self) {
        self.env.storage().instance().set(&REENTRANCY_KEY, &false);
    }
}
