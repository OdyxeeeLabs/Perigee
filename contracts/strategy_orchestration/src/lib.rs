#![no_std]

//! # Perigee Strategy Orchestration — rotation and cross-chain yield
//!
//! Closes OdyxeeeLabs/Perigee#659
//! Closes OdyxeeeLabs/Perigee#656
//!
//! Rotation and cross-chain deployment are the two ways a vault moves capital
//! into a strategy, and both fail in the same quiet way: the decision is taken
//! off-chain and the result is written down afterwards, so by the time anything
//! disagrees the capital is already committed.
//!
//! This crate holds two contracts that pull the *decision* on-chain.
//!
//! ## RotationEngine ([#659])
//!
//! `set_target_weights` stores the intended allocation for a vault;
//! [`RotationEngine::plan_rotation`] compares it with the observed allocation and
//! returns a `RotationPlan` — the legs that are worth moving, prioritised by
//! drift, capped by `max_legs`. The gas-saving is the cap: a rotation that moves
//! every strategy every cycle pays for legs that do not matter, so only the
//! largest drifts are rotated and the rest wait for the next cycle.
//!
//! `drift_threshold_bps` means small deviations do not move capital at all. A
//! vault that rebalances to the exact basis point pays its entire return in gas;
//! one that lets positions wander within a threshold pays nothing until the
//! drift actually justifies a transaction.
//!
//! `record_rotation` is the slippage gate. It is called by the vault with the
//! amount it intended to move and the amount it actually received, and reverts if
//! the difference exceeds the policy's `slippage_bps`.
//!
//! ## CrossChainYield ([#656])
//!
//! A strategy on another chain is not a strategy this chain can call directly.
//! The vault holds the deposit token here, and deploying it remotely means
//! locking it into a bridge and having it re-minted on the other side. That
//! `send`/`recall` round trip is exactly the interface this contract drives, so
//! a strategy is expressible as "a token, an amount, a remote vault, a chain".
//!
//! `add_chain` registers a chain with its bridge and remote vault.
//! [`CrossChainYield::deploy`] sends funds through the bridge, [`confirm`]
//! settles the delivery with the relayer, and [`CrossChainYield::withdraw`]
//! recalls funds home. Each chain's exposure is capped at
//! `max_allocation_bps` of the total book the admin reports, so a bridge failure
//! cannot strand more than the cap allows.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Maximum strategies or targets a single vault may track.
pub const MAX_STRATEGIES: u32 = 32;

/// Maximum remote chains that can be registered.
pub const MAX_CHAINS: u32 = 16;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the orchestration contracts.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The contract has not been initialized yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not authorised for this entry point.
    Unauthorized = 3,
    /// The vault has no rotation policy configured.
    PolicyNotSet = 4,
    /// The vault has no target weights configured.
    TargetsNotSet = 5,
    /// A rotation policy field was out of range.
    InvalidPolicy = 6,
    /// Target weights are empty or do not sum to 100%.
    InvalidTargets = 7,
    /// A target weight repeats a strategy.
    DuplicateStrategy = 8,
    /// More targets or chains than the contract can track.
    TooManyStrategies = 9,
    /// A rotation moved less than the slippage limit allows.
    SlippageExceeded = 10,
    /// A non-positive amount was supplied.
    InvalidAmount = 11,
    /// The chain is not registered.
    ChainNotRegistered = 12,
    /// The chain is already registered.
    ChainAlreadyRegistered = 13,
    /// The chain has been deactivated.
    ChainInactive = 14,
    /// The deployment would exceed the chain's allocation cap.
    CapExceeded = 15,
    /// Nothing is pending on that chain to confirm.
    NothingPending = 16,
    /// More chains than the contract can track.
    TooManyChains = 17,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 18,
}

// ── Bridge interface ─────────────────────────────────────────────────────────

/// The bridge this contract drives to reach a remote chain.
///
/// `send` locks the token on this chain and instructs it to be minted on the
/// destination; `recall` burns the remote tokens and unlocks them here. Both
/// are the whole of the bridge surface the vault needs.
#[soroban_sdk::contractclient(name = "BridgeClient")]
pub trait Bridge {
    /// Lock `amount` of `token` for delivery to `recipient` on `chain_id`.
    fn send(env: Env, token: Address, amount: i128, chain_id: u32, recipient: Address) -> u64;
    /// Recall `amount` of a token previously delivered to `chain_id` back home.
    fn recall(env: Env, chain_id: u32, amount: i128) -> u64;
}

// ── RotationEngine types ─────────────────────────────────────────────────────

/// How a vault is allowed to rotate.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationPolicy {
    /// Minimum drift that justifies a rotation, in bps.
    pub drift_threshold_bps: u32,
    /// Maximum slippage tolerated on the received amount, in bps.
    pub slippage_bps: u32,
    /// Maximum legs rotated in one cycle, bounding the gas a rotation costs.
    pub max_legs: u32,
    /// Legs smaller than this are dust and are never rotated.
    pub min_leg_bps: u32,
}

/// One target weight in a vault's intended allocation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetWeight {
    /// Strategy the weight applies to.
    pub strategy: Address,
    /// Intended weight in basis points.
    pub target_bps: u32,
}

/// A single leg of a planned rotation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationLeg {
    /// Strategy the leg concerns.
    pub strategy: Address,
    /// Observed weight.
    pub current_bps: u32,
    /// Intended weight.
    pub target_bps: u32,
    /// `|target - current|`, in bps.
    pub drift_bps: u32,
}

/// The set of legs worth moving, and what was left alone.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationPlan {
    /// Legs worth rotating, largest drift first.
    pub legs: Vec<RotationLeg>,
    /// Total drift across every strategy, not just the selected legs.
    pub total_drift_bps: u32,
    /// Candidates that were not rotated because `max_legs` was reached.
    pub legs_skipped: u32,
    /// Whether any leg is worth moving.
    pub rotation_needed: bool,
}

// ── CrossChainYield types ────────────────────────────────────────────────────

/// A registered remote chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainConfig {
    /// Chain identifier, as the bridge understands it.
    pub chain_id: u32,
    /// Bridge contract that moves value to and from this chain.
    pub bridge: Address,
    /// Vault address on the remote chain that receives the deployment.
    pub remote_vault: Address,
    /// This chain's cap on the book, in bps.
    pub max_allocation_bps: u32,
    /// Whether new deployments are accepted.
    pub active: bool,
}

/// What has been sent to a remote chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemotePosition {
    /// Chain this position belongs to.
    pub chain_id: u32,
    /// Sent and not yet confirmed delivered.
    pub pending: i128,
    /// Confirmed delivered.
    pub confirmed: i128,
}

// ── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to configure rotation and chains.
    Admin,
    /// [`RotationPolicy`] for a vault.
    Policy(Address),
    /// [`TargetWeight`] list for a vault.
    Targets(Address),
    /// Rotations performed for a vault.
    RotationCount(Address),
    /// Total value moved for a vault.
    Volume(Address),
    /// Token the bridge moves.
    Token,
    /// Relayer allowed to confirm remote deliveries.
    Relayer,
    /// Total book value the per-chain caps are a fraction of.
    TotalManaged,
    /// Number of registered chains.
    ChainCount,
    /// Registered chain id at an index.
    ChainAt(u32),
    /// [`ChainConfig`] for a chain.
    Chain(u32),
    /// [`RemotePosition`] for a chain.
    Position(u32),
}

// ── Shared helpers ───────────────────────────────────────────────────────────

fn load_instance<T>(env: &Env, key: &DataKey) -> Result<T, Error>
where
    T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + Clone,
{
    env.storage().instance().get(key).ok_or(Error::NotInitialized)
}

fn load_persistent<T>(env: &Env, key: &DataKey) -> Result<T, Error>
where
    T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + Clone,
{
    env.storage().persistent().get(key).ok_or(Error::NotInitialized)
}

fn save_persistent<T>(env: &Env, key: &DataKey, value: &T) -> Result<(), Error>
where
    T: soroban_sdk::IntoVal<Env, soroban_sdk::Val> + Clone,
{
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_LEDGERS, TTL_LEDGERS);
    Ok(())
}

fn checked_add(a: i128, b: i128) -> Result<i128, Error> {
    a.checked_add(b).ok_or(Error::MathOverflow)
}

// ── RotationEngine ───────────────────────────────────────────────────────────

fn validate_policy(policy: &RotationPolicy) -> Result<(), Error> {
    if policy.drift_threshold_bps > BPS_DENOMINATOR as u32
        || policy.slippage_bps > BPS_DENOMINATOR as u32
        || policy.min_leg_bps > BPS_DENOMINATOR as u32
    {
        return Err(Error::InvalidPolicy);
    }
    if policy.max_legs == 0 || policy.max_legs > MAX_STRATEGIES {
        return Err(Error::InvalidPolicy);
    }
    Ok(())
}

fn validate_targets(targets: &Vec<TargetWeight>) -> Result<(), Error> {
    let count = targets.len();
    if count == 0 || count > MAX_STRATEGIES {
        return Err(Error::InvalidTargets);
    }

    let mut total: i128 = 0;
    for i in 0..count {
        let weight = targets.get(i).unwrap();
        total = checked_add(total, weight.target_bps as i128)?;

        // O(n²) duplicate check — bounded by MAX_STRATEGIES.
        for j in 0..i {
            if targets.get(j).unwrap().strategy == weight.strategy {
                return Err(Error::DuplicateStrategy);
            }
        }
    }
    if total != BPS_DENOMINATOR {
        return Err(Error::InvalidTargets);
    }
    Ok(())
}

fn load_policy(env: &Env, vault: &Address) -> Result<RotationPolicy, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Policy(vault.clone()))
        .ok_or(Error::PolicyNotSet)
}

fn load_targets(env: &Env, vault: &Address) -> Result<Vec<TargetWeight>, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Targets(vault.clone()))
        .ok_or(Error::TargetsNotSet)
}

fn observed_weight(current: &Vec<TargetWeight>, strategy: &Address) -> u32 {
    for i in 0..current.len() {
        let entry = current.get(i).unwrap();
        if entry.strategy == *strategy {
            return entry.target_bps;
        }
    }
    0
}

#[contract]
pub struct RotationEngine;

#[contractimpl]
impl RotationEngine {
    /// Initialize the rotation engine.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.events().publish((symbol_short!("init"),), admin);
        Ok(())
    }

    /// Configure the rotation policy for a vault.
    pub fn set_policy(
        env: Env,
        admin: Address,
        vault: Address,
        policy: RotationPolicy,
    ) -> Result<(), Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        validate_policy(&policy)?;

        let key = DataKey::Policy(vault.clone());
        save_persistent(&env, &key, &policy)?;

        env.events().publish(
            (symbol_short!("policy"), vault),
            (policy.drift_threshold_bps, policy.max_legs),
        );
        Ok(())
    }

    /// The policy configured for a vault.
    pub fn get_policy(env: Env, vault: Address) -> Result<RotationPolicy, Error> {
        load_policy(&env, &vault)
    }

    /// Set the intended allocation for a vault.
    pub fn set_targets(
        env: Env,
        admin: Address,
        vault: Address,
        targets: Vec<TargetWeight>,
    ) -> Result<(), Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        validate_targets(&targets)?;

        let key = DataKey::Targets(vault.clone());
        save_persistent(&env, &key, &targets)?;

        env.events()
            .publish((symbol_short!("targets"), vault), targets.len());
        Ok(())
    }

    /// The target weights for a vault.
    pub fn get_targets(env: Env, vault: Address) -> Result<Vec<TargetWeight>, Error> {
        load_targets(&env, &vault)
    }

    /// # Plan a rotation
    ///
    /// Compare `current` with the configured targets and return the legs worth
    /// moving. A strategy becomes a candidate when its drift is at least
    /// `drift_threshold_bps` **and** one side of the leg is at least
    /// `min_leg_bps`, so dust below the threshold and dust below the leg floor
    /// both stay where they are. Candidates are then capped at `max_legs`,
    /// largest drift first.
    pub fn plan_rotation(
        env: Env,
        vault: Address,
        current: Vec<TargetWeight>,
    ) -> Result<RotationPlan, Error> {
        let targets = load_targets(&env, &vault)?;
        let policy = load_policy(&env, &vault)?;

        let mut candidates: Vec<RotationLeg> = Vec::new(&env);
        let mut total_drift: u32 = 0;

        for i in 0..targets.len() {
            let target = targets.get(i).unwrap();
            let current_bps = observed_weight(&current, &target.strategy);
            let drift = if target.target_bps > current_bps {
                target.target_bps - current_bps
            } else {
                current_bps - target.target_bps
            };
            total_drift += drift;

            let above_threshold = drift >= policy.drift_threshold_bps;
            let above_floor =
                target.target_bps >= policy.min_leg_bps || current_bps >= policy.min_leg_bps;
            if above_threshold && above_floor {
                candidates.push_back(RotationLeg {
                    strategy: target.strategy.clone(),
                    current_bps,
                    target_bps: target.target_bps,
                    drift_bps: drift,
                });
            }
        }

        // Select the highest-drift candidates, capped at max_legs.
        let candidate_count = candidates.len();
        let limit = if policy.max_legs >= candidate_count {
            candidate_count
        } else {
            policy.max_legs
        };

        let mut used: Vec<u32> = Vec::new(&env);
        for _ in 0..candidate_count {
            used.push_back(0);
        }

        let mut legs: Vec<RotationLeg> = Vec::new(&env);
        for _ in 0..limit {
            let mut best_drift: i128 = -1;
            let mut best_idx: u32 = 0;
            for j in 0..candidate_count {
                if used.get(j).unwrap() == 1 {
                    continue;
                }
                let drift = candidates.get(j).unwrap().drift_bps as i128;
                if drift > best_drift {
                    best_drift = drift;
                    best_idx = j;
                }
            }
            if best_drift < 0 {
                break;
            }
            used.set(best_idx, 1);
            legs.push_back(candidates.get(best_idx).unwrap());
        }

        let legs_skipped = candidate_count - legs.len();
        let rotation_needed = legs.len() > 0;

        Ok(RotationPlan {
            legs,
            total_drift_bps: total_drift,
            legs_skipped,
            rotation_needed,
        })
    }

    /// # Record a rotation and enforce the slippage limit
    ///
    /// `received` must be at least `amount × (1 − slippage_bps/10_000)`.
    pub fn record_rotation(
        env: Env,
        admin: Address,
        vault: Address,
        from: Address,
        to: Address,
        amount: i128,
        received: i128,
    ) -> Result<(), Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        if amount <= 0 || received < 0 {
            return Err(Error::InvalidAmount);
        }
        let policy = load_policy(&env, &vault)?;

        let min_received = amount
            .checked_mul(BPS_DENOMINATOR - policy.slippage_bps as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR;
        if received < min_received {
            return Err(Error::SlippageExceeded);
        }

        let count_key = DataKey::RotationCount(vault.clone());
        let volume_key = DataKey::Volume(vault.clone());
        let mut count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let mut volume: i128 = env.storage().persistent().get(&volume_key).unwrap_or(0);

        count = count.checked_add(1).ok_or(Error::MathOverflow)?;
        volume = checked_add(volume, amount)?;

        save_persistent(&env, &count_key, &count)?;
        save_persistent(&env, &volume_key, &volume)?;

        env.events().publish(
            (symbol_short!("rotated"), vault),
            (from, to, amount, received),
        );
        Ok(())
    }

    /// How many rotations a vault has performed.
    pub fn rotation_count(env: Env, vault: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::RotationCount(vault))
            .unwrap_or(0)
    }

    /// Total value a vault has rotated.
    pub fn volume_moved(env: Env, vault: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Volume(vault))
            .unwrap_or(0)
    }

    /// The admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        load_instance(&env, &DataKey::Admin)
    }
}

// ── CrossChainYield ──────────────────────────────────────────────────────────

fn load_chain(env: &Env, chain_id: u32) -> Result<ChainConfig, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Chain(chain_id))
        .ok_or(Error::ChainNotRegistered)
}

fn load_position(env: &Env, chain_id: u32) -> Result<RemotePosition, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Position(chain_id))
        .ok_or(Error::ChainNotRegistered)
}

fn position_value(env: &Env, chain_id: u32) -> RemotePosition {
    env.storage()
        .persistent()
        .get(&DataKey::Position(chain_id))
        .unwrap_or(RemotePosition {
            chain_id,
            pending: 0,
            confirmed: 0,
        })
}

fn load_total_managed(env: &Env) -> Result<i128, Error> {
    env.storage()
        .instance()
        .get(&DataKey::TotalManaged)
        .ok_or(Error::NotInitialized)
}

fn load_token(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Token)
        .ok_or(Error::NotInitialized)
}

/// The cap on one chain's exposure, in absolute terms.
fn chain_cap(total_managed: i128, max_allocation_bps: u32) -> Result<i128, Error> {
    total_managed
        .checked_mul(max_allocation_bps as i128)
        .ok_or(Error::MathOverflow)
        .map(|product| product / BPS_DENOMINATOR)
}

#[contract]
pub struct CrossChainYield;

#[contractimpl]
impl CrossChainYield {
    /// Initialize with the token the bridge moves and the relayer.
    pub fn initialize(env: Env, admin: Address, token: Address, relayer: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        relayer.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::Relayer, &relayer);
        env.storage()
            .instance()
            .set(&DataKey::TotalManaged, &0i128);
        env.storage().instance().set(&DataKey::ChainCount, &0u32);

        env.events().publish((symbol_short!("init"),), (token, relayer));
        Ok(())
    }

    /// Set the total book value the per-chain caps are a fraction of.
    pub fn set_total_managed(env: Env, admin: Address, value: i128) -> Result<(), Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        if value <= 0 {
            return Err(Error::InvalidAmount);
        }
        env.storage().instance().set(&DataKey::TotalManaged, &value);
        env.events().publish((symbol_short!("managed"),), value);
        Ok(())
    }

    /// The total book value.
    pub fn total_managed(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalManaged)
            .unwrap_or(0)
    }

    /// Register a remote chain with its bridge and remote vault.
    pub fn add_chain(
        env: Env,
        admin: Address,
        chain_id: u32,
        bridge: Address,
        remote_vault: Address,
        max_allocation_bps: u32,
    ) -> Result<(), Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        if max_allocation_bps > BPS_DENOMINATOR as u32 {
            return Err(Error::InvalidAmount);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::Chain(chain_id))
        {
            return Err(Error::ChainAlreadyRegistered);
        }

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ChainCount)
            .unwrap_or(0);
        if count >= MAX_CHAINS {
            return Err(Error::TooManyChains);
        }

        let config = ChainConfig {
            chain_id,
            bridge: bridge.clone(),
            remote_vault,
            max_allocation_bps,
            active: true,
        };
        let chain_key = DataKey::Chain(chain_id);
        save_persistent(&env, &chain_key, &config)?;

        env.storage()
            .instance()
            .set(&DataKey::ChainAt(count), &chain_id);
        env.storage()
            .instance()
            .set(&DataKey::ChainCount, &(count + 1));

        env.events().publish(
            (symbol_short!("chain_add"), chain_id),
            (bridge, max_allocation_bps),
        );
        Ok(())
    }

    /// Enable or disable a chain.
    pub fn set_chain_active(
        env: Env,
        admin: Address,
        chain_id: u32,
        active: bool,
    ) -> Result<(), Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();

        let mut config = load_chain(&env, chain_id)?;
        config.active = active;
        save_persistent(&env, &DataKey::Chain(chain_id), &config)?;

        env.events().publish((symbol_short!("chain_set"), chain_id), active);
        Ok(())
    }

    /// The configuration for a chain.
    pub fn get_chain(env: Env, chain_id: u32) -> Result<ChainConfig, Error> {
        load_chain(&env, chain_id)
    }

    /// How many chains are registered.
    pub fn chain_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ChainCount)
            .unwrap_or(0)
    }

    /// The position on a chain.
    pub fn get_position(env: Env, chain_id: u32) -> RemotePosition {
        position_value(&env, chain_id)
    }

    /// # Deploy funds to a remote vault through the bridge
    ///
    /// The chain must be active and the resulting exposure — `confirmed +
    /// pending + amount` — must stay within `max_allocation_bps` of the total
    /// book.
    pub fn deploy(env: Env, admin: Address, chain_id: u32, amount: i128) -> Result<u64, Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let config = load_chain(&env, chain_id)?;
        if !config.active {
            return Err(Error::ChainInactive);
        }

        let total_managed = load_total_managed(&env)?;
        let cap = chain_cap(total_managed, config.max_allocation_bps)?;
        let position = position_value(&env, chain_id);
        let deployed = checked_add(position.confirmed, position.pending)?;
        let projected = checked_add(deployed, amount)?;
        if projected > cap {
            return Err(Error::CapExceeded);
        }

        let token = load_token(&env)?;
        let message_id =
            BridgeClient::new(&env, &config.bridge).send(&token, &amount, &chain_id, &config.remote_vault);

        let mut updated = position;
        updated.pending = checked_add(updated.pending, amount)?;
        save_persistent(&env, &DataKey::Position(chain_id), &updated)?;

        env.events().publish((symbol_short!("deploy"), chain_id), (amount, message_id));
        Ok(message_id)
    }

    /// # Confirm a delivery, releasing it from pending
    ///
    /// Only the relayer may confirm, so the state changes only when the bridge's
    /// message has actually been processed on the remote chain.
    pub fn confirm(env: Env, chain_id: u32, amount: i128) -> Result<(), Error> {
        let relayer: Address = load_instance(&env, &DataKey::Relayer)?;
        relayer.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let mut position = position_value(&env, chain_id);
        if amount > position.pending {
            return Err(Error::NothingPending);
        }
        position.pending -= amount;
        position.confirmed = checked_add(position.confirmed, amount)?;
        save_persistent(&env, &DataKey::Position(chain_id), &position)?;

        env.events().publish((symbol_short!("confirm"), chain_id), amount);
        Ok(())
    }

    /// # Recall funds from a remote vault
    ///
    /// Burns the remote tokens and unlocks them here, reducing the confirmed
    /// balance by exactly the amount recalled.
    pub fn withdraw(env: Env, admin: Address, chain_id: u32, amount: i128) -> Result<u64, Error> {
        let stored_admin: Address = load_instance(&env, &DataKey::Admin)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let config = load_chain(&env, chain_id)?;
        if !config.active {
            return Err(Error::ChainInactive);
        }

        let mut position = position_value(&env, chain_id);
        if amount > position.confirmed {
            return Err(Error::NothingPending);
        }
        position.confirmed -= amount;

        let token = load_token(&env)?;
        let message_id =
            BridgeClient::new(&env, &config.bridge).recall(&chain_id, &amount);

        save_persistent(&env, &DataKey::Position(chain_id), &position)?;

        env.events().publish((symbol_short!("withdraw"), chain_id), (amount, message_id));
        Ok(message_id)
    }

    /// The admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        load_instance(&env, &DataKey::Admin)
    }
}
