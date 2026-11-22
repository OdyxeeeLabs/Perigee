#![no_std]

//! Deployment-time bootstrapping validation for Perigee Soroban contracts.
//!
//! A contract that has just been deployed and initialized is in its most
//! fragile state: nothing has exercised it, and a mistake in `initialize`
//! (wrong defaults, missing admin registration, an unreachable entry point)
//! is usually only discovered by the first user who calls it.
//!
//! This crate gives every contract a small, uniform way to validate four
//! dimensions of a fresh deployment:
//!
//! 1. **Entry points** — are the expected entry points registered and
//!    callable on this contract address?
//! 2. **Storage defaults** — are the storage keys written and set to valid
//!    defaults?
//! 3. **Admin access** — is the admin surface registered, reachable, and
//!    consistent (owner is an admin, threshold is satisfiable)?
//! 4. **Invariants** — do the critical invariants hold in the validated state?
//!
//! Contracts collect the answers with [`Bootstrap`] and expose the result as
//! a [`BootstrapReport`] through a `self_test` entry point, plus a
//! `bootstrap_ok` convenience gate. The deployment pipeline
//! (`scripts/deploy_testnet.sh`) fails the deployment when a contract reports
//! a failing bootstrap.
//!
//! ## Why entry points are checked outside the contract
//!
//! The Soroban host rejects a contract invoking itself: an inner
//! `try_invoke_contract` on the contract's own address aborts with
//! [`InvokeError::Abort`] regardless of whether the entry point exists (see
//! `test::self_invocation_is_rejected_by_the_host`). Contracts therefore
//! cannot introspect or probe their own interface on-chain, and the
//! entry-point dimension is verified where it can actually be observed:
//!
//! * [`probe_entry_point`] is the host-side probe used by tests and by the
//!   deployment tooling against the deployed contract.
//! * [`Bootstrap::entry_point`] records the outcome of such a probe for the
//!   caller assembling the report, so the dimension is still reported
//!   explicitly instead of being silently dropped.
//!
//! Only side-effect free entry points can be probed: proving that a mutating
//! entry point exists must not require a state change or an authorization, so
//! the probe covers the query surface and the deployment script logs which
//! entry points it verified.
//!
//! ## Example
//!
//! ```ignore
//! use perigee_bootstrap::{Bootstrap, BootstrapReport, CheckKind};
//!
//! pub fn self_test(e: Env, ) -> BootstrapReport {
//!     let mut b = Bootstrap::new(&e, "token");
//!     b.storage_default("config_written", e.storage().instance().has(&DataKey::Config));
//!     b.admin_access("owner_registered", !admins.is_empty());
//!     b.invariant("supply_non_negative", total_supply >= 0);
//!     b.report()
//! }
//! ```

use soroban_sdk::{contracterror, contracttype, Address, Env, InvokeError, Symbol, Val, Vec};

/// Errors returned by the bootstrap validation helpers.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum BootstrapError {
    /// At least one bootstrap check did not hold.
    Failed = 1,
}

/// The aspect of the deployment a check covers.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckKind {
    /// A contract entry point is registered and callable.
    EntryPoint,
    /// A storage key is written and holds a valid default.
    StorageDefault,
    /// The admin surface is registered and reachable.
    AdminAccess,
    /// A critical invariant holds in the validated state.
    Invariant,
}

/// The outcome of a single check.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckStatus {
    /// The check held.
    Passed,
    /// The check did not hold; the deployment is not valid.
    Failed,
    /// The check does not apply to this contract. Contracts that have no
    /// admin surface still report the dimension explicitly rather than
    /// silently omitting it.
    NotApplicable,
}

/// A single bootstrap check with its outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapCheck {
    /// Short, stable name of the check (at most 32 characters).
    pub name: Symbol,
    /// Which dimension of the deployment the check covers.
    pub kind: CheckKind,
    /// Outcome of the check.
    pub status: CheckStatus,
}

/// The result of running a contract's bootstrapping self-test.
///
/// A deployment is valid only when `passed` is `true`, which means no check
/// reported [`CheckStatus::Failed`]. [`CheckStatus::NotApplicable`] entries
/// are reported for transparency but never fail a deployment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapReport {
    /// Name of the contract that was validated.
    pub contract: Symbol,
    /// `true` when every applicable check passed.
    pub passed: bool,
    /// Number of checks that passed.
    pub passed_count: u32,
    /// Number of checks that failed. Zero for a valid deployment.
    pub failed_count: u32,
    /// Every check that was run, in the order it was recorded.
    pub checks: Vec<BootstrapCheck>,
}

/// Invokes `name` on `contract` with `args` and reports whether the entry
/// point exists and ran to completion.
///
/// This is a host-side probe: call it from a test, from deployment tooling, or
/// from another contract against a *different* contract address. It cannot be
/// used by a contract against itself, because the host aborts self-invocations
/// before the entry point is even resolved.
///
/// Returns `true` when the call succeeded and also when the callee returned a
/// *contract* error: a returned error proves the function was found and
/// executed. It returns `false` when the host aborted the call, which is what
/// happens for an entry point that is not exported by the contract's WASM, for
/// arguments that do not decode, or for a trap inside the callee.
///
/// `name` must be a valid Soroban symbol (at most 32 characters); passing a
/// longer name panics, exactly as `Symbol::new` does. Only side-effect free
/// entry points should be probed.
pub fn probe_entry_point(env: &Env, contract: &Address, name: &str, args: Vec<Val>) -> bool {
    let func = Symbol::new(env, name);
    match env.try_invoke_contract::<Val, InvokeError>(contract, &func, args) {
        // The function ran and returned a value, which may itself be an error
        // value for entry points whose result type is a `Result`.
        Ok(_) => true,
        // The function ran and returned a contract error.
        Err(Ok(InvokeError::Contract(_))) => true,
        // Host-level abort: missing entry point, undecodable arguments, a
        // trap inside the callee, or an illegal self-invocation.
        Err(Ok(InvokeError::Abort)) | Err(Err(_)) => false,
    }
}

/// Probes a zero-argument entry point of `contract`.
pub fn probe_no_arg_entry_point(env: &Env, contract: &Address, name: &str) -> bool {
    probe_entry_point(env, contract, name, Vec::new(env))
}

/// Collects bootstrapping checks and produces a [`BootstrapReport`].
///
/// Recording is cheap and infallible; nothing fails until the report is
/// inspected or consumed by [`Bootstrap::require_ok`].
pub struct Bootstrap<'a> {
    env: &'a Env,
    contract: Symbol,
    checks: Vec<BootstrapCheck>,
}

impl<'a> Bootstrap<'a> {
    /// Starts a new validation run for the contract named `contract`.
    pub fn new(env: &'a Env, contract: &str) -> Self {
        Self {
            env,
            contract: Symbol::new(env, contract),
            checks: Vec::new(env),
        }
    }

    fn record(&mut self, name: &str, kind: CheckKind, status: CheckStatus) {
        self.checks.push_back(BootstrapCheck {
            name: Symbol::new(self.env, name),
            kind,
            status,
        });
    }

    /// Records an unconditional pass.
    pub fn pass(&mut self, name: &str, kind: CheckKind) -> &mut Self {
        self.record(name, kind, CheckStatus::Passed);
        self
    }

    /// Records an unconditional failure.
    pub fn fail(&mut self, name: &str, kind: CheckKind) -> &mut Self {
        self.record(name, kind, CheckStatus::Failed);
        self
    }

    /// Records a check that does not apply to this contract.
    pub fn not_applicable(&mut self, name: &str, kind: CheckKind) -> &mut Self {
        self.record(name, kind, CheckStatus::NotApplicable);
        self
    }

    /// Records `holds` as a pass/fail check.
    pub fn require(&mut self, name: &str, kind: CheckKind, holds: bool) -> &mut Self {
        self.record(
            name,
            kind,
            if holds {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            },
        );
        self
    }

    /// Records a storage-default check.
    pub fn storage_default(&mut self, name: &str, holds: bool) -> &mut Self {
        self.require(name, CheckKind::StorageDefault, holds)
    }

    /// Records an admin-access check.
    pub fn admin_access(&mut self, name: &str, holds: bool) -> &mut Self {
        self.require(name, CheckKind::AdminAccess, holds)
    }

    /// Records an invariant check.
    pub fn invariant(&mut self, name: &str, holds: bool) -> &mut Self {
        self.require(name, CheckKind::Invariant, holds)
    }

    /// Records the outcome of an entry-point probe performed by the caller.
    ///
    /// The probe itself must be run by a host-side caller (see
    /// [`probe_entry_point`]); a contract cannot probe its own interface.
    pub fn entry_point(&mut self, name: &str, registered: bool) -> &mut Self {
        self.require(name, CheckKind::EntryPoint, registered)
    }

    /// Returns `true` when no check failed.
    pub fn passed(&self) -> bool {
        let mut ok = true;
        for i in 0..self.checks.len() {
            if self.checks.get(i).unwrap().status == CheckStatus::Failed {
                ok = false;
            }
        }
        ok
    }

    /// Names of the checks that failed, in recording order.
    pub fn failed_checks(&self) -> Vec<Symbol> {
        let mut failed = Vec::new(self.env);
        for i in 0..self.checks.len() {
            let check = self.checks.get(i).unwrap();
            if check.status == CheckStatus::Failed {
                failed.push_back(check.name);
            }
        }
        failed
    }

    /// Builds the report for the run so far.
    pub fn report(&self) -> BootstrapReport {
        let mut passed_count = 0u32;
        let mut failed_count = 0u32;
        for i in 0..self.checks.len() {
            match self.checks.get(i).unwrap().status {
                CheckStatus::Passed => passed_count += 1,
                CheckStatus::Failed => failed_count += 1,
                CheckStatus::NotApplicable => {}
            }
        }

        BootstrapReport {
            contract: self.contract.clone(),
            passed: failed_count == 0,
            passed_count,
            failed_count,
            checks: self.checks.clone(),
        }
    }

    /// Builds the report, returning [`BootstrapError::Failed`] if any check
    /// failed. Use this at the end of `initialize` so a contract can never
    /// finish bootstrapping into an invalid state.
    pub fn require_ok(&self) -> Result<BootstrapReport, BootstrapError> {
        let report = self.report();
        if report.passed {
            Ok(report)
        } else {
            Err(BootstrapError::Failed)
        }
    }
}

#[cfg(test)]
mod test;
