#![no_std]

//! Queue and execute contract calls after a configurable timestamp delay.
//!
//! The controller stores an operation before execution, permits a designated
//! canceller to cancel pending calls, and only permits execution once the
//! configured minimum delay has elapsed.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, xdr::ToXdr, Address,
    BytesN, Env, Symbol, Val, Vec,
};

#[cfg(test)]
mod test;

/// Ledgers that persistent operations remain live after an access.
pub const TTL_LEDGERS: u32 = 3_110_400;

/// Maximum delay keeps a queued operation's record live beyond its ready time
/// under Soroban's maximum persistent-entry TTL.
pub const MAX_DELAY_SECONDS: u64 = 7_776_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidDelay = 4,
    OperationAlreadyExists = 5,
    OperationNotFound = 6,
    OperationNotPending = 7,
    OperationNotReady = 8,
    TimeOverflow = 9,
}

/// Lifecycle state of a queued operation.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
    Pending,
    Executing,
    Done,
    Canceled,
}

/// Timelock roles and minimum delay.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerConfig {
    pub admin: Address,
    pub proposer: Address,
    pub executor: Address,
    pub canceller: Address,
    /// Minimum delay in seconds, measured against the ledger timestamp.
    pub min_delay: u64,
}

/// A stored invocation waiting for execution.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    pub id: BytesN<32>,
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub salt: BytesN<32>,
    pub proposer: Address,
    pub queued_at: u64,
    pub ready_at: u64,
    pub status: OperationStatus,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Config,
    Operation(BytesN<32>),
}

fn load_config(env: &Env) -> Result<ControllerConfig, Error> {
    let config = env
        .storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)?;
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    Ok(config)
}

fn save_config(env: &Env, config: &ControllerConfig) {
    env.storage().instance().set(&DataKey::Config, config);
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
}

fn require_role(caller: &Address, expected: &Address) -> Result<(), Error> {
    if caller != expected {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

fn operation_id(
    env: &Env,
    target: &Address,
    function: &Symbol,
    args: &Vec<Val>,
    salt: &BytesN<32>,
) -> BytesN<32> {
    env.crypto()
        .sha256(
            &(
                target.clone(),
                function.clone(),
                args.clone(),
                salt.clone(),
            )
                .to_xdr(env),
        )
        .into()
}

fn load_operation(env: &Env, id: &BytesN<32>) -> Result<Operation, Error> {
    let key = DataKey::Operation(id.clone());
    let operation: Operation = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(Error::OperationNotFound)?;
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
    Ok(operation)
}

fn save_operation(env: &Env, operation: &Operation) {
    let key = DataKey::Operation(operation.id.clone());
    env.storage().persistent().set(&key, operation);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
}

#[contract]
pub struct TimelockController;

#[contractimpl]
impl TimelockController {
    /// Initialize the controller. A non-zero minimum delay is required.
    pub fn initialize(
        env: Env,
        admin: Address,
        proposer: Address,
        executor: Address,
        canceller: Address,
        min_delay: u64,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(Error::AlreadyInitialized);
        }
        if min_delay == 0 || min_delay > MAX_DELAY_SECONDS {
            return Err(Error::InvalidDelay);
        }
        admin.require_auth();
        let config = ControllerConfig {
            admin,
            proposer,
            executor,
            canceller,
            min_delay,
        };
        save_config(&env, &config);
        env.events().publish(
            (symbol_short!("init"),),
            (
                config.admin,
                config.proposer,
                config.executor,
                config.canceller,
                config.min_delay,
            ),
        );
        Ok(())
    }

    /// Queue a contract invocation. `ready_at` is calculated from the current
    /// ledger timestamp plus the configured minimum delay.
    pub fn queue(
        env: Env,
        proposer: Address,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        salt: BytesN<32>,
    ) -> Result<Operation, Error> {
        let config = load_config(&env)?;
        require_role(&proposer, &config.proposer)?;

        let now = env.ledger().timestamp();
        let ready_at = now.checked_add(config.min_delay).ok_or(Error::TimeOverflow)?;
        let id = operation_id(&env, &target, &function, &args, &salt);
        let key = DataKey::Operation(id.clone());
        if let Some(existing) = env.storage().persistent().get::<_, Operation>(&key) {
            if existing.status != OperationStatus::Canceled {
                return Err(Error::OperationAlreadyExists);
            }
        }

        let operation = Operation {
            id: id.clone(),
            target,
            function,
            args,
            salt,
            proposer: proposer.clone(),
            queued_at: now,
            ready_at,
            status: OperationStatus::Pending,
        };
        save_operation(&env, &operation);
        env.events()
            .publish((symbol_short!("queued"), id), (proposer, ready_at));
        Ok(operation)
    }

    /// Execute a queued invocation once its ready timestamp has passed.
    ///
    /// The status is marked `Executing` before the external call, preventing
    /// reentrant execution. If the target call fails, Soroban transaction
    /// atomicity restores the operation to `Pending`.
    pub fn execute(env: Env, executor: Address, id: BytesN<32>) -> Result<Val, Error> {
        let config = load_config(&env)?;
        require_role(&executor, &config.executor)?;
        let mut operation = load_operation(&env, &id)?;
        if operation.status != OperationStatus::Pending {
            return Err(Error::OperationNotPending);
        }
        if env.ledger().timestamp() < operation.ready_at {
            return Err(Error::OperationNotReady);
        }

        operation.status = OperationStatus::Executing;
        save_operation(&env, &operation);
        let result = env.invoke_contract(
            &operation.target,
            &operation.function,
            operation.args.clone(),
        );
        operation.status = OperationStatus::Done;
        save_operation(&env, &operation);
        env.events().publish(
            (symbol_short!("executed"), id),
            (executor, operation.target, operation.function),
        );
        Ok(result)
    }

    /// Cancel a pending operation.
    pub fn cancel(env: Env, canceller: Address, id: BytesN<32>) -> Result<(), Error> {
        let config = load_config(&env)?;
        require_role(&canceller, &config.canceller)?;
        let mut operation = load_operation(&env, &id)?;
        if operation.status != OperationStatus::Pending {
            return Err(Error::OperationNotPending);
        }
        operation.status = OperationStatus::Canceled;
        save_operation(&env, &operation);
        env.events()
            .publish((symbol_short!("canceled"), id), canceller);
        Ok(())
    }

    /// Update the minimum delay. Admin changes are immediate and audited.
    pub fn set_min_delay(env: Env, admin: Address, min_delay: u64) -> Result<(), Error> {
        let mut config = load_config(&env)?;
        require_role(&admin, &config.admin)?;
        if min_delay == 0 || min_delay > MAX_DELAY_SECONDS {
            return Err(Error::InvalidDelay);
        }
        let old_delay = config.min_delay;
        config.min_delay = min_delay;
        save_config(&env, &config);
        env.events()
            .publish((symbol_short!("delay"),), (admin, old_delay, min_delay));
        Ok(())
    }

    /// Replace proposer, executor, and canceller authorities.
    pub fn set_roles(
        env: Env,
        admin: Address,
        proposer: Address,
        executor: Address,
        canceller: Address,
    ) -> Result<(), Error> {
        let mut config = load_config(&env)?;
        require_role(&admin, &config.admin)?;
        config.proposer = proposer.clone();
        config.executor = executor.clone();
        config.canceller = canceller.clone();
        save_config(&env, &config);
        env.events().publish(
            (symbol_short!("roles"),),
            (admin, proposer, executor, canceller),
        );
        Ok(())
    }

    /// Transfer controller administration.
    pub fn transfer_admin(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
        let mut config = load_config(&env)?;
        require_role(&admin, &config.admin)?;
        config.admin = new_admin.clone();
        save_config(&env, &config);
        env.events()
            .publish((symbol_short!("admin"),), (admin, new_admin));
        Ok(())
    }

    /// Read controller configuration.
    pub fn config(env: Env) -> Result<ControllerConfig, Error> {
        load_config(&env)
    }

    /// Read an operation by id.
    pub fn operation(env: Env, id: BytesN<32>) -> Result<Operation, Error> {
        load_operation(&env, &id)
    }

    /// Recompute an operation id before it is queued.
    pub fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        operation_id(&env, &target, &function, &args, &salt)
    }
}
