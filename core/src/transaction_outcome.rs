//! Distinct response types for simulation vs. transaction-submission outcomes
//! (API-35 / GitHub #796).
//!
//! A contract-call **simulation** is a read-only preview: it never changes
//! ledger state, no matter how confident the result looks. A **submission**
//! only means the transaction was accepted by a node/network, not that it
//! ran. Only a **finalized** outcome reflects an actual, consensus-confirmed
//! ledger change. Collapsing these into one generic "success" response lets
//! a client mistake a preview for a state change, so each stage gets its own
//! explicit type and the allowed transitions between them are enumerable.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The lifecycle stage of a transaction, from preview to confirmed ledger
/// outcome. Each variant corresponds to one of the response types below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    /// Simulated only: a read-only preview. No ledger state was touched and
    /// none will be as a result of this response.
    Simulated,
    /// Accepted into the mempool/network. Not yet included in a ledger.
    Submitted,
    /// Submitted and awaiting ledger close / consensus confirmation.
    Pending,
    /// Confirmed on-chain: the ledger closed with this transaction's result
    /// (success or failure) included.
    Finalized,
}

/// Error returned when a caller attempts to move a transaction between two
/// states that are not a valid lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid transaction status transition: {from:?} -> {to:?}")]
pub struct InvalidTransition {
    pub from: TransactionStatus,
    pub to: TransactionStatus,
}

/// Validates a transaction-status transition against the only lifecycle
/// paths that make sense:
///
/// `Submitted -> Pending -> Finalized`
///
/// `Simulated` is a standalone preview (it never transitions into anything,
/// since simulating never submits a transaction) and `Finalized` is
/// terminal. Any other jump -- skipping a stage, moving backwards, or
/// transitioning out of a terminal state -- is rejected.
pub fn validate_transition(
    from: TransactionStatus,
    to: TransactionStatus,
) -> Result<(), InvalidTransition> {
    use TransactionStatus::*;
    match (from, to) {
        (Submitted, Pending) | (Pending, Finalized) => Ok(()),
        _ => Err(InvalidTransition { from, to }),
    }
}

/// Preview of a contract call produced by simulation. Explicitly **not** a
/// ledger change: `status` is always [`TransactionStatus::Simulated`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SimulationOutcome {
    pub status: TransactionStatus,
    /// Whether the simulated invocation would succeed if submitted as-is.
    pub would_succeed: bool,
    /// Return value of the simulated call, if any (base64 XDR `ScVal`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_value: Option<String>,
    /// Estimated resource/footprint cost of running this call for real.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<crate::simulation::SorobanResources>,
    /// Human-readable reason `would_succeed` is false, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl SimulationOutcome {
    pub fn new(would_succeed: bool) -> Self {
        Self {
            status: TransactionStatus::Simulated,
            would_succeed,
            return_value: None,
            resources: None,
            failure_reason: None,
        }
    }
}

/// Acknowledgement that a transaction was accepted into the mempool/network.
/// This is **not** confirmation that it ran or changed ledger state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmissionAcknowledgement {
    pub status: TransactionStatus,
    pub transaction_hash: String,
}

impl SubmissionAcknowledgement {
    pub fn new(transaction_hash: impl Into<String>) -> Self {
        Self {
            status: TransactionStatus::Submitted,
            transaction_hash: transaction_hash.into(),
        }
    }
}

/// Submitted and awaiting ledger close / consensus. Still no confirmed
/// state change.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PendingConfirmation {
    pub status: TransactionStatus,
    pub transaction_hash: String,
    /// Latest ledger sequence observed while polling for confirmation.
    pub latest_ledger: u64,
}

/// Confirmed, consensus-finalized ledger outcome: the only type that
/// represents an actual state change (or a confirmed failed attempt).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FinalizedOutcome {
    pub status: TransactionStatus,
    pub transaction_hash: String,
    pub success: bool,
    pub ledger: u64,
    /// Return value of the invocation, if it succeeded (base64 XDR `ScVal`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_value: Option<String>,
    /// Failure detail when `success` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions_are_accepted() {
        assert!(validate_transition(TransactionStatus::Submitted, TransactionStatus::Pending).is_ok());
        assert!(validate_transition(TransactionStatus::Pending, TransactionStatus::Finalized).is_ok());
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        // Simulated never transitions into a submission state.
        assert_eq!(
            validate_transition(TransactionStatus::Simulated, TransactionStatus::Submitted),
            Err(InvalidTransition {
                from: TransactionStatus::Simulated,
                to: TransactionStatus::Submitted,
            })
        );
        // Can't skip Pending.
        assert!(validate_transition(TransactionStatus::Submitted, TransactionStatus::Finalized).is_err());
        // Finalized is terminal.
        assert!(validate_transition(TransactionStatus::Finalized, TransactionStatus::Pending).is_err());
        // No backwards transitions.
        assert!(validate_transition(TransactionStatus::Pending, TransactionStatus::Submitted).is_err());
    }
}
