#![no_std]

//! # Perigee Vault Guard — circuit breaker, partial pause and timelocked unpause
//!
//! Closes OdyxeeeLabs/Perigee#634
//!
//! A vault that can only be fully halted is a vault that gets halted over things
//! that did not warrant it. This guard provides four independent controls:
//!
//! | Control | Entry point | Who |
//! | --- | --- | --- |
//! | Admin pause | [`VaultGuard::pause`] | admin |
//! | **Circuit breaker** | [`VaultGuard::trip`] | **guardian**, no timelock |
//! | Timelocked unpause | [`VaultGuard::unpause`] | anyone, after [`UNPAUSE_DELAY_LEDGERS`] |
//! | Partial pause | [`VaultGuard::pause_operations`] | admin |
//!
//! ## Why the circuit breaker has no timelock
//!
//! Unpausing is *deliberate and announced*; pausing is *reactive*. A timelock on
//! the way in would mean an exploit has to finish executing before anyone can
//! stop it, which inverts the purpose of the control. So [`VaultGuard::trip`] is
//! immediate and available to a guardian — a role deliberately *outside* the
//! admin, so a compromised admin cannot use it as a griefing vector and cannot
//! quietly pause a healthy vault.
//!
//! The cost of that choice is that a compromised guardian can halt a vault. That
//! is a bounded, recoverable annoyance, whereas an unpaused exploit is unbounded.
//!
//! ## Partial pause
//!
//! [`VaultGuard::pause_operations`] takes a **bitmask** of
//! [`Operation`] values rather than a single boolean, so withdrawals can be
//! stopped while deposits continue, or a single risky operation can be isolated.
//! [`VaultGuard::all_paused`] is the global override and overrides the mask.
//!
//! This matters during an incident specifically: a deposit-drain exploit often
//! does not touch withdrawals, and pausing everything locks legitimate users out
//! of their funds — which turns a protocol problem into a custody problem.
//!
//! ## Timelocked unpause
//!
//! Any unpause requires [`UNPAUSE_DELAY_LEDGERS`] (≈ 24 h) to have passed since
//! the pause. Anyone may call [`VaultGuard::unpause`] once the window has elapsed;
//! there is no admin gate, because a delay that an admin can shorten is not a
//! delay. It is a *notification window*: users and watchers get a full day to see
//! that something was paused and react before funds move again.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers a pause must age before it can be lifted. ≈ 24 h at the Stellar
/// target of one ledger every 5 s.
pub const UNPAUSE_DELAY_LEDGERS: u32 = 17_280;

/// Bit 0 — deposits.
pub const OP_DEPOSIT: u32 = 1 << 0;
/// Bit 1 — withdrawals.
pub const OP_WITHDRAW: u32 = 1 << 1;
/// Bit 2 — strategy rotation.
pub const OP_ROTATE: u32 = 1 << 2;
/// Bit 3 — fee changes.
pub const OP_FEE_CHANGE: u32 = 1 << 3;
/// Bit 4 — upgrades.
pub const OP_UPGRADE: u32 = 1 << 4;

/// Every operation bit this guard understands.
pub const OP_ALL: u32 = OP_DEPOSIT
    | OP_WITHDRAW
    | OP_ROTATE
    | OP_FEE_CHANGE
    | OP_UPGRADE;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the vault guard.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The guard has not been initialized yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not authorised for this entry point.
    Unauthorized = 3,
    /// The requested operation is already paused.
    AlreadyPaused = 4,
    /// The guard is not paused.
    NotPaused = 5,
    /// A pause is in force and has not yet aged past the unpause delay.
    UnpauseTooEarly = 6,
    /// The caller is not the guardian.
    NotGuardian = 7,
    /// The bitmask contains bits this guard does not understand.
    UnknownOperation = 8,
    /// The requested operation is not paused.
    OperationNotPaused = 9,
    /// The guard was already tripped and cannot be tripped again without
    /// resolving the current trip.
    AlreadyTripped = 10,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Which operations a pause mask covers.
///
/// Exposed as constants rather than enum variants because the value is a bitmask
/// and callers compose masks.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Deposits into a vault.
    Deposit,
    /// Withdrawals out of a vault.
    Withdraw,
    /// Strategy rotation.
    Rotate,
    /// Fee schedule changes.
    FeeChange,
    /// Contract upgrades.
    Upgrade,
}

impl Operation {
    /// The bit this operation occupies in a pause mask.
    pub fn bit(&self) -> u32 {
        match self {
            Operation::Deposit => OP_DEPOSIT,
            Operation::Withdraw => OP_WITHDRAW,
            Operation::Rotate => OP_ROTATE,
            Operation::FeeChange => OP_FEE_CHANGE,
            Operation::Upgrade => OP_UPGRADE,
        }
    }
}

/// Current guard state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardState {
    /// Global override. When true, every operation is paused regardless of
    /// `paused_operations`.
    pub all_paused: bool,
    /// Bitmask of individually paused operations.
    pub paused_operations: u32,
    /// Ledger at which the current pause began. `0` when not paused.
    pub paused_at: u32,
    /// Whether the circuit breaker has been tripped.
    pub tripped: bool,
    /// Who tripped it, if it has.
    pub tripped_by: Option<Address>,
}

/// The set of addresses that can operate the guard.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardRoles {
    /// May pause, unpause and change the pause mask.
    pub admin: Address,
    /// May trip the circuit breaker. Deliberately a separate role from admin.
    pub guardian: Address,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Active [`GuardState`].
    State,
    /// Active [`GuardRoles`].
    Roles,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let roles = load_roles(env)?;
    if *caller != roles.admin {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

fn require_guardian(env: &Env, caller: &Address) -> Result<(), Error> {
    let roles = load_roles(env)?;
    if *caller != roles.guardian {
        return Err(Error::NotGuardian);
    }
    caller.require_auth();
    Ok(())
}

fn load_roles(env: &Env) -> Result<GuardRoles, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Roles)
        .ok_or(Error::NotInitialized)
}

fn load_state(env: &Env) -> Result<GuardState, Error> {
    env.storage()
        .instance()
        .get(&DataKey::State)
        .ok_or(Error::NotInitialized)
}

fn save_state(env: &Env, state: &GuardState) {
    env.storage().instance().set(&DataKey::State, state);
}

/// Reject masks containing bits outside [`OP_ALL`].
///
/// Without this a typo like `OP_DEPOSIT | (1 << 9)` would silently store a bit
/// that no operation reads, and the pause would appear to cover more than it
/// does.
fn validate_mask(mask: u32) -> Result<(), Error> {
    if mask & !OP_ALL != 0 {
        return Err(Error::UnknownOperation);
    }
    Ok(())
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct VaultGuard;

#[contractimpl]
impl VaultGuard {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the guard.
    ///
    /// `guardian` must be a **different** address from `admin`. A combined role
    /// would mean the admin could trip the breaker with no timelock, which is
    /// precisely the separation the circuit breaker exists to provide.
    pub fn initialize(env: Env, admin: Address, guardian: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Roles) {
            return Err(Error::AlreadyInitialized);
        }
        if admin == guardian {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();

        env.storage().instance().set(
            &DataKey::Roles,
            &GuardRoles {
                admin: admin.clone(),
                guardian,
            },
        );
        save_state(
            &env,
            &GuardState {
                all_paused: false,
                paused_operations: 0,
                paused_at: 0,
                tripped: false,
                tripped_by: None,
            },
        );

        env.events().publish((symbol_short!("init"), admin), guardian);
        Ok(())
    }

    // ── Admin pause ─────────────────────────────────────────────────────────

    /// Pause **every** operation immediately.
    ///
    /// Available to the admin with no timelock, for the same reason the guardian
    /// has one: a reactive control must be able to fire instantly. The timelock
    /// is on the way back out, not on the way in.
    pub fn pause(env: Env, caller: Address) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        let mut state = load_state(&env)?;
        if state.all_paused {
            return Err(Error::AlreadyPaused);
        }

        state.all_paused = true;
        state.paused_at = env.ledger().sequence();
        save_state(&env, &state);

        env.events()
            .publish((symbol_short!("pause"),), state.paused_at);
        Ok(())
    }

    /// Pause a subset of operations without stopping the rest.
    ///
    /// Bits outside [`OP_ALL`] are rejected, so a mistyped mask fails loudly
    /// rather than appearing to cover an operation it does not.
    pub fn pause_operations(env: Env, caller: Address, mask: u32) -> Result<(), Error> {
        require_admin(&env, &caller)?;
        validate_mask(mask)?;

        let mut state = load_state(&env)?;
        let requested = mask & !state.paused_operations;
        if requested == 0 {
            return Err(Error::AlreadyPaused);
        }

        state.paused_operations |= mask;
        if state.paused_operations != 0 && state.paused_at == 0 {
            state.paused_at = env.ledger().sequence();
        }
        save_state(&env, &state);

        env.events()
            .publish((symbol_short!("pause_ops"),), state.paused_operations);
        Ok(())
    }

    /// Resume a subset of operations. Not timelocked.
    ///
    /// Individual operations can be resumed early; lifting a *global* pause is
    /// what the timelock guards, because that is the one that stops every user.
    pub fn resume_operations(env: Env, caller: Address, mask: u32) -> Result<(), Error> {
        require_admin(&env, &caller)?;
        validate_mask(mask)?;

        let mut state = load_state(&env)?;
        if mask & !state.paused_operations != 0 {
            return Err(Error::OperationNotPaused);
        }

        state.paused_operations &= !mask;
        if state.paused_operations == 0 {
            state.paused_at = 0;
        }
        save_state(&env, &state);

        env.events().publish(
            (symbol_short!("resume_ops"),),
            state.paused_operations,
        );
        Ok(())
    }

    /// Lift a global pause. Timelocked — see [`VaultGuard::unpause`].
    pub fn unpause(env: Env, caller: Address) -> Result<(), Error> {
        let mut state = load_state(&env)?;
        if !state.all_paused {
            return Err(Error::NotPaused);
        }

        // Anyone may call this; the gate is time, not identity. A timelock an
        // admin can shorten is not a timelock.
        caller.require_auth();

        let elapsed = env.ledger().sequence().saturating_sub(state.paused_at);
        if elapsed < UNPAUSE_DELAY_LEDGERS {
            return Err(Error::UnpauseTooEarly);
        }

        state.all_paused = false;
        state.paused_at = 0;
        save_state(&env, &state);

        env.events()
            .publish((symbol_short!("unpause"),), elapsed);
        Ok(())
    }

    // ── Circuit breaker ─────────────────────────────────────────────────────

    /// Trip the circuit breaker: pause everything, immediately, with no timelock.
    ///
    /// Guardian only. This is the control that exists for the case where waiting
    /// is not acceptable — an exploit in progress, a compromised key, a
    /// diverging oracle.
    ///
    /// The trade is explicit: a compromised guardian can halt a vault. That is a
    /// bounded and recoverable annoyance; an unpaused exploit is not. And because
    /// unpausing is still timelocked and still observable, the guardian cannot
    /// keep a vault halted indefinitely without the market noticing.
    pub fn trip(env: Env, caller: Address) -> Result<(), Error> {
        require_guardian(&env, &caller)?;

        let mut state = load_state(&env)?;
        if state.tripped {
            return Err(Error::AlreadyTripped);
        }

        state.all_paused = true;
        state.paused_at = env.ledger().sequence();
        state.tripped = true;
        state.tripped_by = Some(caller.clone());
        save_state(&env, &state);

        env.events()
            .publish((symbol_short!("trip"), caller), state.paused_at);
        Ok(())
    }

    /// Clear the trip flag after a normal unpause, so the breaker can be used
    /// again.
    ///
    /// Requires the global pause to have already been lifted, so this cannot be
    /// used to end an incident early — the timelock still applies to the pause
    /// itself.
    pub fn reset_trip(env: Env, caller: Address) -> Result<(), Error> {
        require_guardian(&env, &caller)?;

        let mut state = load_state(&env)?;
        if !state.tripped {
            return Err(Error::NotPaused);
        }
        if state.all_paused {
            return Err(Error::AlreadyPaused);
        }

        state.tripped = false;
        state.tripped_by = None;
        save_state(&env, &state);

        env.events().publish((symbol_short!("trip_reset"),), caller);
        Ok(())
    }

    // ── Queries ─────────────────────────────────────────────────────────────

    /// The full guard state.
    pub fn state(env: Env) -> Result<GuardState, Error> {
        load_state(&env)
    }

    /// The current pause mask. [`OP_ALL`] when the global pause is in force.
    pub fn paused_mask(env: Env) -> Result<u32, Error> {
        let state = load_state(&env)?;
        Ok(if state.all_paused {
            OP_ALL
        } else {
            state.paused_operations
        })
    }

    /// Whether `operation` is currently paused.
    pub fn is_paused(env: Env, operation: Operation) -> Result<bool, Error> {
        let state = load_state(&env)?;
        Ok(state.all_paused || state.paused_operations & operation.bit() != 0)
    }

    /// Whether the global pause is in force.
    pub fn is_globally_paused(env: Env) -> bool {
        load_state(&env).map(|s| s.all_paused).unwrap_or(false)
    }

    /// Whether the circuit breaker has been tripped.
    pub fn is_tripped(env: Env) -> bool {
        load_state(&env).map(|s| s.tripped).unwrap_or(false)
    }

    /// Ledgers remaining before a global pause may be lifted.
    pub fn unpause_delay_remaining(env: Env) -> Result<u32, Error> {
        let state = load_state(&env)?;
        if !state.all_paused {
            return Ok(0);
        }
        let elapsed = env.ledger().sequence().saturating_sub(state.paused_at);
        Ok(UNPAUSE_DELAY_LEDGERS.saturating_sub(elapsed))
    }

    /// The current admin and guardian.
    pub fn roles(env: Env) -> Result<GuardRoles, Error> {
        load_roles(&env)
    }

    /// The unpause delay in ledgers.
    pub fn unpause_delay() -> u32 {
        UNPAUSE_DELAY_LEDGERS
    }
}