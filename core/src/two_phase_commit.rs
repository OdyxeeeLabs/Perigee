use axum::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::sync::Arc;
use thiserror::Error;

const STALE_TX_MINUTES: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxPhase {
    Prepared,
    Committing,
    Committed,
    Aborting,
    RolledBack,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TwoPhaseTx {
    pub tx_id: String,
    pub phase: TxPhase,
    pub operations: Vec<String>,
    pub completed_ops: Vec<String>,
    pub prepared_at: Option<DateTime<Utc>>,
}

impl TwoPhaseTx {
    pub fn new(tx_id: String, operations: Vec<String>) -> Self {
        Self {
            tx_id,
            phase: TxPhase::Prepared,
            operations,
            completed_ops: Vec::new(),
            prepared_at: Some(Utc::now()),
        }
    }

    pub fn prepare(&mut self) -> bool {
        if self.phase == TxPhase::Committed {
            return false;
        }
        if !matches!(
            self.phase,
            TxPhase::Prepared | TxPhase::Failed | TxPhase::RolledBack
        ) {
            return false;
        }
        if let Err(message) = self.validate_definition() {
            self.mark_failed(message);
            return false;
        }
        self.phase = TxPhase::Prepared;
        self.completed_ops.clear();
        self.prepared_at = Some(Utc::now());
        true
    }

    pub fn prepare_at(&mut self, now: DateTime<Utc>) -> bool {
        if self.prepare() {
            self.prepared_at = Some(now);
            true
        } else {
            false
        }
    }

    pub fn commit_op(&mut self, op: &str) -> Result<(), String> {
        self.record_commit_op(op)?;
        if self.completed_ops.len() == self.operations.len() {
            self.phase = TxPhase::Committed;
        }
        Ok(())
    }

    fn record_commit_op(&mut self, op: &str) -> Result<(), String> {
        if self.phase != TxPhase::Prepared && self.phase != TxPhase::Committing {
            return Err(format!("Cannot commit in phase {:?}", self.phase));
        }
        if self.prepared_at.is_none() && !self.prepare() {
            return Err("Transaction is not prepared".to_string());
        }
        self.validate_definition()?;
        if !self.operations.iter().any(|operation| operation == op) {
            return Err(format!("Unknown operation: {}", op));
        }
        if self.completed_ops.iter().any(|operation| operation == op) {
            return Err(format!("Operation already committed: {}", op));
        }
        self.phase = TxPhase::Committing;
        self.completed_ops.push(op.to_string());
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), String> {
        if self.operations.is_empty() {
            return Err("Cannot commit a transaction without operations".to_string());
        }
        if self.phase == TxPhase::Committed {
            return Ok(());
        }
        if self.phase != TxPhase::Prepared && self.phase != TxPhase::Committing {
            return Err(format!("Cannot commit in phase {:?}", self.phase));
        }
        let operation_set = self
            .operations
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let completed_set = self
            .completed_ops
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if operation_set != completed_set {
            return Err("Cannot commit before every operation is committed".to_string());
        }
        self.phase = TxPhase::Committed;
        Ok(())
    }

    pub fn is_fully_committed(&self) -> bool {
        self.phase == TxPhase::Committed
            && !self.operations.is_empty()
            && self.completed_ops.len() == self.operations.len()
            && self
                .operations
                .iter()
                .all(|operation| self.completed_ops.contains(operation))
    }

    pub fn abort(&mut self) -> Result<(), String> {
        if self.phase == TxPhase::RolledBack {
            return Ok(());
        }
        self.phase = TxPhase::Aborting;
        self.completed_ops.clear();
        self.prepared_at = None;
        self.phase = TxPhase::RolledBack;
        Ok(())
    }

    pub fn rollback(&mut self) {
        let _ = self.abort();
    }

    pub fn mark_cleanup_failed(&mut self) {
        self.phase = TxPhase::Aborting;
        self.completed_ops.clear();
        if self.prepared_at.is_none() {
            self.prepared_at = Some(Utc::now());
        }
    }

    pub fn cleanup(&mut self) {
        if matches!(
            self.phase,
            TxPhase::Prepared | TxPhase::Committing | TxPhase::Aborting | TxPhase::Failed
        ) {
            self.completed_ops.clear();
            self.prepared_at = None;
            self.phase = TxPhase::RolledBack;
        }
    }

    pub fn can_retry(&self) -> bool {
        matches!(self.phase, TxPhase::Failed | TxPhase::RolledBack)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, TxPhase::Committed | TxPhase::RolledBack)
    }

    pub fn is_stale(&self) -> bool {
        match self.prepared_at {
            Some(at) => Utc::now() - at > chrono::Duration::minutes(STALE_TX_MINUTES),
            None => false,
        }
    }

    fn validate_definition(&self) -> Result<(), String> {
        if self.tx_id.trim().is_empty() {
            return Err("transaction id must not be empty".to_string());
        }
        if self.operations.is_empty() {
            return Err("transaction must contain at least one operation".to_string());
        }
        let mut unique = HashSet::new();
        for operation in &self.operations {
            if operation.trim().is_empty() {
                return Err("transaction operation must not be empty".to_string());
            }
            if !unique.insert(operation) {
                return Err(format!("duplicate transaction operation: {}", operation));
            }
        }
        Ok(())
    }

    fn mark_failed(&mut self, _message: String) {
        self.phase = TxPhase::Failed;
        self.completed_ops.clear();
        self.prepared_at = None;
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub tx_id: String,
    pub action: String,
}

pub struct Reconciler {
    pending_txs: Vec<TwoPhaseTx>,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl Reconciler {
    pub fn new() -> Self {
        Self {
            pending_txs: Vec::new(),
        }
    }

    pub fn register(&mut self, tx: TwoPhaseTx) {
        if let Some(existing) = self
            .pending_txs
            .iter_mut()
            .find(|pending| pending.tx_id == tx.tx_id)
        {
            *existing = tx;
        } else {
            self.pending_txs.push(tx);
        }
    }

    pub fn reconcile(&mut self) -> Vec<ReconciliationResult> {
        let mut results = Vec::new();
        for tx in &mut self.pending_txs {
            if tx.is_fully_committed() {
                results.push(ReconciliationResult {
                    tx_id: tx.tx_id.clone(),
                    success: true,
                    retried: false,
                });
            } else if tx.phase == TxPhase::Aborting {
                results.push(ReconciliationResult {
                    tx_id: tx.tx_id.clone(),
                    success: false,
                    retried: false,
                });
            } else if tx.is_stale() {
                let _ = tx.abort();
                results.push(ReconciliationResult {
                    tx_id: tx.tx_id.clone(),
                    success: false,
                    retried: false,
                });
            } else if tx.can_retry() {
                let retried = tx.prepare();
                if !retried {
                    let _ = tx.abort();
                }
                results.push(ReconciliationResult {
                    tx_id: tx.tx_id.clone(),
                    success: false,
                    retried,
                });
            } else {
                results.push(ReconciliationResult {
                    tx_id: tx.tx_id.clone(),
                    success: false,
                    retried: false,
                });
            }
        }
        self.pending_txs.retain(|tx| !tx.is_terminal());
        results
    }

    pub fn recover_orphaned(&mut self) -> Vec<RecoveryResult> {
        let mut results = Vec::new();
        for tx in &mut self.pending_txs {
            if tx.phase == TxPhase::Aborting {
                continue;
            }
            if matches!(tx.phase, TxPhase::Prepared | TxPhase::Committing) && tx.is_stale() {
                let _ = tx.abort();
                results.push(RecoveryResult {
                    tx_id: tx.tx_id.clone(),
                    action: "rolled_back_stale".to_string(),
                });
            }
        }
        self.pending_txs.retain(|tx| !tx.is_terminal());
        results
    }

    pub fn cleanup_terminal(&mut self) -> usize {
        let before = self.pending_txs.len();
        self.pending_txs.retain(|tx| !tx.is_terminal());
        before - self.pending_txs.len()
    }

    pub fn pending_count(&self) -> usize {
        self.pending_txs.len()
    }

    pub fn failed_count(&self) -> usize {
        self.pending_txs
            .iter()
            .filter(|tx| tx.phase == TxPhase::Failed)
            .count()
    }

    pub fn take_pending(&mut self) -> Vec<TwoPhaseTx> {
        std::mem::take(&mut self.pending_txs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationResult {
    pub tx_id: String,
    pub success: bool,
    pub retried: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransactionError {
    #[error("invalid transaction: {0}")]
    Invalid(String),
    #[error("prepare failed for {operation}: {message}; cleanup errors: {cleanup_errors:?}")]
    Prepare {
        operation: String,
        message: String,
        cleanup_errors: Vec<String>,
    },
    #[error("commit failed for {operation}: {message}; cleanup errors: {cleanup_errors:?}")]
    Commit {
        operation: String,
        message: String,
        cleanup_errors: Vec<String>,
    },
    #[error("transaction cleanup failed: {0}")]
    Cleanup(String),
}

#[async_trait]
pub trait TransactionParticipant: Send + Sync {
    async fn prepare(&self, tx_id: &str, operation: &str) -> Result<(), String>;
    async fn commit(&self, tx_id: &str, operation: &str) -> Result<(), String>;
    async fn abort(&self, tx_id: &str, operation: &str) -> Result<(), String>;
    fn supports(&self, _operation: &str) -> bool {
        true
    }
}

#[derive(Default)]
pub struct TwoPhaseCoordinator;

impl TwoPhaseCoordinator {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(
        &self,
        tx: &mut TwoPhaseTx,
        participants: &[Arc<dyn TransactionParticipant>],
    ) -> Result<(), TransactionError> {
        if participants.is_empty() {
            return Err(TransactionError::Invalid(
                "at least one transaction participant is required".to_string(),
            ));
        }
        if !tx.prepare() {
            return Err(TransactionError::Invalid(
                "transaction could not be prepared".to_string(),
            ));
        }
        let supported_operations = participants
            .iter()
            .flat_map(|participant| {
                tx.operations
                    .iter()
                    .filter(|operation| participant.supports(operation))
                    .cloned()
            })
            .collect::<HashSet<_>>();
        if supported_operations.len() != tx.operations.len() {
            let _ = tx.abort();
            return Err(TransactionError::Invalid(
                "every transaction operation must have a supporting participant".to_string(),
            ));
        }

        let mut prepared = Vec::new();
        for participant in participants {
            for operation in &tx.operations {
                if !participant.supports(operation) {
                    continue;
                }
                let entry = (Arc::clone(participant), operation.clone());
                prepared.push(entry);
                if let Err(message) = participant.prepare(&tx.tx_id, operation).await {
                    let cleanup_errors = Self::finish_abort(tx, &prepared).await;
                    return Err(TransactionError::Prepare {
                        operation: operation.clone(),
                        message,
                        cleanup_errors,
                    });
                }
            }
        }

        if prepared.is_empty() {
            let _ = tx.abort();
            return Err(TransactionError::Invalid(
                "no participant supports a transaction operation".to_string(),
            ));
        }

        let mut committed_operations = HashSet::new();
        for (participant, operation) in &prepared {
            if let Err(message) = participant.commit(&tx.tx_id, operation).await {
                let cleanup_errors = Self::finish_abort(tx, &prepared).await;
                return Err(TransactionError::Commit {
                    operation: operation.clone(),
                    message,
                    cleanup_errors,
                });
            }
            if committed_operations.insert(operation.clone()) {
                if let Err(message) = tx.record_commit_op(operation) {
                    let cleanup_errors = Self::finish_abort(tx, &prepared).await;
                    return Err(TransactionError::Commit {
                        operation: operation.clone(),
                        message,
                        cleanup_errors,
                    });
                }
            }
        }

        if let Err(message) = tx.commit() {
            let cleanup_errors = Self::finish_abort(tx, &prepared).await;
            return Err(TransactionError::Commit {
                operation: "all".to_string(),
                message,
                cleanup_errors,
            });
        }
        Ok(())
    }

    pub async fn abort(
        &self,
        tx: &mut TwoPhaseTx,
        participants: &[Arc<dyn TransactionParticipant>],
    ) -> Result<(), TransactionError> {
        let operations = tx.operations.clone();
        let entries = participants
            .iter()
            .flat_map(|participant| {
                operations.iter().filter_map(|operation| {
                    participant
                        .supports(operation)
                        .then(|| (Arc::clone(participant), operation.clone()))
                })
            })
            .collect::<Vec<_>>();
        let cleanup_errors = Self::finish_abort(tx, &entries).await;
        if cleanup_errors.is_empty() {
            Ok(())
        } else {
            Err(TransactionError::Cleanup(cleanup_errors.join("; ")))
        }
    }

    pub async fn recover(
        &self,
        tx: &mut TwoPhaseTx,
        participants: &[Arc<dyn TransactionParticipant>],
    ) -> Result<RecoveryResult, TransactionError> {
        if tx.is_fully_committed() {
            return Ok(RecoveryResult {
                tx_id: tx.tx_id.clone(),
                action: "retained_committed".to_string(),
            });
        }
        if tx.phase == TxPhase::RolledBack {
            return Ok(RecoveryResult {
                tx_id: tx.tx_id.clone(),
                action: "retained_aborted".to_string(),
            });
        }
        if tx.phase == TxPhase::Failed {
            if tx.prepare() {
                return Ok(RecoveryResult {
                    tx_id: tx.tx_id.clone(),
                    action: "reprepared".to_string(),
                });
            }
            let _ = tx.abort();
            return Ok(RecoveryResult {
                tx_id: tx.tx_id.clone(),
                action: "rolled_back_invalid".to_string(),
            });
        }
        if tx.phase != TxPhase::Aborting && !tx.is_stale() {
            return Ok(RecoveryResult {
                tx_id: tx.tx_id.clone(),
                action: "retained_fresh".to_string(),
            });
        }
        let entries = participants
            .iter()
            .flat_map(|participant| {
                tx.operations.iter().filter_map(|operation| {
                    participant
                        .supports(operation)
                        .then(|| (Arc::clone(participant), operation.clone()))
                })
            })
            .collect::<Vec<_>>();
        let cleanup_errors = Self::finish_abort(tx, &entries).await;
        if cleanup_errors.is_empty() {
            Ok(RecoveryResult {
                tx_id: tx.tx_id.clone(),
                action: "rolled_back_stale".to_string(),
            })
        } else {
            Err(TransactionError::Cleanup(cleanup_errors.join("; ")))
        }
    }

    async fn finish_abort(
        tx: &mut TwoPhaseTx,
        entries: &[(Arc<dyn TransactionParticipant>, String)],
    ) -> Vec<String> {
        let cleanup_errors = Self::abort_entries(&tx.tx_id, entries).await;
        if cleanup_errors.is_empty() {
            let _ = tx.abort();
        } else {
            tx.mark_cleanup_failed();
        }
        cleanup_errors
    }

    async fn abort_entries(
        tx_id: &str,
        entries: &[(Arc<dyn TransactionParticipant>, String)],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        for (participant, operation) in entries.iter().rev() {
            if let Err(error) = participant.abort(tx_id, operation).await {
                errors.push(format!("{}: {}", operation, error));
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_commit_and_abort_are_terminal() {
        let mut tx = TwoPhaseTx::new("tx1".to_string(), vec!["a".to_string(), "b".to_string()]);
        assert!(tx.prepare());
        assert!(tx.commit_op("a").is_ok());
        assert!(!tx.is_fully_committed());
        assert!(tx.commit_op("b").is_ok());
        assert!(tx.is_fully_committed());

        let mut aborted = TwoPhaseTx::new("tx2".to_string(), vec!["a".to_string()]);
        assert!(aborted.prepare());
        assert!(aborted.abort().is_ok());
        assert!(aborted.can_retry());
        assert!(aborted.completed_ops.is_empty());
    }

    #[test]
    fn stale_recovery_cleans_pending_transactions() {
        let mut reconciler = Reconciler::new();
        let mut tx = TwoPhaseTx::new("stale".to_string(), vec!["a".to_string()]);
        tx.prepared_at = Some(Utc::now() - chrono::Duration::minutes(60));
        reconciler.register(tx);
        let result = reconciler.recover_orphaned();
        assert_eq!(result[0].action, "rolled_back_stale");
        assert_eq!(reconciler.pending_count(), 0);
    }
}
