//! Compile-time `Send`/`Sync` audit for the types that cross async
//! boundaries in the local simulation runner.
//!
//! [`LocalRunner`] is driven from async code: callers keep it inside `tokio`
//! tasks, pass a [`ContractInvocation`] to
//! [`LocalRunner::simulate`](super::local::LocalRunner::simulate), and await
//! the returned future. Every value held across an `.await` must be `Send`,
//! and every shared reference must point at a `Sync` type. When that
//! regresses the compiler reports it far from the cause, as an opaque
//! "future cannot be sent between threads safely" at some unrelated call
//! site.
//!
//! The assertions below move that failure to build time: each one
//! monomorphises a `T: Send + Sync` helper for a concrete type, so stripping
//! `Send`/`Sync` from any audited type breaks `cargo check` / `cargo bench`
//! immediately and names the offending type.
//!
//! # Non-`Send` types and the workaround
//!
//! `soroban_env_host::Host` — and the `soroban_sdk::Env` handle that wraps it
//! — is intentionally `!Send`: the interpreter keeps its object store in `Rc`
//! cells and must stay on a single thread. It is the only non-`Send` type on
//! this path, and it never crosses an `.await`:
//!
//! * `simulate` clones the WASM bytes out of the shared store and then
//!   constructs *and* drives the host inside
//!   [`tokio::task::spawn_blocking`], which takes a `Send` closure and runs
//!   it to completion on the blocking pool.
//! * The host is dropped before the `JoinHandle` resolves, so only the
//!   `Send + Sync` [`SimulationResult`] re-enters the async context.
//!
//! Keeping host construction inside the blocking closure is the whole
//! workaround. No `unsafe impl Send` is used here, and none is required.

use super::local::{ContractInvocation, LocalRunner};
use crate::simulation::{SimulationError, SimulationResult, SorobanResources};
use soroban_env_host::LedgerInfo;

/// Monomorphising this for a concrete `T` only type-checks when
/// `T: Send + Sync`, which is what turns the calls below into compile-time
/// assertions.
fn assert_send_sync<T: Send + Sync>() {}

const _: fn() = || {
    // Requests and results cross the `await` point by value.
    assert_send_sync::<ContractInvocation>();
    assert_send_sync::<SimulationResult>();
    assert_send_sync::<SimulationError>();
    assert_send_sync::<SorobanResources>();
    assert_send_sync::<tokio_util::sync::CancellationToken>();

    // The runner is cloned into and shared across tasks, so it must be both
    // `Send` and `Sync`. Asserting it also implies the `LedgerInfo` it holds
    // behind an `Arc` is `Send + Sync`, which is checked explicitly too.
    assert_send_sync::<LocalRunner>();
    assert_send_sync::<LedgerInfo>();
};

/// Compile-time proof that the future returned by
/// [`LocalRunner::simulate`](super::local::LocalRunner::simulate) is `Send`,
/// i.e. that it can be awaited from a spawned task or held across another
/// `.await`.
///
/// This function is never called — assembling the future is enough to make
/// the compiler check the bound.
#[allow(dead_code)]
fn simulate_future_is_send(runner: &LocalRunner, invocation: &ContractInvocation) {
    fn assert_future_send<F: std::future::Future + Send>(_: F) {}
    assert_future_send(runner.simulate(invocation));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::local::default_ledger_info;

    /// A trait-bound check is necessary but not sufficient: `tokio::spawn`
    /// only accepts a future that is `Send + 'static`, so running the real
    /// future on the runtime proves the property end to end.
    #[tokio::test]
    async fn simulate_future_can_run_on_a_spawned_task() {
        let runner = LocalRunner::new(default_ledger_info());
        let invocation = ContractInvocation::new([0x11; 32], "hello", Vec::new());

        let result = tokio::spawn(async move { runner.simulate(&invocation).await })
            .await
            .expect("spawned simulation task should join");

        let err = result.expect_err("no WASM loaded => LocalUnavailable");
        assert!(matches!(err, SimulationError::LocalUnavailable));
    }

    /// `LocalRunner` is `Sync`, so one instance (and its `Arc`-backed WASM
    /// store) stays usable from several tasks at once.
    #[tokio::test]
    async fn cloned_runner_shares_wasm_store_across_tasks() {
        let runner = LocalRunner::new(default_ledger_info());
        let hash = [0x22; 32];
        runner.load_wasm(hash, vec![0x00, 0x61, 0x73, 0x6d]).await;

        let clone = runner.clone();
        let visible = tokio::spawn(async move { clone.has_wasm(&hash).await })
            .await
            .expect("spawned task should join");

        assert!(visible, "clone must see WASM loaded via the original runner");
    }
}
