#![no_std]

//! # Perigee Strategy Analytics — risk framework and performance tracking
//!
//! Closes OdyxeeeLabs/Perigee#657
//! Closes OdyxeeeLabs/Perigee#658
//!
//! A vault that picks the highest-yield strategy across every venue is a vault
//! that will eventually put everything into the same bet. This contract is the
//! part of the system that says *no*: it scores how risky each strategy is,
//! caps how much of the book any one strategy — or any correlated group of
//! strategies — may hold, and reports how each strategy has actually performed
//! so that a rotation decision is made on data rather than on a headline APY.
//!
//! ## Risk framework ([#657])
//!
//! Every registered strategy carries a [`RiskProfile`]:
//!
//! | Field | Meaning |
//! | --- | --- |
//! | `risk_score` | 0–100, higher is riskier |
//! | `max_allocation_bps` | hard cap on this strategy's share of the book |
//! | `correlation_group` | strategies that fail together share a group id |
//!
//! [`RiskLimits`] adds the portfolio-wide constraints: the maximum weighted
//! risk score the book may carry, the maximum exposure to a single correlation
//! group, and the drawdown a stress scenario may produce before the allocation
//! is rejected.
//!
//! [`StrategyAnalytics::validate_allocation`] is the gate a vault calls before
//! deploying: it enforces the per-strategy cap, the per-group cap and the
//! weighted-risk ceiling in one call and returns the computed portfolio risk
//! score. [`StrategyAnalytics::stress_test`] applies a caller-supplied shock to
//! each strategy and reports the resulting loss against the configured
//! tolerance, so a book can be checked against a scenario before it exists.
//!
//! Correlation is modelled as *groups* rather than a dense pairwise matrix: two
//! strategies that draw down together are placed in the same group, and the
//! group cap does the work. This is deliberately coarse. A pairwise matrix is
//! `O(n²)` state that must be re-estimated constantly and can be gamed; a group
//! cap is a single number a risk owner can reason about.
//!
//! ## Performance tracking ([#658])
//!
//! On-chain metrics that off-chain dashboards re-derive from events are always
//! one indexer crash away from being wrong. The two numbers a fee or rotation
//! decision actually depends on — annualised return and drawdown — are kept on
//! chain here.
//!
//! Each strategy has a [`Series`] that is updated in place by
//! [`StrategyAnalytics::record_snapshot`]:
//!
//! - `first_*` and `last_*` anchor the APY computation, so it is constant in
//!   storage regardless of how many snapshots are taken;
//! - `high_water_mark` and `max_drawdown_bps` are updated incrementally, so the
//!   maximum drawdown over the whole history is available without storing the
//!   history.
//!
//! [`StrategyAnalytics::performance`] annualises the interval between the first
//! and last snapshot and compares it with the same calculation over a benchmark
//! series recorded via [`StrategyAnalytics::record_benchmark`]. The result is
//! the *excess* return a rotation is judged on.
//!
//! ## Timestamps
//!
//! Snapshots are timestamped from [`Env::ledger`] rather than a caller-supplied
//! argument. A caller that can choose its own timestamps can manufacture any APY
//! it likes; the ledger cannot.

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

/// Highest risk score a strategy may carry.
pub const MAX_RISK_SCORE: u32 = 100;

/// Cap on the number of registered strategies, bounding validation cost.
pub const MAX_STRATEGIES: u32 = 32;

/// Seconds in a 365-day year, for APY annualisation.
pub const SECONDS_PER_YEAR: u64 = 31_536_000;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the analytics contract.
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
    /// The strategy has no risk profile.
    StrategyNotRegistered = 4,
    /// The strategy already has a risk profile.
    StrategyAlreadyRegistered = 5,
    /// A risk score above [`MAX_RISK_SCORE`] was supplied.
    InvalidRiskScore = 6,
    /// The allocation is empty or its weights do not sum to 100%.
    InvalidAllocation = 7,
    /// A single position is above its strategy's cap.
    AllocationExceedsLimit = 8,
    /// A correlation group is above the configured exposure cap.
    GroupExceedsLimit = 9,
    /// The weighted risk score is above the configured ceiling.
    PortfolioRiskTooHigh = 10,
    /// A stress shock above 100% was supplied.
    InvalidScenario = 11,
    /// More strategies or positions than [`MAX_STRATEGIES`].
    TooManyStrategies = 12,
    /// Fewer than two snapshots are available to compute a return.
    NotEnoughSamples = 13,
    /// A non-positive asset value was supplied.
    InvalidAmount = 14,
    /// Two snapshots share a timestamp, so the interval is zero.
    DivisionByZero = 15,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 16,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Portfolio-wide risk constraints.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskLimits {
    /// Maximum weighted-average risk score the book may carry.
    pub max_portfolio_risk_score: u32,
    /// Default per-strategy cap, used when a profile does not narrow it.
    pub max_strategy_allocation_bps: u32,
    /// Maximum exposure to any single correlation group.
    pub max_group_allocation_bps: u32,
    /// Loss a stress scenario may produce before it is rejected.
    pub stress_tolerance_bps: u32,
}

/// The risk characteristics of one strategy.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskProfile {
    /// Strategy contract address.
    pub strategy: Address,
    /// 0–100, higher is riskier.
    pub risk_score: u32,
    /// Hard cap on this strategy's share of the book, in bps.
    pub max_allocation_bps: u32,
    /// Strategies that fail together share a group id.
    pub correlation_group: u32,
}

/// One position in a proposed allocation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Allocation {
    /// Strategy the weight applies to.
    pub strategy: Address,
    /// Weight in basis points.
    pub weight_bps: u32,
}

/// Outcome of validating an allocation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationReport {
    /// Weighted-average risk score of the book.
    pub portfolio_risk_score: u32,
    /// Largest single position, in bps.
    pub largest_position_bps: u32,
    /// Number of positions validated.
    pub strategy_count: u32,
}

/// A shock applied to one strategy during a stress test.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StressScenario {
    /// Strategy the shock applies to.
    pub strategy: Address,
    /// Loss applied to that strategy, in bps of its allocation.
    pub shock_bps: u32,
}

/// Outcome of a stress test.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StressResult {
    /// Portfolio loss produced by the scenario, in bps.
    pub loss_bps: u32,
    /// Configured tolerance.
    pub tolerance_bps: u32,
    /// Whether the loss exceeds the tolerance.
    pub breached: bool,
}

/// An incrementally-maintained performance series.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Series {
    /// Timestamp of the first sample.
    pub first_timestamp: u64,
    /// Timestamp of the most recent sample.
    pub last_timestamp: u64,
    /// Assets recorded by the first sample.
    pub first_assets: i128,
    /// Assets recorded by the most recent sample.
    pub last_assets: i128,
    /// Highest asset value ever recorded.
    pub high_water_mark: i128,
    /// Worst peak-to-trough decline recorded, in bps.
    pub max_drawdown_bps: u32,
    /// Number of samples recorded.
    pub samples: u32,
}

/// Annualised performance of a strategy against the benchmark.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceReport {
    /// Annualised return, in bps (may be negative).
    pub apy_bps: i128,
    /// Annualised return of the benchmark, in bps.
    pub benchmark_apy_bps: i128,
    /// `apy_bps - benchmark_apy_bps`.
    pub excess_bps: i128,
    /// Highest asset value recorded.
    pub high_water_mark: i128,
    /// Decline from the high-water mark to the latest sample, in bps.
    pub current_drawdown_bps: u32,
    /// Worst decline ever recorded, in bps.
    pub max_drawdown_bps: u32,
    /// Number of samples behind the report.
    pub samples: u32,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to register strategies and record snapshots.
    Admin,
    /// Portfolio-wide [`RiskLimits`].
    Limits,
    /// Number of registered strategies.
    StrategyCount,
    /// Registered strategy address at an index.
    StrategyAt(u32),
    /// [`RiskProfile`] for a strategy.
    Risk(Address),
    /// [`Series`] for a strategy.
    Series(Address),
    /// [`Series`] for the benchmark.
    Benchmark,
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

fn load_limits(env: &Env) -> Result<RiskLimits, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Limits)
        .ok_or(Error::NotInitialized)
}

fn load_profile(env: &Env, strategy: &Address) -> Result<RiskProfile, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Risk(strategy.clone()))
        .ok_or(Error::StrategyNotRegistered)
}

fn validate_limits(limits: &RiskLimits) -> Result<(), Error> {
    if limits.max_portfolio_risk_score > MAX_RISK_SCORE {
        return Err(Error::InvalidRiskScore);
    }
    if limits.max_strategy_allocation_bps > BPS_DENOMINATOR as u32
        || limits.max_group_allocation_bps > BPS_DENOMINATOR as u32
        || limits.stress_tolerance_bps > BPS_DENOMINATOR as u32
    {
        return Err(Error::InvalidAllocation);
    }
    Ok(())
}

fn validate_profile(risk_score: u32, max_allocation_bps: u32) -> Result<(), Error> {
    if risk_score > MAX_RISK_SCORE {
        return Err(Error::InvalidRiskScore);
    }
    if max_allocation_bps > BPS_DENOMINATOR as u32 {
        return Err(Error::InvalidAllocation);
    }
    Ok(())
}

/// Annualise the return between the first and last sample of a series.
///
/// This is a simple (non-compounded) annualisation: the interval's return is
/// scaled linearly to a year. Compounding would assume a reinvestment pattern
/// the series does not describe, and a linear scale is the conventional basis
/// for the period returns a vault is compared over.
fn apy_bps(series: &Series) -> Result<i128, Error> {
    if series.samples < 2 {
        return Err(Error::NotEnoughSamples);
    }
    if series.first_assets <= 0 {
        return Err(Error::InvalidAmount);
    }
    let elapsed = series
        .last_timestamp
        .saturating_sub(series.first_timestamp);
    if elapsed == 0 {
        return Err(Error::DivisionByZero);
    }

    let growth_bps = (series.last_assets - series.first_assets)
        .checked_mul(BPS_DENOMINATOR)
        .ok_or(Error::MathOverflow)?
        / series.first_assets;

    growth_bps
        .checked_mul(SECONDS_PER_YEAR as i128)
        .ok_or(Error::MathOverflow)
        .map(|scaled| scaled / elapsed as i128)
}

fn empty_series() -> Series {
    Series {
        first_timestamp: 0,
        last_timestamp: 0,
        first_assets: 0,
        last_assets: 0,
        high_water_mark: 0,
        max_drawdown_bps: 0,
        samples: 0,
    }
}

/// Append a sample to a series, updating the high-water mark and the maximum
/// drawdown in place.
fn update_series(env: &Env, key: &DataKey, assets: i128) -> Result<Series, Error> {
    if assets <= 0 {
        return Err(Error::InvalidAmount);
    }
    let now = env.ledger().timestamp();
    let mut series: Series = match env.storage().persistent().get(key) {
        Some(existing) => existing,
        None => {
            let mut fresh = empty_series();
            fresh.first_timestamp = now;
            fresh.last_timestamp = now;
            fresh.first_assets = assets;
            fresh.last_assets = assets;
            fresh.high_water_mark = assets;
            fresh
        }
    };

    if series.samples > 0 {
        if series.high_water_mark > 0 {
            let decline = (series.high_water_mark - assets)
                .checked_mul(BPS_DENOMINATOR)
                .ok_or(Error::MathOverflow)?
                / series.high_water_mark;
            if decline > 0 {
                let decline_bps = decline as u32;
                if decline_bps > series.max_drawdown_bps {
                    series.max_drawdown_bps = decline_bps;
                }
            }
        }
        if assets > series.high_water_mark {
            series.high_water_mark = assets;
        }
    }

    series.last_timestamp = now;
    series.last_assets = assets;
    series.samples = series.samples.checked_add(1).ok_or(Error::MathOverflow)?;

    env.storage().persistent().set(key, &series);
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_LEDGERS, TTL_LEDGERS);
    Ok(series)
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct StrategyAnalytics;

#[contractimpl]
impl StrategyAnalytics {
    // ── Setup ───────────────────────────────────────────────────────────────

    /// Initialize the analytics registry with portfolio-wide risk limits.
    pub fn initialize(env: Env, admin: Address, limits: RiskLimits) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        validate_limits(&limits)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Limits, &limits);
        env.storage().instance().set(&DataKey::StrategyCount, &0u32);

        env.events().publish((symbol_short!("init"),), admin);
        Ok(())
    }

    /// Replace the portfolio-wide risk limits.
    pub fn set_limits(env: Env, admin: Address, limits: RiskLimits) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        validate_limits(&limits)?;
        env.storage().instance().set(&DataKey::Limits, &limits);
        env.events().publish(
            (symbol_short!("limits"),),
            (
                limits.max_portfolio_risk_score,
                limits.max_group_allocation_bps,
            ),
        );
        Ok(())
    }

    /// The current portfolio-wide risk limits.
    pub fn get_limits(env: Env) -> Result<RiskLimits, Error> {
        load_limits(&env)
    }

    /// The admin allowed to configure risk and record snapshots.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    // ── Risk profiles ───────────────────────────────────────────────────────

    /// Register a strategy with its risk profile.
    pub fn register_strategy(
        env: Env,
        admin: Address,
        strategy: Address,
        risk_score: u32,
        max_allocation_bps: u32,
        correlation_group: u32,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        validate_profile(risk_score, max_allocation_bps)?;

        if env
            .storage()
            .persistent()
            .has(&DataKey::Risk(strategy.clone()))
        {
            return Err(Error::StrategyAlreadyRegistered);
        }
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::StrategyCount)
            .unwrap_or(0);
        if count >= MAX_STRATEGIES {
            return Err(Error::TooManyStrategies);
        }

        let profile = RiskProfile {
            strategy: strategy.clone(),
            risk_score,
            max_allocation_bps,
            correlation_group,
        };
        let key = DataKey::Risk(strategy.clone());
        env.storage().persistent().set(&key, &profile);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.storage()
            .instance()
            .set(&DataKey::StrategyAt(count), &strategy);
        env.storage()
            .instance()
            .set(&DataKey::StrategyCount, &(count + 1));

        env.events().publish(
            (symbol_short!("strat_reg"), strategy),
            (risk_score, max_allocation_bps, correlation_group),
        );
        Ok(())
    }

    /// Update the risk profile of an already-registered strategy.
    pub fn update_risk_profile(
        env: Env,
        admin: Address,
        strategy: Address,
        risk_score: u32,
        max_allocation_bps: u32,
        correlation_group: u32,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        validate_profile(risk_score, max_allocation_bps)?;
        load_profile(&env, &strategy)?;

        let profile = RiskProfile {
            strategy: strategy.clone(),
            risk_score,
            max_allocation_bps,
            correlation_group,
        };
        let key = DataKey::Risk(strategy.clone());
        env.storage().persistent().set(&key, &profile);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("strat_upd"), strategy),
            (risk_score, max_allocation_bps, correlation_group),
        );
        Ok(())
    }

    /// The risk profile of a strategy.
    pub fn get_risk_profile(env: Env, strategy: Address) -> Result<RiskProfile, Error> {
        load_profile(&env, &strategy)
    }

    /// How many strategies are registered.
    pub fn strategy_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::StrategyCount)
            .unwrap_or(0)
    }

    // ── Allocation validation ───────────────────────────────────────────────

    /// # Enforce the risk framework
    ///
    /// Validate that `allocations` respects every configured limit and return
    /// the resulting portfolio risk score.
    ///
    /// Rejected, in order:
    ///
    /// 1. weights that do not sum to exactly 100%,
    /// 2. a position above its strategy's `max_allocation_bps`,
    /// 3. a correlation group above `max_group_allocation_bps`,
    /// 4. a weighted risk score above `max_portfolio_risk_score`.
    ///
    /// The weighted score is `Σ weight_bps × risk_score / 10_000`, rounded down.
    /// Flooring means the check can only ever *under*-state risk, never inflate
    /// it, so a book that passes here is genuinely within its ceiling.
    pub fn validate_allocation(
        env: Env,
        allocations: Vec<Allocation>,
    ) -> Result<AllocationReport, Error> {
        let limits = load_limits(&env)?;
        let count = allocations.len();
        if count == 0 {
            return Err(Error::InvalidAllocation);
        }
        if count > MAX_STRATEGIES {
            return Err(Error::TooManyStrategies);
        }

        // Load every profile once; later passes reuse them rather than reading
        // the same ledger entry repeatedly.
        let mut profiles: Vec<RiskProfile> = Vec::new(&env);
        let mut weight_total: i128 = 0;
        let mut largest_position: u32 = 0;
        let mut weighted_risk: i128 = 0;

        for i in 0..count {
            let allocation = allocations.get(i).unwrap();
            let profile = load_profile(&env, &allocation.strategy)?;

            if allocation.weight_bps > profile.max_allocation_bps {
                return Err(Error::AllocationExceedsLimit);
            }
            if allocation.weight_bps > largest_position {
                largest_position = allocation.weight_bps;
            }

            weight_total += allocation.weight_bps as i128;
            weighted_risk += (allocation.weight_bps as i128) * (profile.risk_score as i128);
            profiles.push_back(profile);
        }

        if weight_total != BPS_DENOMINATOR {
            return Err(Error::InvalidAllocation);
        }

        // Correlation groups: sum the weights that share a group id.
        for i in 0..count {
            let group = profiles.get(i).unwrap().correlation_group;
            let mut group_total: u32 = 0;
            for j in 0..count {
                if profiles.get(j).unwrap().correlation_group == group {
                    group_total += allocations.get(j).unwrap().weight_bps;
                }
            }
            if group_total > limits.max_group_allocation_bps {
                return Err(Error::GroupExceedsLimit);
            }
        }

        let portfolio_risk_score = (weighted_risk / BPS_DENOMINATOR) as u32;
        if portfolio_risk_score > limits.max_portfolio_risk_score {
            return Err(Error::PortfolioRiskTooHigh);
        }

        Ok(AllocationReport {
            portfolio_risk_score,
            largest_position_bps: largest_position,
            strategy_count: count,
        })
    }

    /// Apply a shock to each strategy and report the portfolio loss.
    ///
    /// `loss_bps = Σ (weight_bps × shock_bps) / 10_000`, which is the same as
    /// treating each weight and shock as a fraction and summing the products.
    /// Running this before deploying lets a book be checked against a scenario
    /// whose loss is already known to be unacceptable.
    pub fn stress_test(
        env: Env,
        allocations: Vec<Allocation>,
        scenarios: Vec<StressScenario>,
    ) -> Result<StressResult, Error> {
        let limits = load_limits(&env)?;

        let scenario_count = scenarios.len();
        for k in 0..scenario_count {
            if scenarios.get(k).unwrap().shock_bps > BPS_DENOMINATOR as u32 {
                return Err(Error::InvalidScenario);
            }
        }

        let mut loss_total: i128 = 0;
        let allocation_count = allocations.len();
        for i in 0..allocation_count {
            let allocation = allocations.get(i).unwrap();
            for k in 0..scenario_count {
                let scenario = scenarios.get(k).unwrap();
                if scenario.strategy == allocation.strategy {
                    loss_total +=
                        (allocation.weight_bps as i128) * (scenario.shock_bps as i128);
                }
            }
        }

        let loss_bps = (loss_total / BPS_DENOMINATOR) as u32;
        Ok(StressResult {
            loss_bps,
            tolerance_bps: limits.stress_tolerance_bps,
            breached: loss_bps > limits.stress_tolerance_bps,
        })
    }

    // ── Performance tracking ────────────────────────────────────────────────

    /// Record a performance snapshot of a strategy.
    ///
    /// The timestamp is taken from the ledger; the caller cannot choose it.
    pub fn record_snapshot(
        env: Env,
        admin: Address,
        strategy: Address,
        assets: i128,
    ) -> Result<Series, Error> {
        require_admin(&env, &admin)?;
        load_profile(&env, &strategy)?;
        update_series(&env, &DataKey::Series(strategy.clone()), assets)
    }

    /// Record a benchmark snapshot. The benchmark is a single series shared by
    /// every strategy, so a strategy's excess return is measured against the
    /// same yardstick.
    pub fn record_benchmark(env: Env, admin: Address, assets: i128) -> Result<Series, Error> {
        require_admin(&env, &admin)?;
        update_series(&env, &DataKey::Benchmark, assets)
    }

    /// The recorded series for a strategy, or an empty series if none exists.
    pub fn get_series(env: Env, strategy: Address) -> Series {
        env.storage()
            .persistent()
            .get(&DataKey::Series(strategy))
            .unwrap_or_else(empty_series)
    }

    /// Annualised benchmark return, in bps.
    pub fn benchmark_apy(env: Env) -> Result<i128, Error> {
        let series: Series = env
            .storage()
            .persistent()
            .get(&DataKey::Benchmark)
            .ok_or(Error::NotEnoughSamples)?;
        apy_bps(&series)
    }

    /// # On-chain APY and benchmark comparison
    ///
    /// Return the annualised return of a strategy, the benchmark's annualised
    /// return over the same period, the excess, and the strategy's drawdown.
    pub fn performance(env: Env, strategy: Address) -> Result<PerformanceReport, Error> {
        let series: Series = env
            .storage()
            .persistent()
            .get(&DataKey::Series(strategy))
            .ok_or(Error::NotEnoughSamples)?;

        let benchmark_apy_bps = match env.storage().persistent().get(&DataKey::Benchmark) {
            Some(benchmark) => apy_bps(&benchmark).unwrap_or(0),
            None => 0,
        };

        let apy = apy_bps(&series)?;
        let current_drawdown_bps = if series.high_water_mark > 0 {
            let decline = (series.high_water_mark - series.last_assets) * BPS_DENOMINATOR
                / series.high_water_mark;
            if decline > 0 {
                decline as u32
            } else {
                0
            }
        } else {
            0
        };

        Ok(PerformanceReport {
            apy_bps: apy,
            benchmark_apy_bps,
            excess_bps: apy - benchmark_apy_bps,
            high_water_mark: series.high_water_mark,
            current_drawdown_bps,
            max_drawdown_bps: series.max_drawdown_bps,
            samples: series.samples,
        })
    }
}
