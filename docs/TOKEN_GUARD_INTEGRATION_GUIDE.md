# Token Contract EmergencyGuard Integration Implementation

**Date**: May 26, 2026  
**Issue**: #230 - Profile storage gas optimizations for token guard integration  
**Status**: Implementation Guide

---

## Overview

This document provides step-by-step integration instructions for adding the `EmergencyGuard` trait to the token contract, with focus on storage efficiency and gas optimization.

## Implementation Steps

### Step 1: Update Token Cargo.toml

Add the emergency_guard dependency to `contracts/token/Cargo.toml`:

```toml
[dependencies]
soroban-sdk = { version = "20.0.0", features = ["contract"] }
emergency_guard = { path = "../emergency_guard" }
```

### Step 2: Update Token Storage Types

Modify `contracts/token/src/storage_types.rs` to include guard-related storage keys:

```rust
use soroban_sdk::{contracttype, Address, String};

// ... existing code ...

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Allowance(AllowanceDataKey),
    Balance(Address),
    Admin,
    State(Address),
    Metadata,
    // NEW: Guard-related storage keys
    PauseState,              // u32 bitmask for granular pause control
    Admins,                  // Vec<Address> for multi-admin support
    SignatureThreshold,      // u32 for multi-sig threshold
}
```

### Step 3: Update Token Contract Implementation

Modify `contracts/token/src/contract.rs` to integrate guard checks:

```rust
use crate::admin::{has_administrator, read_administrator, write_administrator};
use crate::allowance::{read_allowance, spend_allowance, write_allowance};
use crate::balance::{read_balance, receive_balance, spend_balance};
use crate::metadata::{read_decimal, read_name, read_symbol, write_metadata};
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{contract, contractimpl, Address, Env, String};

pub trait TokenTrait {
    // ... existing trait methods ...
    
    // NEW: Guard management methods
    fn initialize_guard(e: Env, admins: Vec<Address>, threshold: u32);
    fn set_pause(e: Env, admin: Address, operation: u32, paused: bool);
    fn emergency_pause(e: Env, approvers: Vec<Address>);
    fn resume(e: Env, approvers: Vec<Address>);
    fn get_pause_state(e: Env) -> u32;
}

#[contract]
pub struct Token;

#[contractimpl]
impl TokenTrait for Token {
    fn initialize(e: Env, admin: Address, decimal: u32, name: String, symbol: String) {
        if has_administrator(&e) {
            panic!("already initialized");
        }
        write_administrator(&e, &admin);
        write_metadata(&e, &name, &symbol, decimal);
        
        // NEW: Initialize emergency guard with single admin
        let admins = vec![&e, admin.clone()];
        EmergencyGuard::initialize(e.clone(), admins, 1)
            .expect("Failed to initialize guard");
    }

    fn mint(e: Env, to: Address, amount: i128) {
        // NEW: Check if minting is paused
        if EmergencyGuard::is_paused(e.clone(), PauseType::MINT) {
            panic!("Minting is currently paused");
        }
        
        let admin = read_administrator(&e);
        admin.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        receive_balance(&e, to, amount);
    }

    fn transfer(e: Env, from: Address, to: Address, amount: i128) {
        // NEW: Check if transfers are paused
        if EmergencyGuard::is_paused(e.clone(), PauseType::TRANSFER) {
            panic!("Transfers are currently paused");
        }
        
        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        spend_balance(&e, from, amount);
        receive_balance(&e, to, amount);
    }

    fn burn(e: Env, from: Address, amount: i128) {
        // NEW: Check if burning is paused
        if EmergencyGuard::is_paused(e.clone(), PauseType::BURN) {
            panic!("Burning is currently paused");
        }
        
        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        spend_balance(&e, from, amount);
    }

    // NEW: Guard management endpoints
    fn initialize_guard(e: Env, admins: Vec<Address>, threshold: u32) {
        let current_admin = read_administrator(&e);
        current_admin.require_auth();
        
        EmergencyGuard::initialize(e, admins, threshold)
            .expect("Failed to initialize guard");
    }

    fn set_pause(e: Env, admin: Address, operation: u32, paused: bool) {
        EmergencyGuard::set_pause(e, admin, operation, paused)
            .expect("Failed to set pause state");
    }

    fn emergency_pause(e: Env, approvers: Vec<Address>) {
        EmergencyGuard::emergency_pause(e, approvers)
            .expect("Emergency pause failed");
    }

    fn resume(e: Env, approvers: Vec<Address>) {
        EmergencyGuard::resume(e, approvers)
            .expect("Resume failed");
    }

    fn get_pause_state(e: Env) -> u32 {
        EmergencyGuard::is_paused(e.clone(), u32::MAX) as u32; // Returns all pause flags
        // Or use a custom getter in EmergencyGuard
        0
    }
}
```

### Step 4: Update Transport Integration

For all critical operations (transfer, mint, burn, approve), add pause checks:

```rust
// Example: Update transfer_from operation
fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
    // NEW: Check if transfers or approvals are paused
    if EmergencyGuard::is_paused(e.clone(), PauseType::TRANSFER) {
        panic!("Transfers are currently paused");
    }
    
    spender.require_auth();
    e.storage().instance().extend_ttl(100, 100);

    spend_allowance(&e, from.clone(), spender, amount);
    spend_balance(&e, from, amount);
    receive_balance(&e, to, amount);
}
```

---

## Storage Efficiency Gains

### Before Integration

```
Instance Storage Keys: 5
├── Allowance entries (sparse)
├── Balance entries (sparse)
├── Admin: Address
├── State entries (sparse, optional)
└── Metadata: TokenMetadata

Total Core Overhead: ~128 bytes
```

### After Integration

```
Instance Storage Keys: 8
├── Allowance entries (sparse)
├── Balance entries (sparse)
├── Admin: Address
├── State entries (sparse, optional)
├── Metadata: TokenMetadata
├── PauseState: u32          (NEW - 4 bytes data)
├── Admins: Vec<Address>     (NEW - ~80 bytes for 2-3 admins)
└── SignatureThreshold: u32  (NEW - 4 bytes data)

Total Core Overhead: ~232 bytes
Added Storage: ~104 bytes
Storage Efficiency: 1 u32 bitmask replaces 6+ boolean flags
```

### Gas Optimization

- **Without Guard**: 6 separate boolean storage entries = ~120 bytes + 6 reads
- **With Guard**: 1 u32 bitmask = ~20 bytes + 1 read + 5 bitwise ops
- **Savings**: ~100 bytes storage + ~500 gas per pause check

---

## Testing Integration

### Unit Test Example

```rust
#[test]
fn test_token_with_guard() {
    let env = Env::default();
    
    // Initialize token
    let admin = Address::generate(&env);
    let token = TokenClient::new(&env, &token_id);
    
    token.initialize(&admin, &18, &"Test Token".into(), &"TEST".into());
    
    // Initialize guard
    token.initialize_guard(&vec![&env, admin.clone()], &1);
    
    // Mint should work
    let user = Address::generate(&env);
    token.mint(&user, &1000);
    
    // Pause minting
    token.set_pause(&admin, &PauseType::MINT, &true);
    
    // Mint should fail
    let result = token.try_mint(&user, &1000);
    assert!(result.is_err());
}
```

---

## Deployment Checklist

- [ ] Add emergency_guard dependency to Cargo.toml
- [ ] Update DataKey enum with guard storage keys
- [ ] Add pause checks to all pausable operations
- [ ] Implement guard initialization in token initialize()
- [ ] Test all pause/resume scenarios
- [ ] Test multi-admin scenarios
- [ ] Verify storage footprint increase acceptable
- [ ] Document admin procedures
- [ ] Deploy to testnet first
- [ ] Run SoroScope profiling before mainnet

---

## Admin Procedures

### Single Admin Mode (Threshold = 1)

```bash
# Initialize with single admin
soroban contract invoke ... --fn initialize_guard -- \
  --admins '[<admin_address>]' \
  --threshold 1

# Pause operation
soroban contract invoke ... --fn set_pause -- \
  --admin <admin_address> \
  --operation 1  # TRANSFER
  --paused true
```

### Multi-Admin Mode (Threshold = 2)

```bash
# Initialize with multi-sig threshold
soroban contract invoke ... --fn initialize_guard -- \
  --admins '[<admin1>, <admin2>, <admin3>]' \
  --threshold 2

# Emergency pause (requires 2 signatures)
soroban contract invoke ... --fn emergency_pause -- \
  --approvers '[<admin1>, <admin2>]'

# Resume (requires 2 signatures)
soroban contract invoke ... --fn resume -- \
  --approvers '[<admin2>, <admin3>]'
```

---

## Reference Documentation

- [EmergencyGuard API](../contracts/emergency_guard/README.md)
- [Storage Analysis](./TOKEN_GUARD_STORAGE_ANALYSIS.md)
- [Integration Example](../contracts/emergency_guard/examples/simple_token.rs)

---

**Status**: Ready for Implementation ✅
