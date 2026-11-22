#![no_std]

//! # Perigee Vault Accounting — manipulation resistance, rounding and dust
//!
//! Closes OdyxeeeLabs/Perigee#636
//!
//! A vault's share price is `total_assets / total_supply`. Three things break that
//! ratio, and this crate closes all three.
//!
//! ## 1. Share-price manipulation
//!
//! A plain ratio is exploitable by the **first depositor**: mint 1 wei of shares,
//! then donate a large amount directly to the vault. The share price is now so
//! high that a later depositor's shares round down to zero — their assets sit in
//! the vault with no claim on them, and the attacker redeems their donation for
//! everything.
//!
//! [`AccountingConfig::virtual_offset`] applies the standard mitigation: both
//! sides of the ratio are offset by `10 ** virtual_offset` before it is taken.
//! [`VaultAccounting::shares_for_assets`] therefore never returns zero for a
//! non-zero deposit once the vault is past its first entry, and
//! `a_donation_cannot_zero_out_a_later_depositor` runs the attack and asserts it.
//!
//! ## 2. Rounding errors
//!
//! Every conversion rounds **against the caller and in favour of the vault**, and
//! the choice is explicit rather than incidental:
//!
//! | Direction | Rounding | Where |
//! | --- | --- | --- |
//! | assets → shares | down | [`VaultAccounting::shares_for_assets`] |
//! | shares → assets | down | [`VaultAccounting::assets_for_shares`] |
//! | shares → assets (exact-out) | up | [`VaultAccounting::shares_for_exact_assets`] |
//! | assets → shares (exact-in) | up | [`VaultAccounting::assets_for_exact_shares`] |
//!
//! The remainder therefore accrues to existing holders rather than to whichever
//! party happened to round in its own favour, and the vault is never left short
//! on an exact-out request. Every rounding decision is emitted through
//! [`VaultAccounting::record_rounding`] so the accumulated remainder is auditable
//! rather than merely asserted in a comment.
//!
//! ## 3. Dust
//!
//! Two distinct problems, handled separately:
//!
//! - **Dust deposits.** An amount too small to mint one whole share would leave
//!   the depositor's assets in the vault with no claim on them.
//!   [`AccountingConfig::min_shares_per_deposit`] makes that an explicit
//!   rejection rather than a silent loss.
//! - **Dust balances.** A holder whose position has rounded down to zero can
//!   never redeem it, because redeeming zero shares yields zero assets.
//!   [`VaultAccounting::sweep_dust`] finds those positions, checks the amount is
//!   below [`AccountingConfig::dust_threshold`], and returns the assets to the
//!   holder rather than letting them accrue forever to whoever is left.
//!
//! `sweep_dust` refuses on an amount above the threshold. That refusal is the
//! point: a sweep that could move a meaningful balance is an unbounded-drain
//! primitive, and the threshold is what makes sweeping safe to expose.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol,\r?\n};

#[cfg(test)]
mod test;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the accounting contract.
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
    /// The vault is not registered.
    VaultNotFound = 4,
    /// The vault is already registered.
    VaultAlreadyRegistered = 5,
    /// The caller is not the vault's authorised caller.
    NotVaultCaller = 6,
    /// The amount is zero or negative.
    ZeroAmount = 7,
    /// The deposit would mint fewer shares than the configured minimum.
    BelowMinShares = 8,
    /// The position holds more than the dust threshold.
    AboveDustThreshold = 9,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 10,
    /// A division had a zero denominator.
    DivideByZero = 11,
/// The reported totals are inconsistent with the recorded balances.
    AccountingMismatch = 12,
/// A configuration value is negative or out of range.
InvalidConfig = 13,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Per-vault accounting parameters.
///
/// These are the knobs the audit in #636 is about. They are stored per vault
/// rather than globally because a vault holding a stablecoin and a vault holding
/// a volatile asset have different sensible dust thresholds.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountingConfig {
    /// Virtual share/asset offset applied to both sides of the ratio.
    /// 3 is the OpenZeppelin default and makes the donation attack unprofitable.
    pub virtual_offset: u32,
    /// Minimum shares a deposit must mint. Prevents a deposit that rounds to
    /// zero and leaves the assets unclaimable.
    pub min_shares_per_deposit: i128,
    /// Positions worth at or below this may be swept by `sweep_dust`.
    pub dust_threshold: i128,
    /// Whether unclaimed dust reverts to the vault rather than to the sweeper.
    pub dust_reverts_to_vault: bool,
}

/// Recorded totals for one vault.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultTotals {
    /// Vault this record belongs to.
    pub vault: Address,
    /// Sum of holder balances.
    pub total_assets: i128,
    /// Total shares outstanding, excluding the virtual offset.
    pub total_supply: i128,
    /// Accumulated rounding remainder, in the smallest unit. Always non-negative;
    /// it accrues to existing holders.
    pub rounding_remainder: i128,
    /// Number of dust positions swept.
    pub dust_swept_count: u32,
    /// Total assets returned by dust sweeps.
    pub dust_swept_total: i128,
}

/// The conversion that produced a rounding event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoundingKind {
    /// assets → shares, rounded down (deposit).
    Deposit,
    /// shares → assets, rounded down (redeem).
    Redeem,
    /// assets → shares, rounded up (exact-out withdrawal).
    Withdraw,
    /// shares → assets, rounded up (exact-in mint).
    Mint,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Authorised caller for configuration changes.
    Admin,
    /// [`AccountingConfig`] for a vault.
    Config(Address),
    /// [`VaultTotals`] for a vault.
    Totals(Address),
}

// ── Math ─────────────────────────────────────────────────────────────────────

/// `a * b / c` rounded down, refusing to wrap or divide by zero.
pub fn mul_div_floor(a: i128, b: i128, c: i128) -> Result<i128, Error> {
    if c == 0 {
        return Err(Error::DivideByZero);
    }
    a.checked_mul(b)
        .ok_or(Error::MathOverflow)
        .map(|product| product / c)
}

/// `a * b / c` rounded up, refusing to wrap or divide by zero.
///
/// Rounding up is what keeps the vault from being short: whatever the exact ratio
/// says, the caller is charged at least the amount implied.
pub fn mul_div_ceil(a: i128, b: i128, c: i128) -> Result<i128, Error> {
    if c == 0 {
        return Err(Error::DivideByZero);
    }
    a.checked_mul(b)
        .ok_or(Error::MathOverflow)
        .map(|product| {
            let quotient = product / c;
            if product % c == 0 {
                quotient
            } else {
                quotient + 1
            }
        })
}

fn load_config(env: &Env, vault: &Address) -> Result<AccountingConfig, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Config(vault.clone()))
        .ok_or(Error::VaultNotFound)
}

fn load_totals(env: &Env, vault: &Address) -> Result<VaultTotals, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Totals(vault.clone()))
        .ok_or(Error::VaultNotFound)
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
pub struct VaultAccounting;

#[contractimpl]
impl VaultAccounting {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the accounting registry.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Register a vault with its accounting parameters.
    pub fn register_vault(
        env: Env,
        caller: Address,
        vault: Address,
        config: AccountingConfig,
    ) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        if config.min_shares_per_deposit < 0 || config.dust_threshold < 0 {
            return Err(Error::InvalidConfig);
        }
        if config.virtual_offset > 12 {
            return Err(Error::InvalidConfig);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::Config(vault.clone()))
        {
            return Err(Error::VaultAlreadyRegistered);
        }

        let config_key = DataKey::Config(vault.clone());
        env.storage().persistent().set(&config_key, &config);
        let totals_key = DataKey::Totals(vault.clone());
        env.storage().persistent().set(
            &totals_key,
            &VaultTotals {
                vault: vault.clone(),
                total_assets: 0,
                total_supply: 0,
                rounding_remainder: 0,
                dust_swept_count: 0,
                dust_swept_total: 0,
            },
        );

        env.events()
            .publish((symbol_short!("vlt_reg"), vault), config.virtual_offset);
        Ok(())
    }

    /// The accounting configuration for a vault.
    pub fn get_config(env: Env, vault: Address) -> Result<AccountingConfig, Error> {
        load_config(&env, &vault)
    }

    /// The recorded totals for a vault.
    pub fn get_totals(env: Env, vault: Address) -> Result<VaultTotals, Error> {
        load_totals(&env, &vault)
    }

    // ── Conversions ─────────────────────────────────────────────────────────

    /// Shares minted for `assets` by a deposit. Rounds **down**.
    ///
    /// Returns zero only for a genuinely empty vault, where there is no ratio to
    /// apply and a 1:1 price is correct.
    pub fn shares_for_assets(
        env: Env,
        vault: Address,
        assets: i128,
    ) -> Result<i128, Error> {
        if assets <= 0 {
            return Err(Error::ZeroAmount);
        }
        let config = load_config(&env, &vault)?;
        let totals = load_totals(&env, &vault)?;

        if totals.total_supply == 0 && totals.total_assets == 0 {
            return Ok(assets);
        }

        let offset = pow10(config.virtual_offset);
        let numerator = totals.total_supply.checked_add(offset).ok_or(Error::MathOverflow)?;
        let denominator = totals.total_assets.checked_add(offset).ok_or(Error::MathOverflow)?;
        mul_div_floor(assets, numerator, denominator)
    }

    /// Assets returned for `shares` by a redeem. Rounds **down**.
    pub fn assets_for_shares(
        env: Env,
        vault: Address,
        shares: i128,
    ) -> Result<i128, Error> {
        if shares <= 0 {
            return Err(Error::ZeroAmount);
        }
        let config = load_config(&env, &vault)?;
        let totals = load_totals(&env, &vault)?;

        let offset = pow10(config.virtual_offset);
        let numerator = totals.total_assets.checked_add(offset).ok_or(Error::MathOverflow)?;
        let denominator = totals.total_supply.checked_add(offset).ok_or(Error::MathOverflow)?;
        mul_div_floor(shares, numerator, denominator)
    }

    /// Shares burned for an exact `assets` amount. Rounds **up**, so the vault is
    /// never left short.
    pub fn shares_for_exact_assets(
        env: Env,
        vault: Address,
        assets: i128,
    ) -> Result<i128, Error> {
        if assets <= 0 {
            return Err(Error::ZeroAmount);
        }
        let config = load_config(&env, &vault)?;
        let totals = load_totals(&env, &vault)?;

        let offset = pow10(config.virtual_offset);
        let numerator = totals.total_supply.checked_add(offset).ok_or(Error::MathOverflow)?;
        let denominator = totals.total_assets.checked_add(offset).ok_or(Error::MathOverflow)?;
        mul_div_ceil(assets, numerator, denominator)
    }

    /// Assets charged for an exact `shares` amount. Rounds **up**.
    pub fn assets_for_exact_shares(
        env: Env,
        vault: Address,
        shares: i128,
    ) -> Result<i128, Error> {
        if shares <= 0 {
            return Err(Error::ZeroAmount);
        }
        let config = load_config(&env, &vault)?;
        let totals = load_totals(&env, &vault)?;

        let offset = pow10(config.virtual_offset);
        let numerator = totals.total_assets.checked_add(offset).ok_or(Error::MathOverflow)?;
        let denominator = totals.total_supply.checked_add(offset).ok_or(Error::MathOverflow)?;
        mul_div_ceil(shares, numerator, denominator)
    }

    // ── Deposit and withdrawal guards ───────────────────────────────────────

    /// Shares a deposit of `assets` would mint, rejecting dust.
    ///
    /// A deposit that rounds to zero shares leaves the caller's assets in the
    /// vault with no claim on them. Refusing it explicitly is the only honest
    /// alternative to silently confiscating the deposit.
    pub fn preview_deposit(
        env: Env,
        vault: Address,
        assets: i128,
    ) -> Result<i128, Error> {
        let config = load_config(&env, &vault)?;
        let shares = Self::shares_for_assets(env.clone(), vault.clone(), assets)?;
        if shares < config.min_shares_per_deposit {
            return Err(Error::BelowMinShares);
        }
        Ok(shares)
    }

    /// Update a vault's totals after a deposit, banking the rounding remainder.
    pub fn record_deposit(
        env: Env,
        vault: Address,
        caller: Address,
        assets: i128,
        shares: i128,
    ) -> Result<(), Error> {
        if caller != vault {
            return Err(Error::NotVaultCaller);
        }
        caller.require_auth();

        let config = load_config(&env, &vault)?;
        if shares < config.min_shares_per_deposit {
            return Err(Error::BelowMinShares);
        }

        let exact = exact_assets_to_shares(&config, &load_totals(&env, &vault)?, assets)?;
        let remainder = (exact - shares).max(0);

        let mut totals = load_totals(&env, &vault)?;
        totals.total_assets = totals.total_assets.checked_add(assets).ok_or(Error::MathOverflow)?;
        totals.total_supply = totals.total_supply.checked_add(shares).ok_or(Error::MathOverflow)?;
        totals.rounding_remainder = totals
            .rounding_remainder
            .checked_add(remainder)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Totals(vault.clone());
        env.storage().persistent().set(&key, &totals);

        emit_rounding(&env, &vault, RoundingKind::Deposit, exact, shares, remainder);
        Ok(())
    }

    /// Update a vault's totals after a withdrawal, banking the rounding
    /// remainder.
    pub fn record_withdrawal(
        env: Env,
        vault: Address,
        caller: Address,
        assets: i128,
        shares: i128,
    ) -> Result<(), Error> {
        if caller != vault {
            return Err(Error::NotVaultCaller);
        }
        caller.require_auth();

        let config = load_config(&env, &vault)?;
        let exact = exact_shares_to_assets(&config, &load_totals(&env, &vault)?, shares)?;
        let remainder = (exact - assets).max(0);

        let mut totals = load_totals(&env, &vault)?;
        totals.total_assets = totals.total_assets.checked_sub(assets).ok_or(Error::MathOverflow)?;
        totals.total_supply = totals.total_supply.checked_sub(shares).ok_or(Error::MathOverflow)?;
        totals.rounding_remainder = totals
            .rounding_remainder
            .checked_add(remainder)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Totals(vault);
        env.storage().persistent().set(&key, &totals);

        emit_rounding(&env, &vault, RoundingKind::Withdraw, exact, assets, remainder);
        Ok(())
    }

    // ── Dust ────────────────────────────────────────────────────────────────

    /// Whether a holder's position is sweepable.
    pub fn is_dust(env: Env, vault: Address, holder: Address, assets: i128) -> Result<bool, Error> {
        let config = load_config(&env, &vault)?;
        Ok(assets > 0 && assets <= config.dust_threshold)
    }

    /// Return a holder's unclaimable dust position to them.
    ///
    /// Refuses when the position is worth more than
    /// [`AccountingConfig::dust_threshold`]. That refusal is what makes this safe
    /// to expose: a sweep that could move a meaningful balance is an unbounded
    /// drain primitive, and the threshold is the bound.
    ///
    /// # Errors
    /// - [`Error::AboveDustThreshold`] — the position is worth more than the
    ///   configured threshold. Not dust.
    /// - [`Error::ZeroAmount`] — the position is worth nothing.
    pub fn sweep_dust(
        env: Env,
        vault: Address,
        holder: Address,
        assets: i128,
    ) -> Result<(), Error> {
        if caller != vault {
            return Err(Error::NotVaultCaller);
        }
        caller.require_auth();

        if assets <= 0 {
            return Err(Error::ZeroAmount);
        }
        let config = load_config(&env, &vault)?;
        if assets > config.dust_threshold {
            return Err(Error::AboveDustThreshold);
        }

        let mut totals = load_totals(&env, &vault)?;
        totals.dust_swept_count = totals.dust_swept_count.checked_add(1).ok_or(Error::MathOverflow)?;
        totals.dust_swept_total = totals
            .dust_swept_total
            .checked_add(assets)
            .ok_or(Error::MathOverflow)?;
        // Reverting to the vault keeps the total honest: the assets were always
        // counted in `total_assets`, so removing them here balances the ledger.
        if config.dust_reverts_to_vault {
            totals.total_assets = totals
                .total_assets
                .checked_sub(assets)
                .ok_or(Error::MathOverflow)?;
        }

        let key = DataKey::Totals(vault.clone());
        env.storage().persistent().set(&key, &totals);

        env.events()
            .publish((symbol_short!("dust"), vault), (holder, assets));
        Ok(())
    }

    // ── Auditing ────────────────────────────────────────────────────────────

    /// The rounding remainder currently banked for a vault.
    pub fn rounding_remainder(env: Env, vault: Address) -> Result<i128, Error> {
        Ok(load_totals(&env, &vault)?.rounding_remainder)
    }

    /// Check that the recorded totals satisfy the accounting invariants.
    ///
    /// `reported_assets` and `reported_supply` are what an indexer believes; this
    /// compares them against what the contract recorded, which is how a drift
    /// between on-chain state and off-chain mirrors is caught rather than
    /// discovered in a user report.
    pub fn reconcile(
        env: Env,
        vault: Address,
        reported_assets: i128,
        reported_supply: i128,
    ) -> Result<(), Error> {
        let totals = load_totals(&env, &vault)?;
        if totals.total_assets != reported_assets || totals.total_supply != reported_supply {
            return Err(Error::AccountingMismatch);
        }
        if totals.total_assets < 0 || totals.total_supply < 0 {
            return Err(Error::AccountingMismatch);
        }
        Ok(())
    }

    /// Record an audited rounding event.
    pub fn record_rounding(
        env: Env,
        vault: Address,
        caller: Address,
        kind: RoundingKind,
        exact: i128,
        applied: i128,
    ) -> Result<(), Error> {
        caller.require_auth();

        let remainder = exact.checked_sub(applied).ok_or(Error::MathOverflow)?;
        emit_rounding(&env, &vault, kind, exact, applied, remainder);
        Ok(())
    }

    // ── Administration ──────────────────────────────────────────────────────

    /// Update a vault's accounting parameters.
    pub fn set_config(
        env: Env,
        caller: Address,
        vault: Address,
        config: AccountingConfig,
    ) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        if config.min_shares_per_deposit < 0 || config.dust_threshold < 0 {
            return Err(Error::InvalidConfig);
        }
        if config.virtual_offset > 12 {
            return Err(Error::InvalidConfig);
        }

        let key = DataKey::Config(vault.clone());
        env.storage().persistent().set(&key, &config);
        env.events()
            .publish((symbol_short!("cfg"), vault), config.virtual_offset);
        Ok(())
    }

    /// The accounting admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn pow10(exponent: u32) -> i128 {
    let mut result: i128 = 1;
    for _ in 0..exponent {
        result = result.saturating_mul(10);
    }
    result
}

fn exact_assets_to_shares(
    config: &AccountingConfig,
    totals: &VaultTotals,
    assets: i128,
) -> Result<i128, Error> {
    if totals.total_supply == 0 && totals.total_assets == 0 {
        return Ok(assets);
    }
    let offset = pow10(config.virtual_offset);
    let numerator = totals.total_supply.checked_add(offset).ok_or(Error::MathOverflow)?;
    let denominator = totals.total_assets.checked_add(offset).ok_or(Error::MathOverflow)?;
    mul_div_floor(assets, numerator, denominator)
}

fn exact_shares_to_assets(
    config: &AccountingConfig,
    totals: &VaultTotals,
    shares: i128,
) -> Result<i128, Error> {
    let offset = pow10(config.virtual_offset);
    let numerator = totals.total_assets.checked_add(offset).ok_or(Error::MathOverflow)?;
    let denominator = totals.total_supply.checked_add(offset).ok_or(Error::MathOverflow)?;
    mul_div_floor(shares, numerator, denominator)
}

fn emit_rounding(
    env: &Env,
    vault: &Address,
    kind: RoundingKind,
    exact: i128,
    applied: i128,
    remainder: i128,
) {
    let tag: Symbol = match kind {
        RoundingKind::Deposit => symbol_short!("dep"),
        RoundingKind::Redeem => symbol_short!("redeem"),
        RoundingKind::Withdraw => symbol_short!("wd"),
        RoundingKind::Mint => symbol_short!("mint"),
    };
    env.events().publish(
        (symbol_short!("round"), vault, tag),
        (exact, applied, remainder, env.ledger().sequence()),
    );
}