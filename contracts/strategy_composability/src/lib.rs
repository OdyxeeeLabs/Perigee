#![no_std]

//! # Perigee Strategy Composability — strategy-of-strategies and fee layering
//!
//! Closes OdyxeeeLabs/Perigee#660
//!
//! A strategy does not have to be a single deployment. A vault can allocate to
//! a *composite* — a strategy whose children are themselves strategies, several
//! of which may themselves be composites. This contract is the registry that
//! describes those trees, resolves them into a flat allocation over leaf
//! strategies, and computes the fee that actually applies when more than one
//! layer charges.
//!
//! ## The tree
//!
//! A [`CompositeStrategy`] holds an ordered list of [`ChildWeight`] children and
//! its own `fee_bps`. Children are addresses; a child that is itself a
//! registered composite is expanded, and one that is not is a leaf. Nothing
//! about a leaf is stored here — the leaf is whatever contract implements the
//! strategy interface — so the tree is a pure allocation description.
//!
//! ## Depth is bounded, not trusted
//!
//! [`StrategyComposability::resolve_allocation`] expands a composite into a
//! flat list of [`AllocationLeaf`]. Recursion is bounded by [`MAX_DEPTH`] and by
//! a visited set that catches cycles, so a malformed tree — one that references
//! itself directly or through a loop — is rejected rather than expanded forever.
//! A vault that could be tricked into unbounded recursion would be a vault that
//! could be tricked into a failed transaction.
//!
//! ## Fee layering
//!
//! When a composite charges `fee_bps` and its children charge their own fees,
//! the fee that applies to capital flowing through the tree is not the sum.
//! [`StrategyComposability::effective_fee_bps`] composes them:
//!
//! ```text
//! effective(node) = fee(node) + Σ weight(child) × effective(child) / 10_000
//! ```
//!
//! Each layer takes its cut from what actually reaches it, so the effective fee
//! is the layered result, not a naive addition that would over-charge. The
//! result is capped at [`MAX_FEE_BPS`].

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Maximum recursion depth when expanding a composite.
pub const MAX_DEPTH: u32 = 4;

/// Maximum children a single composite may hold.
pub const MAX_CHILDREN: u32 = 16;

/// Ceiling on the effective fee of a composite, in basis points.
pub const MAX_FEE_BPS: u32 = 2_000;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the composability registry.
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
    /// The strategy is not registered.
    StrategyNotFound = 4,
    /// The strategy is already registered.
    StrategyAlreadyRegistered = 5,
    /// The children list is empty or its weights do not sum to 100%.
    InvalidChildren = 6,
    /// A child weight is above 100%.
    InvalidWeight = 7,
    /// Expanding the tree would exceed `MAX_DEPTH`.
    MaxDepthExceeded = 8,
    /// A composite lists itself as a child.
    SelfReference = 9,
    /// More children than `MAX_CHILDREN`.
    TooManyChildren = 10,
    /// The tree contains a cycle.
    CyclicReference = 11,
    /// A negative amount was supplied.
    InvalidAmount = 12,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 13,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// One child of a composite and its weight.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildWeight {
    /// Child strategy address.
    pub child: Address,
    /// Weight in basis points.
    pub weight_bps: u32,
}

/// A strategy whose children are themselves strategies.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositeStrategy {
    /// This composite's address.
    pub address: Address,
    /// Ordered children and their weights.
    pub children: Vec<ChildWeight>,
    /// Fee this layer charges on capital passing through, in bps.
    pub fee_bps: u32,
    /// Depth of this node: 1 for a registered composite.
    pub depth: u32,
}

/// A leaf in a resolved allocation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationLeaf {
    /// Leaf strategy address.
    pub strategy: Address,
    /// Amount allocated to this leaf.
    pub amount: i128,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to register composites.
    Admin,
    /// Number of registered composites.
    CompositeCount,
    /// Composite address at an index.
    CompositeAt(u32),
    /// [`CompositeStrategy`] for an address.
    Composite(Address),
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    let stored: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if *admin != stored {
        return Err(Error::Unauthorized);
    }
    admin.require_auth();
    Ok(())
}

fn validate_children(env: &Env, parent: &Address, children: &Vec<ChildWeight>) -> Result<(), Error> {
    let count = children.len();
    if count == 0 || count > MAX_CHILDREN {
        return Err(Error::InvalidChildren);
    }

    let mut total: i128 = 0;
    for i in 0..count {
        let child = children.get(i).unwrap();
        if child.weight_bps == 0 || child.weight_bps > BPS_DENOMINATOR as u32 {
            return Err(Error::InvalidWeight);
        }
        if child.child == *parent {
            return Err(Error::SelfReference);
        }
        total += child.weight_bps as i128;

        // No duplicate children.
        for j in 0..i {
            if children.get(j).unwrap().child == child.child {
                return Err(Error::InvalidChildren);
            }
        }
    }
    if total != BPS_DENOMINATOR {
        return Err(Error::InvalidChildren);
    }
    Ok(())
}

fn load_composite(env: &Env, address: &Address) -> Result<CompositeStrategy, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Composite(address.clone()))
        .ok_or(Error::StrategyNotFound)
}

/// Expand a node into its leaf allocations.
///
/// `visited` holds the addresses already on the current path, so a cycle is
/// detected before recursion continues.
fn expand(
    env: &Env,
    address: &Address,
    total: i128,
    depth: u32,
    visited: &Vec<Address>,
) -> Result<Vec<AllocationLeaf>, Error> {
    if depth > MAX_DEPTH {
        return Err(Error::MaxDepthExceeded);
    }
    for i in 0..visited.len() {
        if visited.get(i).unwrap() == *address {
            return Err(Error::CyclicReference);
        }
    }

    let mut path = visited.clone();
    path.push_back(address.clone());

    let composite: Option<CompositeStrategy> = env
        .storage()
        .persistent()
        .get(&DataKey::Composite(address.clone()));

    match composite {
        // A child that is not a registered composite is a leaf.
        None => Ok(vec![
            env,
            AllocationLeaf {
                strategy: address.clone(),
                amount: total,
            },
        ]),
        Some(composite) => {
            let mut leaves: Vec<AllocationLeaf> = Vec::new(env);
            for i in 0..composite.children.len() {
                let child = composite.children.get(i).unwrap();
                let child_amount = total
                    .checked_mul(child.weight_bps as i128)
                    .ok_or(Error::MathOverflow)?
                    / BPS_DENOMINATOR;
                let child_leaves = expand(env, &child.child, child_amount, depth + 1, &path)?;
                for j in 0..child_leaves.len() {
                    leaves.push_back(child_leaves.get(j).unwrap());
                }
            }
            Ok(leaves)
        }
    }
}

/// The fee that applies to capital flowing through a node, composing the fees
/// of every layer beneath it.
fn layered_fee(
    env: &Env,
    address: &Address,
    depth: u32,
    visited: &Vec<Address>,
) -> Result<u32, Error> {
    if depth > MAX_DEPTH {
        return Err(Error::MaxDepthExceeded);
    }
    for i in 0..visited.len() {
        if visited.get(i).unwrap() == *address {
            return Err(Error::CyclicReference);
        }
    }

    let mut path = visited.clone();
    path.push_back(address.clone());

    let composite: Option<CompositeStrategy> = env
        .storage()
        .persistent()
        .get(&DataKey::Composite(address.clone()));

    match composite {
        None => Ok(0),
        Some(composite) => {
            let mut total = composite.fee_bps as i128;
            for i in 0..composite.children.len() {
                let child = composite.children.get(i).unwrap();
                let child_fee = layered_fee(env, &child.child, depth + 1, &path)?;
                total += (child.weight_bps as i128) * (child_fee as i128) / BPS_DENOMINATOR;
            }
            Ok(total.min(MAX_FEE_BPS as i128) as u32)
        }
    }
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct StrategyComposability;

#[contractimpl]
impl StrategyComposability {
    /// Initialize the registry.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::CompositeCount, &0u32);

        env.events().publish((symbol_short!("init"),), admin);
        Ok(())
    }

    /// The admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    /// Register a composite strategy.
    ///
    /// `children` must be non-empty, free of duplicates and self-references, and
    /// their weights must sum to exactly 100%.
    pub fn register_composite(
        env: Env,
        admin: Address,
        address: Address,
        children: Vec<ChildWeight>,
        fee_bps: u32,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if env
            .storage()
            .persistent()
            .has(&DataKey::Composite(address.clone()))
        {
            return Err(Error::StrategyAlreadyRegistered);
        }
        if fee_bps > MAX_FEE_BPS {
            return Err(Error::InvalidWeight);
        }
        validate_children(&env, &address, &children)?;

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CompositeCount)
            .unwrap_or(0);

        let composite = CompositeStrategy {
            address: address.clone(),
            children,
            fee_bps,
            depth: 1,
        };
        let key = DataKey::Composite(address.clone());
        env.storage().persistent().set(&key, &composite);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.storage()
            .instance()
            .set(&DataKey::CompositeAt(count), &address);
        env.storage()
            .instance()
            .set(&DataKey::CompositeCount, &(count + 1));

        env.events()
            .publish((symbol_short!("comp_add"), address), fee_bps);
        Ok(())
    }

    /// Replace the children of an existing composite.
    pub fn set_children(
        env: Env,
        admin: Address,
        address: Address,
        children: Vec<ChildWeight>,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        let mut composite = load_composite(&env, &address)?;
        validate_children(&env, &address, &children)?;
        composite.children = children;

        let key = DataKey::Composite(address.clone());
        env.storage().persistent().set(&key, &composite);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("comp_set"), address), composite.children.len());
        Ok(())
    }

    /// The composite registered at an address, or an error if none exists.
    pub fn get_composite(env: Env, address: Address) -> Result<CompositeStrategy, Error> {
        load_composite(&env, &address)
    }

    /// How many composites are registered.
    pub fn composite_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CompositeCount)
            .unwrap_or(0)
    }

    /// # Resolve a composite into a flat allocation over leaf strategies
    ///
    /// The tree is expanded depth-first. A child that is itself a registered
    /// composite is expanded; a child that is not is a leaf. The result is the
    /// set of leaves and the amount that reaches each, which sums to `total`.
    pub fn resolve_allocation(
        env: Env,
        address: Address,
        total: i128,
    ) -> Result<Vec<AllocationLeaf>, Error> {
        if total < 0 {
            return Err(Error::InvalidAmount);
        }
        let visited: Vec<Address> = Vec::new(&env);
        expand(&env, &address, total, 0, &visited)
    }

    /// # The fee that applies through a composite
    ///
    /// Composes this layer's fee with the effective fee of each child, weighted
    /// by how much capital reaches that child. Capped at `MAX_FEE_BPS`.
    pub fn effective_fee_bps(env: Env, address: Address) -> Result<u32, Error> {
        let visited: Vec<Address> = Vec::new(&env);
        layered_fee(&env, &address, 0, &visited)
    }
}
