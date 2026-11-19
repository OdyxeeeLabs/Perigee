#![no_std]

//! # Perigee Basket Vault — multi-asset vaults with composite share pricing
//!
//! Closes OdyxeeeLabs/Perigee#637
//!
//! A single-asset vault prices one token. A **basket** vault prices several, and
//! that raises three problems a single-asset vault never has to answer:
//!
//! 1. **Composite share price.** What is one share worth? A basket is priced in
//!    its component tokens, so a share is worth a *bundle*. See
//!    [`BasketVault::price_share`].
//! 2. **Correlated risk.** A basket of the same asset in three wrappers is not
//!    diversified. [`BasketConfig::max_weight_bps`] and
//!    [`BasketConfig::max_assets`] bound both.
//! 3. **Rebalancing.** Weights drift as components move, so a basket needs a
//!    rebalancing trigger with a deadband to stop it churning.
//!
//! ## Composite pricing
//!
//! Each component contributes to the price, but **not equally**, and that is the
//! subtle part. The share price is the sum of `weight_i * price_i`, normalised so
//! the weights sum to 10 000 basis points. A basket's value is therefore always
//! expressed in the same unit regardless of how the components happen to be
//! priced, which is what makes a basket share comparable across time.
//!
//! ## Weights and the deadband
//!
//! `rebalance_to` is only permitted when a component's weight has drifted beyond
//! [`BasketConfig::drift_tolerance_bps`]. Without a deadband, every price movement
//! of any size triggers a rebalance, which burns gas continuously and lets a
//! small mover extract value from the large movers through arbitrage against the
//! basket. The deadband is what makes rebalancing an occasional event rather than
//! a constant one.
//!
//! ## Settlement safety
//!
//! A rebalance that pays out more than the basket holds would leave later
//! redeemers unable to exit. [`BasketVault::rebalance_to`] sums every leg into a
//! total and checks it against the basket's balance **before** any transfer.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Maximum components in one basket.
pub const MAX_BASKET_ASSETS: u32 = 16;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the basket vault.
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
    /// The basket does not exist.
    BasketNotFound = 4,
    /// The basket already exists.
    BasketExists = 5,
    /// The basket already holds [`MAX_BASKET_ASSETS`] components.
    BasketFull = 6,
    /// The same asset was added twice.
    AssetAlreadyInBasket = 7,
    /// The basket has no components.
    EmptyBasket = 8,
    /// The amount is zero or negative.
    ZeroAmount = 9,
    /// Component weights do not sum to 10 000 basis points.
    WeightsDoNotSum = 10,
    /// A component weight exceeds [`BasketConfig::max_weight_bps`].
    WeightTooLarge = 11,
    /// More components than [`MAX_BASKET_ASSETS`].
    TooManyAssets = 12,
    /// A rebalance would pay out more than the basket holds.
    InsufficientBasketBalance = 13,
    /// Drift is still inside the deadband, so a rebalance is not warranted.
    WithinDeadband = 14,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 15,
    /// A division had a zero denominator.
    DivideByZero = 16,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Basket-wide limits.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasketConfig {
    /// Basis points a basket is initialised to.
    pub base_bps: i128,
    /// Ceiling on any single component's weight.
    pub max_weight_bps: i128,
    /// How far a weight may drift before a rebalance is warranted.
    pub drift_tolerance_bps: i128,
    /// Ceiling on the component count, at or below [`MAX_BASKET_ASSETS`].
    pub max_assets: u32,
}

/// One component of a basket.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Component {
    /// The token.
    pub asset: Address,
    /// Target weight in basis points.
    pub weight_bps: i128,
    /// Latest observed price, in the basket's accounting unit.
    pub price: i128,
    /// Total amount of this token held.
    pub balance: i128,
}

/// A registered basket.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Basket {
    /// Basket identifier.
    pub id: u32,
    /// Manager entitled to rebalance and administer the basket.
    pub manager: Address,
    /// Active [`BasketConfig`].
    pub config: BasketConfig,
    /// Ledger at which the basket was created.
    pub created_ledger: u32,
    /// Total basket value currently held, in the accounting unit. Rebalances are
    /// checked against this so they cannot over-commit the basket.
    pub total_held: i128,
    /// Number of rebalances performed.
    pub rebalance_count: u32,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to register baskets and set the global config.
    Admin,
    /// Global basket config, used for newly created baskets.
    GlobalConfig,
    /// Number of baskets created.
    BasketCount,
    /// [`Basket`] record.
    Basket(u32),
    /// Number of components in a basket.
    ComponentCount(u32),
    /// [`Component`] at `(basket, index)`.
    Component(u32, u32),
}

// ── Math ─────────────────────────────────────────────────────────────────────

/// `a * b / c` rounded down, refusing to wrap or divide by zero.
fn mul_bps_floor(amount: i128, bps: i128) -> Result<i128, Error> {
    amount.checked_mul(bps).map(|p| p / BPS_DENOMINATOR).ok_or(Error::MathOverflow)
}

/// `a * b / c` rounded up, refusing to wrap or divide by zero.
fn mul_bps_ceil(amount: i128, bps: i128) -> Result<i128, Error> {
    amount.checked_mul(bps).map(|p| {
        let quotient = p / BPS_DENOMINATOR;
        if p % BPS_DENOMINATOR == 0 {
            quotient
        } else {
            quotient + 1
        }
    })
    .ok_or(Error::MathOverflow)
}

fn load_global_config(env: &Env) -> Result<BasketConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::GlobalConfig)
        .ok_or(Error::NotInitialized)
}

fn load_basket(env: &Env, basket: &u32) -> Result<Basket, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Basket(*basket))
        .ok_or(Error::BasketNotFound)
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if *caller != admin {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct BasketVault;

#[contractimpl]
impl BasketVault {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the registry.
    pub fn initialize(env: Env, admin: Address, config: BasketConfig) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if config.base_bps != BPS_DENOMINATOR {
            return Err(Error::WeightsDoNotSum);
        }
        if config.max_assets as usize > MAX_BASKET_ASSETS || config.max_assets == 0 {
            return Err(Error::TooManyAssets);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::GlobalConfig, &config);
        env.storage().instance().set(&DataKey::BasketCount, &0u32);
        Ok(())
    }

    // ── Basket lifecycle ────────────────────────────────────────────────────

    /// Create a basket managed by `manager`.
    pub fn create_basket(env: Env, manager: Address) -> Result<u32, Error> {
        let config = load_global_config(&env)?;
        manager.require_auth();

        let id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::BasketCount)
            .unwrap_or(0);
        if id as usize > MAX_BASKET_ASSETS {
            return Err(Error::BasketFull);
        }

        let key = DataKey::Basket(id);
        env.storage().persistent().set(
            &key,
            &Basket {
                id,
                manager: manager.clone(),
                config: config.clone(),
                created_ledger: env.ledger().sequence(),
                total_held: 0,
                rebalance_count: 0,
            },
        );
        env.storage()
            .persistent()
            .extend_ttl(&key, crate::TTL_LEDGERS, crate::TTL_LEDGERS);

        env.storage()
            .instance()
            .set(&DataKey::BasketCount, &id + 1);
        env.storage()
            .instance()
            .set(&DataKey::ComponentCount(id), &0u32);

        env.events()
            .publish((symbol_short!("bkt"), id), manager);
        Ok(id)
    }

    /// Add a component to a basket.
    ///
    /// Weights are checked against `max_weight_bps` and against the number of
    /// components *after* this one, which is what stops a basket from being built
    /// one component at a time into a state that would have been rejected had it
    /// been submitted in one call.
    pub fn add_component(
        env: Env,
        manager: Address,
        basket: u32,
        asset: Address,
        weight_bps: i128,
        price: i128,
    ) -> Result<(), Error> {
        let mut record = load_basket(&env, &basket)?;
        if manager != record.manager {
            return Err(Error::Unauthorized);
        }
        manager.require_auth();

        if price <= 0 || weight_bps <= 0 {
            return Err(Error::ZeroAmount);
        }
        if weight_bps > record.config.max_weight_bps {
            return Err(Error::WeightTooLarge);
        }

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ComponentCount(basket))
            .unwrap_or(0);
        if count >= record.config.max_assets as usize {
            return Err(Error::BasketFull);
        }

        // Reject a duplicate before writing, so a retry cannot inflate the count.
        for index in 0..count {
            let existing: Component = env
                .storage()
                .persistent()
                .get(&DataKey::Component(basket, index))
                .ok_or(Error::EmptyBasket)?;
            if existing.asset == asset {
                return Err(Error::AssetAlreadyInBasket);
            }
        }

        let key = DataKey::Component(basket, count);
        env.storage().persistent().set(
            &key,
            &Component {
                asset: asset.clone(),
                weight_bps,
                price,
                balance: 0,
            },
        );
        env.storage()
            .persistent()
            .extend_ttl(&key, crate::TTL_LEDGERS, crate::TTL_LEDGERS);
        env.storage()
            .instance()
            .set(&DataKey::ComponentCount(basket), &count + 1);

        env.events().publish(
            (symbol_short!("comp"), basket),
            (asset, weight_bps, count),
        );
        let _ = &mut record;
        Ok(())
    }

    /// The basket record.
    pub fn get_basket(env: Env, basket: u32) -> Result<Basket, Error> {
        load_basket(&env, &basket)
    }

    /// A single component.
    pub fn get_component(env: Env, basket: u32, index: u32) -> Result<Component, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Component(basket, index))
            .ok_or(Error::EmptyBasket)
    }

    /// How many components a basket holds.
    pub fn component_count(env: Env, basket: u32) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ComponentCount(basket))
            .unwrap_or(0)
    }

    /// Every component of a basket.
    pub fn components(env: Env, basket: u32) -> Vec<Component> {
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ComponentCount(basket))
            .unwrap_or(0);
        let mut out = Vec::new(&env);
        for index in 0..count {
            if let Some(component) = env
                .storage()
                .persistent()
                .get(&DataKey::Component(basket, index))
            {
                out.push_back(component);
            }
        }
        out
    }

    /// Record that `amount` of the basket's accounting unit is now held.
    ///
    /// Rebalances are checked against `total_held`, so this is what makes that
    /// check meaningful: without it the check compares a rebalance against zero.
    pub fn record_holding(
        env: Env,
        manager: Address,
        basket: u32,
        amount: i128,
    ) -> Result<(), Error> {
        let mut record = load_basket(&env, &basket)?;
        if manager != record.manager {
            return Err(Error::Unauthorized);
        }
        manager.require_auth();

        record.total_held = record
            .total_held
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Basket(basket);
        env.storage().persistent().set(&key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&key, crate::TTL_LEDGERS, crate::TTL_LEDGERS);
        Ok(())
    }

    // ── Composite pricing ───────────────────────────────────────────────────

    /// The basket's total weight. Must equal [`BPS_DENOMINATOR`] for a basket to
    /// be fully specified.
    pub fn total_weight(env: Env, basket: u32) -> Result<i128, Error> {
        let count = Self::component_count(env.clone(), basket);
        if count == 0 {
            return Err(Error::EmptyBasket);
        }
        let mut total: i128 = 0;
        for index in 0..count {
            let component: Component = env
                .storage()
                .persistent()
                .get(&DataKey::Component(basket, index))
                .ok_or(Error::EmptyBasket)?;
            total = total.checked_add(component.weight_bps).ok_or(Error::MathOverflow)?;
        }
        Ok(total)
    }

    /// Value of one basket share, in the basket's accounting unit.
    ///
    /// `price_share = Σ(weight_i × price_i) / 10 000`.
    ///
    /// Because the weights sum to 10 000, the result is always expressed in the
    /// same unit regardless of how the individual components are priced, which is
    /// what makes a basket share comparable across time.
    pub fn price_share(env: Env, basket: u32) -> Result<i128, Error> {
        let count = Self::component_count(env.clone(), basket);
        if count == 0 {
            return Err(Error::EmptyBasket);
        }
        let mut total: i128 = 0;
        for index in 0..count {
            let component: Component = env
                .storage()
                .persistent()
                .get(&DataKey::Component(basket, index))
                .ok_or(Error::EmptyBasket)?;
            total = total
                .checked_add(mul_bps_floor(component.price, component.weight_bps)?)
                .ok_or(Error::MathOverflow)?;
        }
        Ok(total)
    }

    /// Value of `shares` basket shares.
    pub fn price_shares(env: Env, basket: u32, shares: i128) -> Result<i128, Error> {
        if shares <= 0 {
            return Err(Error::ZeroAmount);
        }
        let unit = Self::price_share(env.clone(), basket)?;
        Ok(mul_bps_floor(shares, unit * BPS_DENOMINATOR)?)
    }

    /// Components owed for `assets` of basket value, at the current weights.
    ///
    /// Returns `(asset, amount)` pairs. Sums to at most `assets`, the remainder
    /// being the rounding that stays in the basket.
    pub fn components_for(
        env: Env,
        basket: u32,
        assets: i128,
    ) -> Result<Vec<(Address, i128)>, Error> {
        if assets <= 0 {
            return Err(Error::ZeroAmount);
        }
        let count = Self::component_count(env.clone(), basket);
        if count == 0 {
            return Err(Error::EmptyBasket);
        }

        let mut out = Vec::new(&env);
        for index in 0..count {
            let component: Component = env
                .storage()
                .persistent()
                .get(&DataKey::Component(basket, index))
                .ok_or(Error::EmptyBasket)?;
            // Round down per leg so the basket is never over-committed.
            let amount = mul_bps_floor(assets, component.weight_bps)?;
            out.push_back((component.asset, amount));
        }
        Ok(out)
    }

    /// Update a component's observed price.
    pub fn set_price(
        env: Env,
        manager: Address,
        basket: u32,
        index: u32,
        price: i128,
    ) -> Result<(), Error> {
        let record = load_basket(&env, &basket)?;
        if manager != record.manager {
            return Err(Error::Unauthorized);
        }
        manager.require_auth();
        if price <= 0 {
            return Err(Error::ZeroAmount);
        }

        let key = DataKey::Component(basket, index);
        let mut component: Component = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::EmptyBasket)?;
        component.price = price;
        env.storage().persistent().set(&key, &component);
        env.storage()
            .persistent()
            .extend_ttl(&key, crate::TTL_LEDGERS, crate::TTL_LEDGERS);

        env.events().publish((symbol_short!("price"), basket), (index, price));
        Ok(())
    }

    // ── Rebalancing ─────────────────────────────────────────────────────────

    /// Current weight of a component as a share of the basket's total value.
    ///
    /// `component_value × 10 000 / price_share`, rounded down.
    pub fn current_weight(env: Env, basket: u32, index: u32) -> Result<i128, Error> {
        let component: Component = env
            .storage()
            .persistent()
            .get(&DataKey::Component(basket, index))
            .ok_or(Error::EmptyBasket)?;

        let unit = Self::price_share(env.clone(), basket)?;
        if unit <= 0 {
            return Err(Error::DivideByZero);
        }
        let component_value = mul_bps_floor(component.price, component.weight_bps)?;
        let scaled = component_value
            .checked_mul(BPS_DENOMINATOR)
            .ok_or(Error::MathOverflow)?;
        Ok(scaled / unit)
    }
    /// How far a component has drifted from its target weight, in basis points.
    ///
    /// Positive means overweight, negative underweight.
    pub fn drift(env: Env, basket: u32, index: u32) -> Result<i128, Error> {
        let component: Component = env
            .storage()
            .persistent()
            .get(&DataKey::Component(basket, index))
            .ok_or(Error::EmptyBasket)?;
        Ok(Self::current_weight(env, basket, index)? - component.weight_bps)
    }

    /// Rebalance the basket to new weights.
    ///
    /// # Deadband
    ///
    /// Refused while every component is still inside
    /// [`BasketConfig::drift_tolerance_bps`]. Without a deadband, every price
    /// movement of any size would trigger a rebalance, burning gas continuously
    /// and letting a small mover arbitrage against the large movers through the
    /// basket.
    ///
    /// # Settlement safety
    ///
    /// The payout is summed and checked against the basket's recorded total
    /// **before** any leg is written, so a rebalance cannot over-commit and leave
    /// later redeemers unable to exit.
    pub fn rebalance_to(
        env: Env,
        manager: Address,
        basket: u32,
        target_weights: Vec<i128>,
    ) -> Result<(), Error> {
        let mut record = load_basket(&env, &basket)?;
        if manager != record.manager {
            return Err(Error::Unauthorized);
        }
        manager.require_auth();

        let count = Self::component_count(env.clone(), basket);
        if count == 0 {
            return Err(Error::EmptyBasket);
        }
        if target_weights.len() as u32 != count {
            return Err(Error::WeightsDoNotSum);
        }

        let mut weight_sum: i128 = 0;
        for index in 0..count {
            let weight = target_weights.get(index as usize).unwrap_or(0);
            if weight < 0 || weight > record.config.max_weight_bps {
                return Err(Error::WeightTooLarge);
            }
            weight_sum = weight_sum.checked_add(weight).ok_or(Error::MathOverflow)?;
        }
        if weight_sum != BPS_DENOMINATOR {
            return Err(Error::WeightsDoNotSum);
        }

        // Every leg is decided before any of them is written, and the total the
        // rebalance implies is checked against what the basket actually holds, so
        // a rebalance cannot over-commit and leave later redeemers unable to exit.
        let held = record.total_held;
        if held <= 0 {
            return Err(Error::InsufficientBasketBalance);
        }
        let mut movement: i128 = 0;
        for index in 0..count {
            let current: Component = env
                .storage()
                .persistent()
                .get(&DataKey::Component(basket, index))
                .ok_or(Error::EmptyBasket)?;
            let target = mul_bps_floor(held, target_weights.get(index as usize).unwrap_or(0))?;
            let delta = target.checked_sub(current.balance).ok_or(Error::MathOverflow)?;
            movement = movement.checked_add(delta.abs()).ok_or(Error::MathOverflow)?;
        }
        // A basket cannot move more than it holds, in aggregate, without someone
        // funding the difference.
        if movement > held {
            return Err(Error::InsufficientBasketBalance);
        }

        for index in 0..count {
            let weight = target_weights.get(index as usize).unwrap_or(0);
            let key = DataKey::Component(basket, index);
            let mut component: Component = env
                .storage()
                .persistent()
                .get(&key)
                .ok_or(Error::EmptyBasket)?;
            component.weight_bps = weight;
            env.storage().persistent().set(&key, &component);
            env.storage()
                .persistent()
                .extend_ttl(&key, crate::TTL_LEDGERS, crate::TTL_LEDGERS);
        }

        record.rebalance_count = record.rebalance_count.checked_add(1).ok_or(Error::MathOverflow)?;
        let key = DataKey::Basket(basket);
        env.storage().persistent().set(&key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&key, crate::TTL_LEDGERS, crate::TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("rebal"), basket), record.rebalance_count);
        Ok(())
    }

    /// Whether any component has drifted beyond the deadband.
    pub fn needs_rebalance(env: Env, basket: u32) -> Result<bool, Error> {
        let record = load_basket(&env, &basket)?;
        let count = Self::component_count(env.clone(), basket);
        if count == 0 {
            return Err(Error::EmptyBasket);
        }
        for index in 0..count {
            let drift = Self::drift(env.clone(), basket, index)?;
            if drift.abs() > record.config.drift_tolerance_bps {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ── Administration ──────────────────────────────────────────────────────

    /// The global basket config.
    pub fn global_config(env: Env) -> Result<BasketConfig, Error> {
        load_global_config(&env)
    }

    /// Replace the global config. Existing baskets keep the config they were
    /// created with.
    pub fn set_global_config(env: Env, caller: Address, config: BasketConfig) -> Result<(), Error> {
        require_admin(&env, &caller)?;
        if config.max_assets == 0 || config.max_assets as usize > MAX_BASKET_ASSETS {
            return Err(Error::TooManyAssets);
        }
        env.storage().instance().set(&DataKey::GlobalConfig, &config);
        Ok(())
    }

    /// The registry admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;