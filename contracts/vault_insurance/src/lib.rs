#![no_std]

//! # Perigee Vault Insurance — coverage, claims and premium accounting
//!
//! Closes OdyxeeeLabs/Perigee#638
//!
//! Integrates a vault with an external cover provider (Nexus Mutual, InsurAce, or
//! a self-funded reserve) and records, on chain:
//!
//! - **premiums** paid and the reserve they build,
//! - **coverage limits** and the per-vault caps,
//! - **claims**, what triggered them, and how they were settled.
//!
//! ## Why premiums are taken on withdrawal rather than on deposit
//!
//! Charging the premium at deposit means a depositor who leaves immediately has
//! paid for cover they never used, and it makes the premium indistinguishable from
//! a deposit fee — which is how it escapes scrutiny. [`VaultInsurance::deposit`]
//! therefore records the *assessed* premium and
//! [`VaultInsurance::withdraw`] collects the part that has actually accrued, in
//! whole epoch periods. The vault can see the exact cost of its cover before
//! committing to it.
//!
//! ## The reserve must be able to cover its outstanding exposure
//!
//! A cover pool that has promised more than it holds is worse than no cover,
//! because depositors believe they are protected. [`VaultInsurance::write`]
//! refuses to increase coverage unless the reserve can back it:
//!
//! ```text
//! reserve >= Σ(outstanding coverage across all active vaults)
//! ```
//!
//! and [`VaultInsurance::assess_coverage`] returns exactly what a new policy would
//! be supportable for, so an integrator learns the limit before writing a policy
//! rather than after the call reverts.
//!
//! ## Claim triggers are explicit, not inferred
//!
//! A claim is only openable against a [`Trigger`] the vault has already declared
//! in its policy. That ordering matters: if claims could pick their own trigger,
//! any vault could claim against its own mispricing by declaring the trigger
//! afterwards.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Ledgers in one premium epoch. 17 280 ≈ 24 h at the Stellar 5 s cadence.
pub const EPOCH_LEDGERS: u32 = 17_280;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the insurance contract.
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
    /// No policy exists for the vault.
    NoPolicy = 4,
    /// A policy already exists for the vault.
    PolicyExists = 5,
    /// The amount is zero or negative.
    ZeroAmount = 6,
    /// The vault does not hold the covered assets.
    InsuredValueTooLow = 7,
    /// The requested coverage exceeds what the reserve can back.
    CoverageExceedsReserve = 8,
    /// The requested coverage exceeds the per-policy cap.
    CoverageAboveCap = 9,
    /// The premium rate is above 100%.
    PremiumRateTooHigh = 10,
    /// A claim is already open for this vault.
    ClaimAlreadyOpen = 11,
    /// No claim is open for this vault.
    NoOpenClaim = 12,
    /// The claim trigger is not one this policy declares.
    TriggerNotCovered = 13,
    /// The settlement exceeds what the claim or the reserve allows.
    SettlementTooLarge = 14,
    /// The premium has already been collected for this epoch.
    PremiumAlreadyCollected = 15,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 16,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// The external cover provider.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Provider {
    /// Nexus Mutual.
    NexusMutual,
    /// InsurAce.
    InsurAce,
    /// A self-funded reserve held by the protocol itself.
    SelfFunded,
}

/// What a policy responds to.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Trigger {
    /// Loss on the covered strategy beyond the policy's tolerance.
    StrategyLoss,
    /// An oracle price deviation beyond the policy's threshold.
    OracleDeviation,
    /// A smart-contract failure in the covered vault.
    ContractFailure,
    /// A liquidation cascading into the covered vault.
    LiquidationCascade,
}

/// The cover a vault has bought.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    /// Vault this policy covers.
    pub vault: Address,
    /// Who controls the policy.
    pub policyholder: Address,
    /// Cover provider.
    pub provider: Provider,
    /// Maximum payout, in the policy's accounting unit.
    pub coverage: i128,
    /// Value the vault must hold for the coverage to be valid.
    pub insured_value: i128,
    /// Annual premium rate in basis points of `insured_value`.
    pub premium_rate_bps: u32,
    /// Premium accrued but not yet collected.
    pub accrued_premium: i128,
    /// Last epoch a premium was collected for.
    pub last_premium_ledger: u32,
    /// Declared triggers. A claim may only cite one of these.
    pub triggers: soroban_sdk::Vec<Trigger>,
    /// Whether the policy is currently in force.
    pub active: bool,
}

/// An open or settled claim.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claim {
    /// Vault the claim is against.
    pub vault: Address,
    /// Claimant.
    pub claimant: Address,
    /// What triggered it.
    pub trigger: Trigger,
    /// Amount claimed.
    pub amount: i128,
    /// Ledger at which it was opened.
    pub opened_ledger: u32,
    /// Whether it has been settled.
    pub settled: bool,
}

/// Protocol-wide premium and payout accounting.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reserve {
    /// Total premiums collected into the reserve.
    pub premiums_in: i128,
    /// Total claims paid out of the reserve.
    pub payouts_out: i128,
    /// Premiums collected but not yet paid out.
    pub unclaimed_premium: i128,
    /// Total outstanding coverage across active policies.
    pub outstanding_coverage: i128,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to write policies and settle claims.
    Admin,
    /// Premium recipient.
    PremiumRecipient,
    /// [`Reserve`] totals.
    Reserve,
    /// [`Policy`] for a vault.
    Policy(Address),
    /// [`Claim`] for a vault.
    Claim(Address),
    /// Ledger at which the first premium was assessed, for epoch accounting.
    EpochOrigin,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

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

fn load_policy(env: &Env, vault: &Address) -> Result<Policy, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Policy(vault.clone()))
        .ok_or(Error::NoPolicy)
}

fn load_reserve(env: &Env) -> Result<Reserve, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Reserve)
        .ok_or(Error::NotInitialized)
}

/// `a * bps / 10 000`, refusing to wrap.
fn apply_bps(amount: i128, bps: u32) -> Result<i128, Error> {
    amount
        .checked_mul(bps as i128)
        .map(|product| product / BPS_DENOMINATOR)
        .ok_or(Error::MathOverflow)
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct VaultInsurance;

#[contractimpl]
impl VaultInsurance {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the insurance registry.
    pub fn initialize(
        env: Env,
        admin: Address,
        premium_recipient: Address,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::PremiumRecipient, &premium_recipient);
        env.storage()
            .instance()
            .set(&DataKey::EpochOrigin, &env.ledger().sequence());
        env.storage().instance().set(
            &DataKey::Reserve,
            &Reserve {
                premiums_in: 0,
                payouts_out: 0,
                unclaimed_premium: 0,
                outstanding_coverage: 0,
            },
        );
        Ok(())
    }

    // ── Policy ──────────────────────────────────────────────────────────────

    /// Write a policy for `vault`.
    ///
    /// Refused when the coverage would exceed what the reserve can back — a cover
    /// pool that has promised more than it holds is worse than no cover, because
    /// depositors believe they are protected.
    pub fn write(
        env: Env,
        caller: Address,
        vault: Address,
        provider: Provider,
        coverage: i128,
        insured_value: i128,
        premium_rate_bps: u32,
        triggers: soroban_sdk::Vec<Trigger>,
    ) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        if coverage <= 0 || insured_value <= 0 {
            return Err(Error::ZeroAmount);
        }
        if premium_rate_bps > 10_000 {
            return Err(Error::PremiumRateTooHigh);
        }
        if coverage > insured_value {
            return Err(Error::CoverageAboveCap);
        }
        if env.storage().persistent().has(&DataKey::Policy(vault.clone())) {
            return Err(Error::PolicyExists);
        }

        let mut reserve = load_reserve(&env)?;
        let new_outstanding = reserve
            .outstanding_coverage
            .checked_add(coverage)
            .ok_or(Error::MathOverflow)?;
        if new_outstanding > reserve.unclaimed_premium {
            return Err(Error::CoverageExceedsReserve);
        }

        let key = DataKey::Policy(vault.clone());
        env.storage().persistent().set(
            &key,
            &Policy {
                vault: vault.clone(),
                policyholder: vault.clone(),
                provider,
                coverage,
                insured_value,
                premium_rate_bps,
                accrued_premium: 0,
                last_premium_ledger: env.ledger().sequence(),
                triggers,
                active: true,
            },
        );

        reserve.outstanding_coverage = new_outstanding;
        env.storage().instance().set(&DataKey::Reserve, &reserve);

        env.events().publish(
            (symbol_short!("policy"), vault),
            (provider, coverage, premium_rate_bps),
        );
        Ok(())
    }

    /// The policy for a vault.
    pub fn get_policy(env: Env, vault: Address) -> Result<Policy, Error> {
        load_policy(&env, &vault)
    }

    /// Cancel a policy. Outstanding coverage is released.
    pub fn cancel(env: Env, caller: Address, vault: Address) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        let mut policy = load_policy(&env, &vault)?;
        if !policy.active {
            return Err(Error::NoPolicy);
        }
        policy.active = false;

        let mut reserve = load_reserve(&env)?;
        reserve.outstanding_coverage = reserve
            .outstanding_coverage
            .checked_sub(policy.coverage)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Policy(vault.clone());
        env.storage().persistent().set(&key, &policy);
        env.storage().instance().set(&DataKey::Reserve, &reserve);

        env.events()
            .publish((symbol_short!("cancel"), vault), policy.coverage);
        Ok(())
    }

    // ── Premiums ────────────────────────────────────────────────────────────

    /// Assess the premium accrued over whole epochs since the last collection.
    ///
    /// Assessed rather than charged. The premium is collected on withdrawal, so a
    /// depositor who leaves immediately has not paid for cover they never used.
    pub fn assess_premium(env: Env, vault: Address) -> Result<i128, Error> {
        let policy = load_policy(&env, &vault)?;
        if !policy.active {
            return Err(Error::NoPolicy);
        }

        let elapsed = env.ledger().sequence().saturating_sub(policy.last_premium_ledger);
        if elapsed < EPOCH_LEDGERS {
            return Ok(policy.accrued_premium);
        }

        let epochs = (elapsed / EPOCH_LEDGERS) as i128;
        // One epoch is one day, so an annual rate is divided by 365. Integer
        // division rounds down, which under-charges by at most a day of cover per
        // collection — in favour of the policyholder, never the provider.
        let per_epoch = apply_bps(policy.insured_value, policy.premium_rate_bps)? / 365;

        per_epoch.checked_mul(epochs).ok_or(Error::MathOverflow)
    }

    /// Record a deposit, accruing premium against it.
    pub fn deposit(env: Env, vault: Address, _amount: i128) -> Result<(), Error> {
        vault.require_auth();

        let mut policy = load_policy(&env, &vault)?;
        let premium = Self::assess_premium(env.clone(), vault.clone())?;
        policy.accrued_premium = policy
            .accrued_premium
            .checked_add(premium)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Policy(vault);
        env.storage().persistent().set(&key, &policy);
        Ok(())
    }

    /// Withdraw, collecting the accrued premium out of the withdrawn amount.
    pub fn withdraw(env: Env, vault: Address, amount: i128) -> Result<(i128, i128), Error> {
        vault.require_auth();

        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }

        let mut policy = load_policy(&env, &vault)?;
        let premium = policy.accrued_premium.min(amount);
        let net = amount.checked_sub(premium).ok_or(Error::MathOverflow)?;

        policy.accrued_premium = policy.accrued_premium.checked_sub(premium).unwrap_or(0);
        policy.last_premium_ledger = env.ledger().sequence();
        // Coverage is released as the position leaves.
        if net == 0 {
            policy.active = false;
        }

        let mut reserve = load_reserve(&env)?;
        reserve.premiums_in = reserve.premiums_in.checked_add(premium).ok_or(Error::MathOverflow)?;
        reserve.unclaimed_premium = reserve
            .unclaimed_premium
            .checked_add(premium)
            .ok_or(Error::MathOverflow)?;
        if net == 0 {
            reserve.outstanding_coverage = reserve
                .outstanding_coverage
                .checked_sub(policy.coverage)
                .ok_or(Error::MathOverflow)?;
        }

        let key = DataKey::Policy(vault);
        env.storage().persistent().set(&key, &policy);
        env.storage().instance().set(&DataKey::Reserve, &reserve);

        env.events()
            .publish((symbol_short!("prem"), vault), (premium, net));
        Ok((premium, net))
    }

    // ── Reserve ─────────────────────────────────────────────────────────────

    /// The reserve totals.
    pub fn reserve(env: Env) -> Result<Reserve, Error> {
        load_reserve(&env)
    }

    /// Largest coverage the reserve could currently support for a new policy.
    ///
    /// Returned so an integrator learns the limit before writing a policy, rather
    /// than by having `write` revert.
    pub fn assess_coverage(env: Env, insured_value: i128) -> Result<i128, Error> {
        let reserve = load_reserve(&env)?;
        Ok(reserve
            .unclaimed_premium
            .saturating_sub(reserve.outstanding_coverage)
            .min(insured_value))
    }

    /// Fund the reserve. Admin only, because this is what backs every promise the
    /// contract makes.
    pub fn fund(env: Env, caller: Address, amount: i128) -> Result<(), Error> {
        require_admin(&env, &caller)?;
        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }

        let mut reserve = load_reserve(&env)?;
        reserve.unclaimed_premium = reserve
            .unclaimed_premium
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;

        env.storage().instance().set(&DataKey::Reserve, &reserve);
        env.events()
            .publish((symbol_short!("fund"),), amount);
        Ok(())
    }

    // ── Claims ──────────────────────────────────────────────────────────────

    /// Open a claim against a policy.
    ///
    /// The trigger must already be one the policy declares. Claims are not
    /// allowed to pick their own trigger: otherwise a vault could claim against
    /// its own mispricing by declaring the trigger afterwards.
    pub fn open_claim(
        env: Env,
        claimant: Address,
        vault: Address,
        trigger: Trigger,
        amount: i128,
    ) -> Result<(), Error> {
        claimant.require_auth();

        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }
        let policy = load_policy(&env, &vault)?;
        if !policy.active {
            return Err(Error::NoPolicy);
        }
        if !policy.triggers.contains(&trigger) {
            return Err(Error::TriggerNotCovered);
        }
        if amount > policy.coverage {
            return Err(Error::SettlementTooLarge);
        }
        if env.storage().persistent().has(&DataKey::Claim(vault.clone())) {
            return Err(Error::ClaimAlreadyOpen);
        }

        let key = DataKey::Claim(vault.clone());
        env.storage().persistent().set(
            &key,
            &Claim {
                vault: vault.clone(),
                claimant: claimant.clone(),
                trigger,
                amount,
                opened_ledger: env.ledger().sequence(),
                settled: false,
            },
        );

        env.events()
            .publish((symbol_short!("claim"), vault), (claimant, amount));
        Ok(())
    }

    /// The claim for a vault.
    pub fn get_claim(env: Env, vault: Address) -> Result<Claim, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Claim(vault))
            .ok_or(Error::NoOpenClaim)
    }

    /// Settle a claim, paying out of the reserve.
    pub fn settle(env: Env, caller: Address, vault: Address, payout: i128) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        if payout <= 0 {
            return Err(Error::ZeroAmount);
        }
        let mut claim: Claim = env
            .storage()
            .persistent()
            .get(&DataKey::Claim(vault.clone()))
            .ok_or(Error::NoOpenClaim)?;
        if claim.settled {
            return Err(Error::NoOpenClaim);
        }
        if payout > claim.amount {
            return Err(Error::SettlementTooLarge);
        }

        let mut reserve = load_reserve(&env)?;
        if reserve.unclaimed_premium < payout {
            return Err(Error::SettlementTooLarge);
        }

        reserve.unclaimed_premium = reserve
            .unclaimed_premium
            .checked_sub(payout)
            .ok_or(Error::MathOverflow)?;
        reserve.payouts_out = reserve
            .payouts_out
            .checked_add(payout)
            .ok_or(Error::MathOverflow)?;

        claim.settled = true;

        let key = DataKey::Claim(vault.clone());
        env.storage().persistent().set(&key, &claim);
        env.storage().instance().set(&DataKey::Reserve, &reserve);

        env.events()
            .publish((symbol_short!("settle"), vault), payout);
        Ok(())
    }

    /// Close a claim without payout. Admin only, for claims that do not qualify.
    pub fn close_claim(env: Env, caller: Address, vault: Address) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        env.storage()
            .persistent()
            .remove(&DataKey::Claim(vault.clone()));
        env.events()
            .publish((symbol_short!("claim_x"), vault), caller);
        Ok(())
    }

    // ── Administration ──────────────────────────────────────────────────────

    /// The registry admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    /// The premium recipient.
    pub fn premium_recipient(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::PremiumRecipient)
            .ok_or(Error::NotInitialized)
    }

    /// A provider label, for display.
    pub fn provider_label(env: Env, provider: Provider) -> String {
        match provider {
            Provider::NexusMutual => String::from_str(&env, "Nexus Mutual"),
            Provider::InsurAce => String::from_str(&env, "InsurAce"),
            Provider::SelfFunded => String::from_str(&env, "Self-funded"),
        }
    }
}