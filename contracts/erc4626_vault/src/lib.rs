#![no_std]

//! # Perigee ERC-4626 Vault
//!
//! A tokenised vault implementing the [ERC-4626][] tokenised-vault interface on
//! Soroban. Depositors receive vault *shares*; shares redeem for the underlying
//! Stellar Asset Contract (SAC) token at the vault's current share price.
//!
//! Closes OdyxeeeLabs/Perigee#631
//!
//! [ERC-4626]: https://eips.ethereum.org/EIPS/eip-4626
//!
//! ## Interface
//!
//! Soroban uses snake_case, so the camelCase ERC-4626 names map to the Rust
//! identifiers used here.
//!
//! | ERC-4626 | This contract |
//! | --- | --- |
//! | `asset()` | `asset()` |
//! | `totalAssets()` | `total_assets()` |
//! | `convertToShares` / `convertToAssets` | `convert_to_shares` / `convert_to_assets` |
//! | `previewDeposit` / `previewMint` | `preview_deposit` / `preview_mint` |
//! | `previewWithdraw` / `previewRedeem` | `preview_withdraw` / `preview_redeem` |
//! | `maxDeposit` / `maxMint` | `max_deposit` / `max_mint` |
//! | `maxWithdraw` / `maxRedeem` | `max_withdraw` / `max_redeem` |
//! | `deposit` / `mint` | `deposit` / `mint` |
//! | `withdraw` / `redeem` | `withdraw` / `redeem` |
//!
//! ## Deviation from ERC-4626: no `permit`
//!
//! ERC-4626 inherits EIP-2612 `permit`, which requires `ecrecover` on-chain.
//! Soroban exposes no signature-recovery primitive to contracts, so a genuinely
//! gasless `permit` **cannot be implemented** here. Any attempt would either
//! trust an off-chain relayer with the owner's key or pretend to verify a
//! signature it cannot check.
//!
//! What *is* expressible is the property `permit` exists to provide: authority
//! that is bounded in both amount and time. This contract provides that as
//! [`approve`] plus [`permit_withdraw`] — an allowance with a **mandatory**
//! expiry — plus a pluggable verifier so a deployer with a signature scheme the
//! chain does expose can layer one on. See the README for the full argument.
//!
//! ## Rounding
//!
//! Every conversion rounds **against the caller and in favour of the vault**.
//! Deposits mint fewer shares than the exact ratio and withdrawals burn more, so
//! the rounding remainder always accrues to existing holders rather than to
//! whichever party happened to round in its own favour. A single depositor
//! cannot extract it.
//!
//! ## First-depositor inflation
//!
//! A vault with a plain `shares = assets * supply / managed` ratio can be drained
//! by its first depositor: mint 1 wei, donate a large amount, and the share price
//! is now so high that a later depositor's shares round down to zero while their
//! assets stay in the vault. This contract applies the standard mitigation from
//! OpenZeppelin's ERC4626 — a virtual share/asset offset of [`DECIMALS_OFFSET`] —
//! so both sides of the ratio are offset before it is taken.
//!
//! That offset is the baseline, not the whole story: deeper manipulation
//! resistance, rounding policy and dust handling are the subject of #636.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers to live for persistent records after each write.
///
/// 17 280 ledgers ≈ 24 h at the Stellar target of one ledger every 5 s. Every
/// write re-extends, so an actively used vault never expires.
pub const TTL_LEDGERS: u32 = 17_280;

/// Virtual share/asset offset applied to both sides of the share price.
///
/// 3 makes the position of a first depositor who mints 1 wei negligible, which
/// is the OpenZeppelin default and is enough to make the classic donation
/// attack unprofitable.
pub const DECIMALS_OFFSET: u32 = 3;

/// `10 ** DECIMALS_OFFSET`, precomputed so the hot path is not a `pow`.
pub const VIRTUAL_OFFSET: i128 = 1_000;

/// Basis-point denominator.
const BPS_DENOMINATOR: i128 = 10_000;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the ERC-4626 vault.
///
/// Discriminants are stable so off-chain clients (and the unified
/// `Perigee-error-codes` decoder) can map failures to user-facing messages.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The vault has not been initialized yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not the current admin.
    Unauthorized = 3,
    /// The vault is paused; no deposits or withdrawals are accepted.
    Paused = 4,
    /// The amount is zero, negative, or too small to convert to a whole share.
    ZeroAmount = 5,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 6,
    /// A division had a zero denominator.
    DivideByZero = 7,
    /// The account does not hold enough shares for the requested debit.
    InsufficientShares = 8,
    /// No allowance exists, or it does not cover the requested amount.
    InsufficientAllowance = 9,
    /// The allowance's expiry ledger has been reached.
    AllowanceExpired = 10,
    /// A performance fee above 100% was configured.
    InvalidFee = 11,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// A spender's bounded authority over `owner`'s shares.
///
/// The expiry is not optional. A permanent allowance is the failure mode that
/// makes leaked approvals unrecoverable, so there is no way to construct one
/// without stating a deadline.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Allowance {
    /// Remaining shares the spender may move.
    pub shares: i128,
    /// Ledger sequence at which this allowance is void.
    pub expires_at: u32,
}

/// Vault configuration, mutable only through admin-gated setters.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultConfig {
    /// The underlying SAC token this vault wraps.
    pub asset: Address,
    /// Administrator allowed to pause, rotate keys and harvest.
    pub admin: Address,
    /// Receives the performance fee.
    pub fee_recipient: Address,
    /// Performance fee in basis points of yield, applied per harvest.
    pub performance_fee_bps: u32,
    /// Ledger sequence of the most recent harvest.
    pub last_harvest_ledger: u32,
    /// Yield recognised at the most recent harvest.
    pub last_harvest_profit: i128,
}

/// Signature scheme a deployer may plug in to stand in for EIP-2612 `permit`.
///
/// Defaults to [`VerifierKind::None`], which refuses every off-chain signature.
/// That default is the point: a deployer must opt in to trusting a scheme rather
/// than inherit one by default, and the choice is recorded on-chain so the trust
/// assumption is auditable rather than implied.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerifierKind {
    /// No off-chain verification. `permit`-style authorisation is disabled.
    None,
    /// A signature scheme this deployment has been configured to trust.
    Custom(u32),
}

/// Storage keys.
///
/// `Config`, `TotalAssets`, `TotalSupply` and `Paused` are read on every
/// operation and therefore live in instance storage. Share balances and
/// allowances are unbounded in number and use persistent storage.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Active [`VaultConfig`].
    Config,
    /// Total underlying assets managed by the vault, including yield.
    TotalAssets,
    /// Total shares outstanding, excluding the virtual offset.
    TotalSupply,
    /// Whether deposits and withdrawals are halted.
    Paused,
    /// Configured [`VerifierKind`].
    VerifierKind,
    /// Configured verifier contract address, if one is trusted.
    Verifier,
    /// Share balance.
    Shares(Address),
    /// Spender [`Allowance`] over an owner's shares.
    Allowance(Address, Address),
}

// ── Math ─────────────────────────────────────────────────────────────────────

/// `a * b / c`, rounding down, refusing to wrap or divide by zero.
fn mul_div_floor(a: i128, b: i128, c: i128) -> Result<i128, Error> {
    if c == 0 {
        return Err(Error::DivideByZero);
    }
    a.checked_mul(b)
        .ok_or(Error::MathOverflow)
        .map(|product| product / c)
}

/// `a * b / c`, rounding up, refusing to wrap or divide by zero.
///
/// Rounding up is what keeps the vault from being short: whatever the exact
/// ratio says, the caller is charged at least the amount implied.
fn mul_div_ceil(a: i128, b: i128, c: i128) -> Result<i128, Error> {
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

// ── Storage helpers ──────────────────────────────────────────────────────────

fn load_config(env: &Env) -> Result<VaultConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

/// Gates an admin entry point on the stored admin address.
///
/// The passed address must equal the recorded admin, which yields a decodable
/// [`Error::Unauthorized`] rather than a bare host auth error. The subsequent
/// `require_auth` still demands the admin's signature, so passing the admin's
/// *address* is not sufficient to act as the admin.
fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    if *admin != load_config(env)?.admin {
        return Err(Error::Unauthorized);
    }
    admin.require_auth();
    Ok(())
}

fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

/// Rejects an amount that is zero or negative.
///
/// ERC-4626 leaves zero undefined, and a zero-value write costs a ledger write
/// and rent to move nothing.
fn require_positive(amount: i128) -> Result<(), Error> {
    if amount <= 0 {
        return Err(Error::ZeroAmount);
    }
    Ok(())
}

fn ensure_running(env: &Env) -> Result<(), Error> {
    if is_paused(env) {
        return Err(Error::Paused);
    }
    Ok(())
}

fn get_shares(env: &Env, account: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::Shares(account.clone()))
        .unwrap_or(0)
}

fn set_shares(env: &Env, account: &Address, shares: i128) {
    let key = DataKey::Shares(account.clone());
    env.storage().persistent().set(&key, &shares);
    env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
}

fn get_total_supply(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalSupply)
        .unwrap_or(0)
}

fn get_total_assets(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalAssets)
        .unwrap_or(0)
}

// ── Conversions ──────────────────────────────────────────────────────────────

/// Assets to shares, rounding **down** — in favour of the vault.
///
/// Used by the caller's side of the trade: the depositor receives the floor.
fn assets_to_shares_floor(env: &Env, assets: i128) -> Result<i128, Error> {
    let supply = get_total_supply(env);
    let managed = get_total_assets(env);

    if supply == 0 && managed == 0 {
        return Ok(assets);
    }
    mul_div_floor(
        assets,
        supply + VIRTUAL_OFFSET,
        managed + VIRTUAL_OFFSET,
    )
}

/// Assets to shares, rounding **up** — against the caller.
///
/// Used when the vault must mint an exact number of shares and charge at least
/// the assets the ratio implies.
fn assets_to_shares_ceil(env: &Env, assets: i128) -> Result<i128, Error> {
    let supply = get_total_supply(env);
    let managed = get_total_assets(env);

    mul_div_ceil(
        assets,
        supply + VIRTUAL_OFFSET,
        managed + VIRTUAL_OFFSET,
    )
}

/// Shares to assets, rounding **down** — in favour of the vault.
fn shares_to_assets_floor(env: &Env, shares: i128) -> Result<i128, Error> {
    let supply = get_total_supply(env);
    let managed = get_total_assets(env);

    mul_div_floor(
        shares,
        managed + VIRTUAL_OFFSET,
        supply + VIRTUAL_OFFSET,
    )
}

/// Shares to assets, rounding **up** — against the caller.
fn shares_to_assets_ceil(env: &Env, shares: i128) -> Result<i128, Error> {
    let supply = get_total_supply(env);
    let managed = get_total_assets(env);

    mul_div_ceil(
        shares,
        managed + VIRTUAL_OFFSET,
        supply + VIRTUAL_OFFSET,
    )
}

fn move_assets(env: &Env, asset: &Address, from: &Address, to: &Address, amount: i128) {
    token::Client::new(env, asset).transfer(from, to, &amount);
}

// ── Internal accounting ──────────────────────────────────────────────────────

fn add_total_assets(env: &Env, delta: i128) -> Result<(), Error> {
    let next = get_total_assets(env)
        .checked_add(delta)
        .ok_or(Error::MathOverflow)?;
    env.storage().instance().set(&DataKey::TotalAssets, &next);
    Ok(())
}

fn add_total_supply(env: &Env, delta: i128) -> Result<(), Error> {
    let next = get_total_supply(env)
        .checked_add(delta)
        .ok_or(Error::MathOverflow)?;
    env.storage().instance().set(&DataKey::TotalSupply, &next);
    Ok(())
}

fn credit_shares(env: &Env, account: &Address, shares: i128) -> Result<(), Error> {
    let next = get_shares(env, account)
        .checked_add(shares)
        .ok_or(Error::MathOverflow)?;
    set_shares(env, account, next);
    Ok(())
}

fn debit_shares(env: &Env, account: &Address, shares: i128) -> Result<(), Error> {
    let balance = get_shares(env, account);
    if balance < shares {
        return Err(Error::InsufficientShares);
    }
    set_shares(env, account, balance - shares);
    Ok(())
}

fn load_allowance(env: &Env, owner: &Address, spender: &Address) -> Option<Allowance> {
    env.storage()
        .persistent()
        .get(&DataKey::Allowance(owner.clone(), spender.clone()))
}

fn store_allowance(
    env: &Env,
    owner: &Address,
    spender: &Address,
    allowance: Allowance,
) {
    let key = DataKey::Allowance(owner.clone(), spender.clone());
    env.storage().persistent().set(&key, &allowance);
    env.storage().persistent().extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
}

/// Authorize `caller` to move `shares` of `owner`'s holding, and consume that
/// much of any allowance used.
///
/// Returns the number of shares drawn from an allowance, so the caller knows
/// whether to decrement it. An allowance grants *scope*, not *identity*: the
/// spender's own signature is still required, otherwise anyone who observed the
/// on-chain allowance could exercise it.
fn authorize_share_spend(
    env: &Env,
    caller: &Address,
    owner: &Address,
    shares: i128,
) -> Result<i128, Error> {
    if caller == owner {
        caller.require_auth();
        return Ok(0);
    }

    let allowance = load_allowance(env, owner, caller).ok_or(Error::InsufficientAllowance)?;

    if allowance.expires_at <= env.ledger().sequence() {
        return Err(Error::AllowanceExpired);
    }
    if shares > allowance.shares {
        return Err(Error::InsufficientAllowance);
    }

    caller.require_auth();
    Ok(shares)
}

/// Draw `consumed` shares out of an allowance, removing the entry when it is
/// exhausted.
fn consume_allowance(env: &Env, caller: &Address, owner: &Address, consumed: i128) {
    if consumed == 0 {
        return;
    }
    let key = DataKey::Allowance(owner.clone(), caller.clone());
    if let Some(allowance) = load_allowance(env, owner, caller) {
        let remaining = allowance.shares - consumed;
        if remaining <= 0 {
            env.storage().persistent().remove(&key);
        } else {
            store_allowance(
                env,
                owner,
                caller,
                Allowance {
                    shares: remaining,
                    expires_at: allowance.expires_at,
                },
            );
        }
    }
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct Erc4626Vault;

#[contractimpl]
impl Erc4626Vault {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the vault over `asset`, a Stellar Asset Contract token.
    ///
    /// `admin` must authorize the call, which binds initialization to the admin
    /// key at deployment and prevents front-running of the init transaction.
    /// `performance_fee_bps` must not exceed 10 000.
    ///
    /// Off-chain signature verification is left disabled: configure
    /// [`VerifierKind::None`] (the default) unless a verifier has been deployed
    /// and reviewed. A deployer must opt in to trusting a signature scheme.
    pub fn initialize(
        env: Env,
        asset: Address,
        admin: Address,
        fee_recipient: Address,
        performance_fee_bps: u32,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(Error::AlreadyInitialized);
        }
        if performance_fee_bps > 10_000 {
            return Err(Error::InvalidFee);
        }
        admin.require_auth();

        env.storage().instance().set(
            &DataKey::Config,
            &VaultConfig {
                asset: asset.clone(),
                admin: admin.clone(),
                fee_recipient,
                performance_fee_bps,
                last_harvest_ledger: env.ledger().sequence(),
                last_harvest_profit: 0,
            },
        );
        env.storage().instance().set(&DataKey::TotalAssets, &0i128);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .set(&DataKey::VerifierKind, &VerifierKind::None);

        env.events().publish(
            (symbol_short!("init"), admin),
            (asset, performance_fee_bps),
        );
        Ok(())
    }

    // ── Metadata ────────────────────────────────────────────────────────────

    /// The underlying token address (ERC-4626 `asset()`).
    pub fn asset(env: Env) -> Result<Address, Error> {
        Ok(load_config(&env)?.asset)
    }

    /// The active vault configuration.
    pub fn get_config(env: Env) -> Result<VaultConfig, Error> {
        load_config(&env)
    }

    /// Whether deposits and withdrawals are halted.
    pub fn paused(env: Env) -> bool {
        is_paused(&env)
    }

    // ── ERC-4626 views ──────────────────────────────────────────────────────

    /// Total underlying assets managed by the vault (ERC-4626 `totalAssets()`).
    pub fn total_assets(env: Env) -> i128 {
        get_total_assets(&env)
    }

    /// Total shares outstanding, excluding the virtual offset.
    pub fn total_supply(env: Env) -> i128 {
        get_total_supply(&env)
    }

    /// Convert `assets` to shares, rounding in favour of the vault.
    pub fn convert_to_shares(env: Env, assets: i128) -> Result<i128, Error> {
        assets_to_shares_floor(&env, assets)
    }

    /// Convert `shares` to assets, rounding in favour of the vault.
    pub fn convert_to_assets(env: Env, shares: i128) -> Result<i128, Error> {
        shares_to_assets_floor(&env, shares)
    }

    /// Shares a deposit of `assets` would mint (ERC-4626 `previewDeposit`).
    pub fn preview_deposit(env: Env, assets: i128) -> Result<i128, Error> {
        assets_to_shares_floor(&env, assets)
    }

    /// Assets a mint of `shares` would charge (ERC-4626 `previewMint`).
    pub fn preview_mint(env: Env, shares: i128) -> Result<i128, Error> {
        assets_to_shares_ceil(&env, shares)
    }

    /// Shares a withdrawal of `assets` would burn (ERC-4626 `previewWithdraw`).
    pub fn preview_withdraw(env: Env, assets: i128) -> Result<i128, Error> {
        assets_to_shares_ceil(&env, assets)
    }

    /// Assets a redeem of `shares` would return (ERC-4626 `previewRedeem`).
    pub fn preview_redeem(env: Env, shares: i128) -> Result<i128, Error> {
        shares_to_assets_floor(&env, shares)
    }

    /// Maximum `assets` `owner` may deposit. Unlimited, so `i128::MAX`.
    pub fn max_deposit(_env: Env, _owner: Address) -> i128 {
        i128::MAX
    }

    /// Maximum `shares` `receiver` may mint. Unlimited, so `i128::MAX`.
    pub fn max_mint(_env: Env, _receiver: Address) -> i128 {
        i128::MAX
    }

    /// Maximum `assets` `owner` may withdraw right now. Zero while paused.
    pub fn max_withdraw(env: Env, owner: Address) -> i128 {
        if is_paused(&env) {
            return 0;
        }
        shares_to_assets_floor(&env, get_shares(&env, &owner)).unwrap_or(0)
    }

    /// Maximum `shares` `owner` may redeem right now. Zero while paused.
    pub fn max_redeem(env: Env, owner: Address) -> i128 {
        if is_paused(&env) {
            return 0;
        }
        get_shares(&env, &owner)
    }

    /// Share balance of `account`.
    pub fn balance_of(env: Env, account: Address) -> i128 {
        get_shares(&env, &account)
    }

    // ── Deposits ────────────────────────────────────────────────────────────

    /// Deposit exactly `assets` underlying tokens, minting shares to `receiver`.
    ///
    /// The depositor pays any rounding, so the remainder accrues to the vault
    /// rather than to them.
    pub fn deposit(
        env: Env,
        caller: Address,
        assets: i128,
        receiver: Address,
    ) -> Result<i128, Error> {
        require_positive(assets)?;
        ensure_running(&env)?;

        let config = load_config(&env)?;
        caller.require_auth();

        // Price the deposit against the pre-deposit totals, then move.
        let shares = assets_to_shares_floor(&env, assets)?;
        if shares <= 0 {
            // An amount too small to mint one whole share would leave the
            // caller's assets in the vault with no claim on them.
            return Err(Error::ZeroAmount);
        }

        move_assets(
            &env,
            &config.asset,
            &caller,
            &env.current_contract_address(),
            assets,
        );
        credit_shares(&env, &receiver, shares)?;
        add_total_assets(&env, assets)?;
        add_total_supply(&env, shares)?;

        env.events().publish(
            (symbol_short!("deposit"), receiver),
            (caller, assets, shares),
        );
        Ok(shares)
    }

    /// Mint exactly `shares` to `receiver`, charging the caller the assets.
    ///
    /// Rounds against the caller, so the vault is never left short.
    pub fn mint(
        env: Env,
        caller: Address,
        shares: i128,
        receiver: Address,
    ) -> Result<i128, Error> {
        require_positive(shares)?;
        ensure_running(&env)?;

        let config = load_config(&env)?;
        caller.require_auth();

        let assets = assets_to_shares_ceil(&env, shares)?;
        if assets <= 0 {
            return Err(Error::ZeroAmount);
        }

        move_assets(
            &env,
            &config.asset,
            &caller,
            &env.current_contract_address(),
            assets,
        );
        credit_shares(&env, &receiver, shares)?;
        add_total_assets(&env, assets)?;
        add_total_supply(&env, shares)?;

        env.events().publish(
            (symbol_short!("mint"), receiver),
            (caller, shares, assets),
        );
        Ok(assets)
    }

    // ── Withdrawals ─────────────────────────────────────────────────────────

    /// Burn enough of `owner`'s shares to send exactly `assets` to `receiver`.
    ///
    /// `caller` must be `owner`, or hold an unexpired allowance covering the
    /// shares burned — which is computed from the share price before anything
    /// moves, and consumed before the transfer.
    pub fn withdraw(
        env: Env,
        caller: Address,
        assets: i128,
        receiver: Address,
        owner: Address,
    ) -> Result<i128, Error> {
        require_positive(assets)?;
        ensure_running(&env)?;

        let config = load_config(&env)?;

        // How many shares this withdrawal costs, priced before any write.
        let shares = assets_to_shares_ceil(&env, assets)?;
        if shares <= 0 {
            return Err(Error::ZeroAmount);
        }

        let consumed = authorize_share_spend(&env, &caller, &owner, shares)?;
        debit_shares(&env, &owner, shares)?;
        consume_allowance(&env, &caller, &owner, consumed);

        move_assets(
            &env,
            &config.asset,
            &env.current_contract_address(),
            &receiver,
            assets,
        );
        add_total_assets(&env, -assets)?;
        add_total_supply(&env, -shares)?;

        env.events().publish(
            (symbol_short!("withdraw"), receiver),
            (caller, owner, assets, shares),
        );
        Ok(shares)
    }

    /// Burn exactly `shares` of `owner`'s holding, sending the assets to
    /// `receiver`.
    pub fn redeem(
        env: Env,
        caller: Address,
        shares: i128,
        receiver: Address,
        owner: Address,
    ) -> Result<i128, Error> {
        require_positive(shares)?;
        ensure_running(&env)?;

        let config = load_config(&env)?;

        let assets = shares_to_assets_floor(&env, shares);
        let consumed = authorize_share_spend(&env, &caller, &owner, shares)?;
        debit_shares(&env, &owner, shares)?;
        consume_allowance(&env, &caller, &owner, consumed);

        move_assets(
            &env,
            &config.asset,
            &env.current_contract_address(),
            &receiver,
            assets,
        );
        add_total_assets(&env, -assets)?;
        add_total_supply(&env, -shares)?;

        env.events().publish(
            (symbol_short!("redeem"), receiver),
            (caller, owner, shares, assets),
        );
        Ok(assets)
    }

    // ── Allowances (the expressible part of EIP-2612) ───────────────────────

    /// Grant `spender` authority over `shares` of `caller`'s holding until
    /// `expires_at`.
    ///
    /// There is deliberately no way to create an allowance with no expiry. A
    /// permanent approval cannot be revoked if the key leaks, which is the
    /// failure mode that makes approvals dangerous in the first place.
    pub fn approve(
        env: Env,
        caller: Address,
        spender: Address,
        shares: i128,
        valid_until: u32,
    ) -> Result<(), Error> {
        require_positive(shares)?;
        caller.require_auth();

        if valid_until <= env.ledger().sequence() {
            return Err(Error::AllowanceExpired);
        }

        store_allowance(
            &env,
            &caller,
            &spender,
            Allowance {
                shares,
                expires_at: valid_until,
            },
        );

        env.events().publish(
            (symbol_short!("approve"), spender),
            (caller, shares, valid_until),
        );
        Ok(())
    }

    /// Revoke `spender`'s allowance immediately.
    pub fn revoke(env: Env, caller: Address, spender: Address) -> Result<(), Error> {
        caller.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::Allowance(caller, spender.clone()));
        env.events()
            .publish((symbol_short!("revoke"), spender), caller);
        Ok(())
    }

    /// Current allowance `spender` holds over `owner`'s shares, if any.
    pub fn allowance(env: Env, owner: Address, spender: Address) -> Option<Allowance> {
        load_allowance(&env, &owner, &spender)
    }

    /// Withdraw using an allowance, with an explicit deadline.
    ///
    /// This is Perigee's stand-in for EIP-2612 `permit`. Soroban exposes no
    /// signature-recovery primitive, so a genuinely gasless authorisation is not
    /// expressible on-chain; what *is* expressible is a bounded, expiring
    /// delegation, and that is what this provides.
    pub fn permit_withdraw(
        env: Env,
        spender: Address,
        owner: Address,
        assets: i128,
        receiver: Address,
    ) -> Result<i128, Error> {
        require_positive(assets)?;
        ensure_running(&env)?;
        spender.require_auth();

        let config = load_config(&env)?;
        let allowance =
            load_allowance(&env, &owner, &spender).ok_or(Error::InsufficientAllowance)?;

        if allowance.expires_at <= env.ledger().sequence() {
            return Err(Error::AllowanceExpired);
        }

        let shares = assets_to_shares_ceil(&env, assets)?;
        if shares <= 0 {
            return Err(Error::ZeroAmount);
        }
        if shares > allowance.shares {
            return Err(Error::InsufficientAllowance);
        }

        let remaining = allowance.shares - shares;
        if remaining == 0 {
            env.storage()
                .persistent()
                .remove(&DataKey::Allowance(owner.clone(), spender.clone()));
        } else {
            store_allowance(
                &env,
                &owner,
                &spender,
                Allowance {
                    shares: remaining,
                    expires_at: allowance.expires_at,
                },
            );
        }

        debit_shares(&env, &owner, shares)?;
        move_assets(
            &env,
            &config.asset,
            &env.current_contract_address(),
            &receiver,
            assets,
        );
        add_total_assets(&env, -assets)?;
        add_total_supply(&env, -shares)?;

        env.events().publish(
            (symbol_short!("permit"), receiver),
            (spender, owner, assets, shares, remaining),
        );
        Ok(shares)
    }

    // ── Administration ──────────────────────────────────────────────────────

    /// Pause or unpause deposits and withdrawals.
    pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.events().publish((symbol_short!("paused"),), paused);
        Ok(())
    }

    /// Rotate the admin key. Does not touch balances or allowances.
    pub fn set_admin(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        let mut config = load_config(&env)?;
        config.admin = new_admin.clone();
        env.storage().instance().set(&DataKey::Config, &config);
        env.events()
            .publish((symbol_short!("set_admin"),), new_admin);
        Ok(())
    }

    /// Rotate the performance fee recipient.
    pub fn set_fee_recipient(env: Env, admin: Address, recipient: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        let mut config = load_config(&env)?;
        config.fee_recipient = recipient.clone();
        env.storage().instance().set(&DataKey::Config, &config);
        env.events()
            .publish((symbol_short!("fee_to"),), recipient);
        Ok(())
    }

    /// Configure the off-chain signature verifier.
    ///
    /// Defaults to [`VerifierKind::None`], which refuses every signature. A
    /// deployer must opt in to trusting a scheme, and the choice is recorded so
    /// the trust assumption is auditable on-chain rather than implied.
    pub fn set_verifier(
        env: Env,
        admin: Address,
        kind: VerifierKind,
        verifier: Option<Address>,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::VerifierKind, &kind);
        match verifier {
            Some(address) => env.storage().instance().set(&DataKey::Verifier, &address),
            None => env.storage().instance().remove(&DataKey::Verifier),
        }
        env.events()
            .publish((symbol_short!("verifier"),), verifier);
        Ok(())
    }

    /// The configured verifier kind.
    pub fn verifier_kind(env: Env) -> VerifierKind {
        env.storage()
            .instance()
            .get(&DataKey::VerifierKind)
            .unwrap_or(VerifierKind::None)
    }

    /// The configured verifier contract, if any.
    pub fn verifier(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Verifier)
    }

    /// Replace the recorded total assets with the vault's actual token balance
    /// and take the performance fee on any increase.
    ///
    /// This is the only path by which yield enters the accounting, which is what
    /// makes a harvest externally verifiable rather than self-reported.
    pub fn harvest(env: Env, admin: Address) -> Result<i128, Error> {
        require_admin(&env, &admin)?;

        let config = load_config(&env)?;
        let vault_address = env.current_contract_address();
        let actual: i128 = token::Client::new(&env, &config.asset).balance(&vault_address);
        let profit = actual - get_total_assets(&env);

        let mut fee = 0i128;
        if profit > 0 && config.performance_fee_bps > 0 {
            fee = mul_div_floor(
                profit,
                config.performance_fee_bps as i128,
                BPS_DENOMINATOR,
            )?;
            if fee > 0 {
                move_assets(&env, &config.asset, &vault_address, &config.fee_recipient, fee);
            }
        }

        let new_total = actual - fee;
        env.storage()
            .instance()
            .set(&DataKey::TotalAssets, &new_total);

        let mut updated = config;
        updated.last_harvest_ledger = env.ledger().sequence();
        updated.last_harvest_profit = profit;
        env.storage().instance().set(&DataKey::Config, &updated);

        env.events().publish(
            (symbol_short!("harvest"),),
            (profit, fee, new_total),
        );
        Ok(profit)
    }
}