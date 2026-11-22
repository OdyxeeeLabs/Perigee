//! Unit tests for the bootstrap validation harness.
//!
//! The fixture contract mirrors how a real Perigee contract uses the harness:
//! it records storage/admin/invariant checks and returns the resulting report.
//! Entry-point checks are recorded from a host-side probe, which is the only
//! place they can be observed.

use crate::{
    probe_entry_point, probe_no_arg_entry_point, Bootstrap, BootstrapError, BootstrapReport,
    CheckKind, CheckStatus,
};
use soroban_sdk::{contract, contracterror, contractimpl, Env, IntoVal, Val, Vec};

#[contract]
pub struct BootstrapFixture;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FixtureError {
    Boom = 1,
}

#[contractimpl]
impl BootstrapFixture {
    pub fn ping(_e: Env) -> u32 {
        7
    }

    pub fn echo(_e: Env, value: u32) -> u32 {
        value
    }

    pub fn boom(_e: Env) -> Result<u32, FixtureError> {
        Err(FixtureError::Boom)
    }

    /// Diagnostic used to document the host's self-invocation behaviour.
    /// 0 = ok, 1 = contract error, 2 = abort, 3 = invoke error.
    pub fn self_call_code(e: Env) -> u32 {
        use soroban_sdk::{InvokeError, Symbol};
        match e.try_invoke_contract::<Val, InvokeError>(
            &e.current_contract_address(),
            &Symbol::new(&e, "ping"),
            Vec::new(&e),
        ) {
            Ok(_) => 0,
            Err(Ok(InvokeError::Contract(_))) => 1,
            Err(Ok(InvokeError::Abort)) => 2,
            Err(Err(_)) => 3,
        }
    }

    /// Runs the state dimension of the harness from inside a contract.
    pub fn self_report(e: Env) -> BootstrapReport {
        let mut bootstrap = Bootstrap::new(&e, "fixture");
        bootstrap.storage_default("config_written", true);
        bootstrap.admin_access("admins_registered", false);
        bootstrap.invariant("supply_non_negative", true);
        bootstrap.not_applicable("fee_surface", CheckKind::AdminAccess);
        bootstrap.report()
    }
}

fn fixture(e: &Env) -> soroban_sdk::Address {
    e.register(BootstrapFixture, ())
}

#[test]
fn probe_finds_registered_entry_point() {
    let e = Env::default();
    let id = fixture(&e);

    assert!(probe_no_arg_entry_point(&e, &id, "ping"));
}

#[test]
fn probe_reports_missing_entry_point() {
    let e = Env::default();
    let id = fixture(&e);

    assert!(!probe_no_arg_entry_point(&e, &id, "not_an_entry_point"));
}

#[test]
fn probe_treats_contract_error_as_registered() {
    let e = Env::default();
    let id = fixture(&e);

    // `boom` returns a contract error: the entry point exists and ran, so the
    // probe must still consider it registered.
    assert!(probe_no_arg_entry_point(&e, &id, "boom"));
}

#[test]
fn probe_accepts_arguments() {
    let e = Env::default();
    let id = fixture(&e);

    let good: Vec<Val> = soroban_sdk::vec![&e, 41u32.into_val(&e)];
    assert!(probe_entry_point(&e, &id, "echo", good));

    // Arguments that do not decode abort the call at the host, which the probe
    // reports as a failure rather than propagating.
    let bad: Vec<Val> = soroban_sdk::vec![&e, 41u32.into_val(&e), 42u32.into_val(&e)];
    assert!(!probe_entry_point(&e, &id, "echo", bad));
}

/// A contract cannot probe its own interface: the host aborts the inner call
/// before resolving the entry point, whichever function is targeted. Any
/// on-chain `self_test` must therefore validate only state, and entry points
/// are verified by host-side callers instead.
#[test]
fn self_invocation_is_rejected_by_the_host() {
    let e = Env::default();
    let id = fixture(&e);
    let client = BootstrapFixtureClient::new(&e, &id);

    assert_eq!(client.self_call_code(), 2);
}

#[test]
fn report_passes_only_when_no_check_fails() {
    let e = Env::default();
    let id = fixture(&e);
    let client = BootstrapFixtureClient::new(&e, &id);

    let report = client.self_report();

    assert!(!report.passed);
    assert_eq!(report.contract, soroban_sdk::Symbol::new(&e, "fixture"));
    // config_written, supply_non_negative
    assert_eq!(report.passed_count, 2);
    // admins_registered
    assert_eq!(report.failed_count, 1);
    // config_written, admins_registered, supply_non_negative, fee_surface
    assert_eq!(report.checks.len(), 4);
}

#[test]
fn entry_point_checks_are_recorded_per_dimension() {
    let e = Env::default();
    let id = fixture(&e);

    let mut bootstrap = Bootstrap::new(&e, "fixture");
    bootstrap.entry_point("ping", probe_no_arg_entry_point(&e, &id, "ping"));
    bootstrap.entry_point(
        "not_an_entry_point",
        probe_no_arg_entry_point(&e, &id, "not_an_entry_point"),
    );
    let report = bootstrap.report();

    assert!(!report.passed);
    assert_eq!(report.failed_count, 1);
    assert_eq!(report.checks.get(0).unwrap().kind, CheckKind::EntryPoint);
    assert_eq!(report.checks.get(0).unwrap().status, CheckStatus::Passed);
    assert_eq!(
        bootstrap.failed_checks().get(0).unwrap(),
        soroban_sdk::Symbol::new(&e, "not_an_entry_point")
    );
}

#[test]
fn not_applicable_checks_never_fail_a_report() {
    let e = Env::default();
    let mut bootstrap = Bootstrap::new(&e, "fixture");
    bootstrap.not_applicable("admin_surface", CheckKind::AdminAccess);
    bootstrap.pass("config_written", CheckKind::StorageDefault);

    let report = bootstrap.report();

    assert!(report.passed);
    assert_eq!(report.failed_count, 0);
    assert_eq!(report.passed_count, 1);
    assert!(bootstrap.require_ok().is_ok());
}

#[test]
fn require_ok_reports_failed_deployments() {
    let e = Env::default();
    let mut bootstrap = Bootstrap::new(&e, "fixture");
    bootstrap.invariant("threshold_satisfiable", false);

    assert_eq!(bootstrap.require_ok(), Err(BootstrapError::Failed));
    assert!(!bootstrap.passed());
    assert_eq!(bootstrap.failed_checks().len(), 1);
}

#[test]
fn failed_checks_are_listed_in_recording_order() {
    let e = Env::default();
    let mut bootstrap = Bootstrap::new(&e, "fixture");
    bootstrap.invariant("first", false);
    bootstrap.pass("second", CheckKind::Invariant);
    bootstrap.admin_access("third", false);

    let failed = bootstrap.failed_checks();

    assert_eq!(failed.len(), 2);
    assert_eq!(failed.get(0).unwrap(), soroban_sdk::Symbol::new(&e, "first"));
    assert_eq!(failed.get(1).unwrap(), soroban_sdk::Symbol::new(&e, "third"));
}
