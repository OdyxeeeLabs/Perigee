#![no_std]
use soroban_sdk::contracterror;

/// Unified error codes shared across all SoroScope contracts.
///
/// Each variant maps to a stable `u32` discriminant so that the SoroScope UI
/// can decode errors consistently regardless of which contract emitted them.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    // ── Initialisation ──────────────────────────────────────────────────────
    AlreadyInitialized = 1,
    NotInitialized = 2,

    // ── Authorisation ───────────────────────────────────────────────────────
    Unauthorized = 3,

    // ── Balances & liquidity ─────────────────────────────────────────────────
    InsufficientBalance = 4,
    InsufficientLiquidity = 5,
    InsufficientShares = 6,
    InsufficientAllowance = 7,

    // ── Swap / pricing ───────────────────────────────────────────────────────
    SlippageExceeded = 8,

    // ── Fee management ───────────────────────────────────────────────────────
    InvalidFee = 9,
    NoPendingFeeUpdate = 10,
    TimelockNotElapsed = 11,

    // ── Oracle ───────────────────────────────────────────────────────────────
    OracleNotConfigured = 12,
    InvalidOraclePrice = 13,

    // ── Circuit-breaker ──────────────────────────────────────────────────────
    Paused = 14,

    // ── Math ─────────────────────────────────────────────────────────────────
    Overflow = 15,
    DivisionByZero = 16,
    InvalidInput = 17,
}
