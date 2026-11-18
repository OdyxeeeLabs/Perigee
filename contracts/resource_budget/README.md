# Contract resource budget (#886)

A unit test can pass while the operation it covers quietly becomes too expensive
to run on-chain. This crate measures the **CPU instructions** and **memory bytes**
the Soroban host charges for representative operations and compares them with a
committed baseline.

| Area | Contract | Operations measured |
| --- | --- | --- |
| Vault | `vault_guard` | `initialize`, `pause_operations`, `pause`, `unpause` |
| Strategy | `vault_strategy` | `initialize`, `register_strategy`, `approve_strategy`, `initialize_vault` |
| Governance | `cross_chain_governance` | `initialize`, `set_trusted_sender`, `sync_parameter` |

## Run it

Toolchain: stable Rust (the workspace builds with `dtolnay/rust-toolchain@stable`
in CI) and `soroban-sdk` 22.x with the `testutils` feature; Python 3.9+ for the
guard script.

```bash
# Raw report: one line per operation
cargo test -p perigee-resource-budget -- --nocapture --test-threads=1

# Report plus comparison with the baseline (what CI runs)
python3 scripts/resource_budget_guard.py --tolerance 5
```

Output lines identify the contract and operation:

```text
RESOURCE|vault_guard|pause|cpu=...|mem=...
```

`--test-threads=1` only keeps the printed lines grouped; the numbers do not
depend on it.

## Why this is not flaky

The figures come from the host's deterministic cost model, not from a wall
clock, so the same code produces the same numbers on any machine. There is no
timing in the comparison. The 5% default tolerance exists to absorb a
`soroban-sdk` bump, not noise.

## Baseline

`contracts/resource_budget/baseline.json` holds the accepted numbers.

- An operation with no baseline entry is reported as `NEW` and does not fail.
- When you change a contract on purpose and the cost legitimately moves, refresh
  the baseline in the same PR:

  ```bash
  python3 scripts/resource_budget_guard.py --update
  ```

- A deliberate regression shows up as a `REGRESSION` row in the job summary and
  fails the **Contract resource budget** workflow.

Adding an operation: add a `measure(&env, "<contract>", "<operation>", || ...)`
call in `src/report.rs`, then refresh the baseline.
