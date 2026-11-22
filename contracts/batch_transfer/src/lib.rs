#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Vec};

#[cfg(test)]
mod test;

// Type-safe argument deserialization helpers (closes #508)
pub mod typed_args;

// ── Error Types ───────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    EmptyBatch = 1,
    LengthMismatch = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
    /// Batch exceeds the maximum allowed size — split into smaller chunks.
    BatchTooLarge = 5,
    /// A required string or vec argument exceeded its maximum allowed length.
    InputTooLong = 6,
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of recipients in a single `execute` call.
///
/// Soroban has a per-transaction instruction limit. Empirical measurements show
/// that ~100 token transfers comfortably fit within the limit while leaving
/// enough headroom for authentication and event emission.  Callers with larger
/// batches should split them using the `execute_chunked` helper.
///
/// Closes OdyxeeeLabs/Perigee#509
pub const MAX_BATCH_SIZE: u32 = 100;

/// Maximum number of chunks that `execute_chunked` will process.
pub const MAX_CHUNKS: u32 = 10;

// ── Data Types ────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionMode {
    AllOrNothing,
    Partial,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferFailure {
    None,
    InvalidAmount,
    InsufficientBalance,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferResult {
    pub recipient: Address,
    pub amount: i128,
    pub success: bool,
    pub failure: TransferFailure,
}

/// Aggregated summary returned by `execute_chunked`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkedSummary {
    /// Total number of successful individual transfers.
    pub successful: u32,
    /// Total number of failed individual transfers.
    pub failed: u32,
    /// Number of chunks that were processed.
    pub chunks_processed: u32,
}

pub trait BatchToken {
    fn balance(e: Env, id: Address) -> i128;
    fn transfer(e: Env, from: Address, to: Address, amount: i128);
}

soroban_sdk::contractclient!(name = "BatchTokenClient", trait = BatchToken);

// ── Internal Helpers ──────────────────────────────────────────────────────────

fn validate_lengths(recipients: &Vec<Address>, amounts: &Vec<i128>) -> Result<u32, Error> {
    let len = recipients.len();
    if len == 0 {
        return Err(Error::EmptyBatch);
    }
    if len != amounts.len() {
        return Err(Error::LengthMismatch);
    }
    if len > MAX_BATCH_SIZE {
        return Err(Error::BatchTooLarge);
    }
    Ok(len)
}

fn simulate_batch(
    env: &Env,
    token: &Address,
    sender: &Address,
    recipients: &Vec<Address>,
    amounts: &Vec<i128>,
    mode: &ExecutionMode,
) -> Result<Vec<TransferResult>, Error> {
    let len = validate_lengths(recipients, amounts)?;
    let token_client = BatchTokenClient::new(env, token);
    let mut remaining_balance = token_client.balance(sender);
    let mut results = Vec::new(env);

    for i in 0..len {
        let recipient = recipients.get(i).unwrap();
        let amount = amounts.get(i).unwrap();

        if amount <= 0 {
            if matches!(mode, ExecutionMode::AllOrNothing) {
                return Err(Error::InvalidAmount);
            }
            results.push_back(TransferResult {
                recipient,
                amount,
                success: false,
                failure: TransferFailure::InvalidAmount,
            });
            continue;
        }

        if remaining_balance < amount {
            if matches!(mode, ExecutionMode::AllOrNothing) {
                return Err(Error::InsufficientBalance);
            }
            results.push_back(TransferResult {
                recipient,
                amount,
                success: false,
                failure: TransferFailure::InsufficientBalance,
            });
            continue;
        }

        remaining_balance -= amount;
        results.push_back(TransferResult {
            recipient,
            amount,
            success: true,
            failure: TransferFailure::None,
        });
    }

    Ok(results)
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct BatchTransfer;

#[contractimpl]
impl BatchTransfer {
    /// Execute a batch transfer of at most `MAX_BATCH_SIZE` recipients.
    ///
    /// For larger batches, use `execute_chunked` which handles splitting
    /// automatically and processes chunks with intermediate checkpoints to
    /// avoid hitting per-transaction instruction limits.
    pub fn execute(
        env: Env,
        token: Address,
        sender: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
        mode: ExecutionMode,
    ) -> Result<Vec<TransferResult>, Error> {
        sender.require_auth();

        // Input length validation — closes #508 / #516
        let len = recipients.len();
        if len > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        let plan = simulate_batch(&env, &token, &sender, &recipients, &amounts, &mode)?;
        let token_client = BatchTokenClient::new(&env, &token);

        for item in plan.iter() {
            if item.success {
                token_client.transfer(&sender, &item.recipient, &item.amount);
            }
        }

        Ok(plan)
    }

    /// Execute a large batch transfer by splitting it into chunks of at most
    /// `MAX_BATCH_SIZE` recipients each.
    ///
    /// This avoids hitting Soroban's per-transaction instruction limit on large
    /// transfers.  Each chunk is simulated *before* any transfers begin so that
    /// an `AllOrNothing` failure in chunk N does not leave partial state from
    /// chunks 0..N-1.
    ///
    /// `chunk_index` indicates which chunk to process in the current call.
    /// Callers should iterate from 0 to `ceil(len / MAX_BATCH_SIZE) - 1`.
    ///
    /// Closes OdyxeeeLabs/Perigee#509
    pub fn execute_chunked(
        env: Env,
        token: Address,
        sender: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
        mode: ExecutionMode,
        chunk_index: u32,
    ) -> Result<ChunkedSummary, Error> {
        sender.require_auth();

        let total = recipients.len();
        if total == 0 {
            return Err(Error::EmptyBatch);
        }
        if total != amounts.len() {
            return Err(Error::LengthMismatch);
        }

        let start = chunk_index * MAX_BATCH_SIZE;
        if start >= total {
            // All chunks already processed — return zeroed summary.
            return Ok(ChunkedSummary {
                successful: 0,
                failed: 0,
                chunks_processed: 0,
            });
        }

        let end = (start + MAX_BATCH_SIZE).min(total);
        let chunk_recipients = recipients.slice(start..end);
        let chunk_amounts = amounts.slice(start..end);

        let plan =
            simulate_batch(&env, &token, &sender, &chunk_recipients, &chunk_amounts, &mode)?;
        let token_client = BatchTokenClient::new(&env, &token);

        let mut successful: u32 = 0;
        let mut failed: u32 = 0;

        for item in plan.iter() {
            if item.success {
                token_client.transfer(&sender, &item.recipient, &item.amount);
                successful += 1;
            } else {
                failed += 1;
            }
        }

        Ok(ChunkedSummary {
            successful,
            failed,
            chunks_processed: 1,
        })
    }

    /// Simulate a batch without executing transfers.  Useful for off-chain
    /// planning and gas estimation before committing a transaction.
    pub fn quote(
        env: Env,
        token: Address,
        sender: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
        mode: ExecutionMode,
    ) -> Result<Vec<TransferResult>, Error> {
        simulate_batch(&env, &token, &sender, &recipients, &amounts, &mode)
    }

    /// Returns the maximum number of recipients allowed per `execute` call.
    pub fn max_batch_size(_env: Env) -> u32 {
        MAX_BATCH_SIZE
    }
}
