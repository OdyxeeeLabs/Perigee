#![no_std]

mod admin;
mod allowance;
mod balance;
mod contract;
mod metadata;
mod storage_types;

// Re-export emergency guard for use in token contracts
pub use emergency_guard;

#[cfg(test)]
mod test;
#[cfg(test)]
mod test_admin_rotation;

pub use crate::contract::Token;
pub use crate::contract::TokenClient;
