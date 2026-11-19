//! One scenario per representative operation. Each measures a single contract
//! invocation as the difference in the host's cumulative cost counters, the same
//! technique `storage_heavy`'s tests use.

use perigee_cross_chain_governance::{
    BridgeProof, CrossChainGovMessage, CrossChainGovernance, CrossChainGovernanceClient,
    GovernanceActionType,
};
use perigee_vault_guard::{VaultGuard, VaultGuardClient, OP_DEPOSIT, UNPAUSE_DELAY_LEDGERS};
use perigee_vault_strategy::{VaultStrategy, VaultStrategyClient};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    xdr::ToXdr,
    Address, Env, String,
};

/// Run `f` and print its CPU / memory cost under `contract` / `operation`.
fn measure<T>(env: &Env, contract: &str, operation: &str, f: impl FnOnce() -> T) -> T {
    let budget = env.cost_estimate().budget();
    let cpu_before = budget.cpu_instruction_cost();
    let mem_before = budget.memory_bytes_cost();
    let out = f();
    let budget = env.cost_estimate().budget();
    let cpu = budget.cpu_instruction_cost().saturating_sub(cpu_before);
    let mem = budget.memory_bytes_cost().saturating_sub(mem_before);
    // One machine-readable line per measurement; scripts/resource_budget_guard.py
    // parses exactly this format.
    println!("RESOURCE|{contract}|{operation}|cpu={cpu}|mem={mem}");
    out
}

// ── Vault: vault_guard ───────────────────────────────────────────────────────

#[test]
fn vault_guard_operations() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.sequence_number = 1_000);
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let guardian = Address::generate(&env);
    let id = env.register(VaultGuard, ());
    let client = VaultGuardClient::new(&env, &id);

    measure(&env, "vault_guard", "initialize", || {
        client.initialize(&admin, &guardian)
    });
    measure(&env, "vault_guard", "pause_operations", || {
        client.pause_operations(&admin, &OP_DEPOSIT)
    });
    measure(&env, "vault_guard", "pause", || client.pause(&admin));

    env.ledger()
        .with_mut(|li| li.sequence_number += UNPAUSE_DELAY_LEDGERS);
    measure(&env, "vault_guard", "unpause", || client.unpause(&admin));
}

// ── Strategy: vault_strategy ─────────────────────────────────────────────────

#[test]
fn vault_strategy_operations() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);
    let strategy = Address::generate(&env);
    let id = env.register(VaultStrategy, ());
    let client = VaultStrategyClient::new(&env, &id);

    measure(&env, "vault_strategy", "initialize", || {
        client.initialize(&admin, &treasury)
    });
    measure(&env, "vault_strategy", "register_strategy", || {
        client.register_strategy(&admin, &strategy, &1, &String::from_str(&env, "yield-a"))
    });
    measure(&env, "vault_strategy", "approve_strategy", || {
        client.approve_strategy(&admin, &strategy, &true)
    });
    measure(&env, "vault_strategy", "initialize_vault", || {
        client.initialize_vault(&admin, &vault, &strategy, &1_000)
    });
}

// ── Governance: cross_chain_governance ───────────────────────────────────────

#[test]
fn cross_chain_governance_operations() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let id = env.register(CrossChainGovernance, ());
    let client = CrossChainGovernanceClient::new(&env, &id);

    measure(&env, "cross_chain_governance", "initialize", || {
        client.initialize(&admin, &10u32, &0u64)
    });

    let source = 40u32;
    let sender = admin.to_xdr(&env);
    measure(&env, "cross_chain_governance", "set_trusted_sender", || {
        client.set_trusted_sender(&source, &sender, &true)
    });

    let key = symbol_short!("fee_bps");
    let value = 25i128;
    let message = CrossChainGovMessage {
        source_chain_id: source,
        destination_chain_id: 10,
        sequence: 1,
        sender,
        action_type: GovernanceActionType::ParameterSync,
        payload: client.encode_parameter_payload(&key, &value),
        timestamp: env.ledger().timestamp(),
    };
    let proof: BridgeProof = client.native_proof();
    measure(&env, "cross_chain_governance", "sync_parameter", || {
        client.sync_parameter(&message, &proof, &key, &value)
    });
}
