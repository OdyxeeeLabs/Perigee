#![no_std]

mod admin;
mod allowance;
mod balance;
mod contract;
mod metadata;
mod storage_types;

#[cfg(test)]
mod test;
#[cfg(test)]
mod test_multisig;

pub use crate::contract::Token;
pub use crate::contract::TokenClient;
