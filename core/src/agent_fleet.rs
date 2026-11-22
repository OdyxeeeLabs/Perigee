pub use crate::agent_health::{
    deep_health_handler, deep_liveness, probe_dependency, probe_dependency_with_retry,
    DeepHealthReport, DependencyKind, DependencyStatus, HealthAttestationService, HealthStatus,
    RetryPolicy,
};
pub use crate::agent_identity::{AgentIdentity, IdentityError};
pub use crate::failover::{FailoverEvent, FailoverManager};
pub use crate::reputation::ReputationRecord;

use crate::input_sanitization::sanitize_text;
use axum::async_trait;
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FleetError {
    #[error("agent id must not be empty")]
    EmptyAgentId,
    #[error("agent id contains invalid input")]
    InvalidAgentId,
    #[error("agent already exists")]
    DuplicateAgent,
    #[error("agent not found")]
    UnknownAgent,
    #[error("agent is revoked or inactive")]
    AgentUnavailable,
    #[error("no healthy agent is available")]
    NoHealthyAgent,
    #[error("agent {agent_id} failed: {message}")]
    Execution { agent_id: String, message: String },
}

pub trait AgentDirectory: Send + Sync {
    fn register(&self, identity: AgentIdentity) -> Result<(), FleetError>;
    fn remove(&self, agent_id: &str) -> bool;
    fn get(&self, agent_id: &str) -> Option<AgentIdentity>;
    fn list(&self) -> Vec<AgentIdentity>;
    fn set_active(&self, agent_id: &str, active: bool) -> Result<(), FleetError>;
}

pub trait HealthRegistry: Send + Sync {
    fn register(&self, agent_id: &str, threshold: usize);
    fn record_self_report(&self, agent_id: &str);
    fn record_peer_attestation(&self, target: &str, peer: &str);
    fn is_healthy(&self, agent_id: &str) -> Option<bool>;
    fn remove(&self, _agent_id: &str) {}
}

pub trait FailoverCoordinator: Send + Sync {
    fn can_execute(&self, vault_id: &str) -> bool;
    fn record_failure(&self, agent_id: &str) -> bool;
    fn record_success(&self, agent_id: &str);
    fn initiate_failover(&self, vault_id: &str, replacement_agent_id: &str);
    fn confirm_reauth(&self, vault_id: &str) -> bool;
    fn remove(&self, _agent_id: &str) {}
}

pub trait ReputationStore: Send + Sync {
    fn score(&self, agent_id: &str, now: chrono::DateTime<Utc>) -> Option<f64>;
    fn adjust(
        &self,
        agent_id: &str,
        delta: f64,
        now: chrono::DateTime<Utc>,
    );
    fn remove(&self, _agent_id: &str) {}
}

#[async_trait]
pub trait AgentExecutor: Send + Sync {
    async fn execute(&self, agent_id: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct InMemoryAgentDirectory {
    agents: RwLock<HashMap<String, AgentIdentity>>,
}

impl InMemoryAgentDirectory {
    fn lock(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, AgentIdentity>> {
        self.agents
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_mut(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, AgentIdentity>> {
        self.agents
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl AgentDirectory for InMemoryAgentDirectory {
    fn register(&self, mut identity: AgentIdentity) -> Result<(), FleetError> {
        let agent_id = sanitize_text(&identity.agent_id)
            .map_err(|_| FleetError::InvalidAgentId)?;
        if agent_id.is_empty() {
            return Err(FleetError::EmptyAgentId);
        }
        identity.agent_id = agent_id.clone();
        let mut agents = self.lock_mut();
        if agents.contains_key(&agent_id) {
            return Err(FleetError::DuplicateAgent);
        }
        agents.insert(agent_id, identity);
        Ok(())
    }

    fn remove(&self, agent_id: &str) -> bool {
        self.lock_mut().remove(agent_id).is_some()
    }

    fn get(&self, agent_id: &str) -> Option<AgentIdentity> {
        self.lock().get(agent_id).cloned()
    }

    fn list(&self) -> Vec<AgentIdentity> {
        let mut agents = self.lock().values().cloned().collect::<Vec<_>>();
        agents.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));
        agents
    }

    fn set_active(&self, agent_id: &str, active: bool) -> Result<(), FleetError> {
        let mut agents = self.lock_mut();
        let identity = agents
            .get_mut(agent_id)
            .ok_or(FleetError::UnknownAgent)?;
        identity.is_active = active;
        if active {
            identity.is_active = true;
            identity.revoked_at = None;
        } else {
            identity.revoke();
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryHealthRegistry {
    service: Mutex<HealthAttestationService>,
}

impl HealthRegistry for InMemoryHealthRegistry {
    fn register(&self, agent_id: &str, threshold: usize) {
        self.service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_agent(agent_id.to_string(), threshold);
    }

    fn record_self_report(&self, agent_id: &str) {
        self.service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_self_report(agent_id);
    }

    fn record_peer_attestation(&self, target: &str, peer: &str) {
        self.service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_peer_attestation(target, peer);
    }

    fn is_healthy(&self, agent_id: &str) -> Option<bool> {
        self.service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .check_health(agent_id)
    }

    fn remove(&self, agent_id: &str) {
        self.service
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove_agent(agent_id);
    }
}

#[derive(Default)]
pub struct InMemoryFailoverCoordinator {
    manager: Mutex<FailoverManager>,
}

impl FailoverCoordinator for InMemoryFailoverCoordinator {
    fn can_execute(&self, vault_id: &str) -> bool {
        self.manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .can_execute(vault_id)
    }

    fn record_failure(&self, agent_id: &str) -> bool {
        self.manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_health_failure(agent_id)
    }

    fn record_success(&self, agent_id: &str) {
        self.manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record_health_success(agent_id);
    }

    fn initiate_failover(&self, vault_id: &str, replacement_agent_id: &str) {
        self.manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .initiate_failover(vault_id, replacement_agent_id);
    }

    fn confirm_reauth(&self, vault_id: &str) -> bool {
        self.manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .confirm_reauth(vault_id)
    }

    fn remove(&self, agent_id: &str) {
        self.manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove_agent(agent_id);
    }
}

#[derive(Default)]
pub struct InMemoryReputationStore {
    records: Mutex<HashMap<String, ReputationRecord>>,
}

impl ReputationStore for InMemoryReputationStore {
    fn score(&self, agent_id: &str, now: chrono::DateTime<Utc>) -> Option<f64> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(agent_id)
            .map(|record| record.current_score(now))
    }

    fn adjust(
        &self,
        agent_id: &str,
        delta: f64,
        now: chrono::DateTime<Utc>,
    ) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = records
            .entry(agent_id.to_string())
            .or_insert_with(|| ReputationRecord::new(agent_id.to_string(), 50.0, 0.1));
        record.add_score(delta, now);
    }

    fn remove(&self, agent_id: &str) {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(agent_id);
    }
}

pub type DefaultAgentFleet = AgentFleet<
    InMemoryAgentDirectory,
    InMemoryHealthRegistry,
    InMemoryFailoverCoordinator,
    InMemoryReputationStore,
>;

pub struct AgentFleet<D = InMemoryAgentDirectory, H = InMemoryHealthRegistry, F = InMemoryFailoverCoordinator, R = InMemoryReputationStore> {
    pub directory: D,
    pub health: H,
    pub failover: F,
    pub reputation: R,
}

impl AgentFleet<InMemoryAgentDirectory, InMemoryHealthRegistry, InMemoryFailoverCoordinator, InMemoryReputationStore> {
    pub fn new() -> Self {
        Self::with_services(
            InMemoryAgentDirectory::default(),
            InMemoryHealthRegistry::default(),
            InMemoryFailoverCoordinator::default(),
            InMemoryReputationStore::default(),
        )
    }
}

impl Default for AgentFleet<InMemoryAgentDirectory, InMemoryHealthRegistry, InMemoryFailoverCoordinator, InMemoryReputationStore> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D, H, F, R> AgentFleet<D, H, F, R>
where
    D: AgentDirectory,
    H: HealthRegistry,
    F: FailoverCoordinator,
    R: ReputationStore,
{
    pub fn with_services(directory: D, health: H, failover: F, reputation: R) -> Self {
        Self {
            directory,
            health,
            failover,
            reputation,
        }
    }

    pub fn register(&self, identity: AgentIdentity) -> Result<(), FleetError> {
        let agent_id = sanitize_text(&identity.agent_id)
            .map_err(|_| FleetError::InvalidAgentId)?;
        if agent_id.is_empty() {
            return Err(FleetError::EmptyAgentId);
        }
        self.directory.register(identity)?;
        self.health.register(&agent_id, 1);
        Ok(())
    }

    pub fn register_with_threshold(
        &self,
        identity: AgentIdentity,
        threshold: usize,
    ) -> Result<(), FleetError> {
        let agent_id = sanitize_text(&identity.agent_id)
            .map_err(|_| FleetError::InvalidAgentId)?;
        if agent_id.is_empty() {
            return Err(FleetError::EmptyAgentId);
        }
        self.directory.register(identity)?;
        self.health.register(&agent_id, threshold);
        Ok(())
    }

    pub fn register_agent(&self, agent_id: impl Into<String>) -> Result<(), FleetError> {
        self.register(AgentIdentity::new(agent_id.into()))
    }

    pub fn unregister(&self, agent_id: &str) -> bool {
        let removed = self.directory.remove(agent_id);
        if removed {
            self.health.remove(agent_id);
            self.failover.remove(agent_id);
            self.reputation.remove(agent_id);
        }
        removed
    }

    pub fn revoke(&self, agent_id: &str) -> Result<(), FleetError> {
        self.directory.set_active(agent_id, false)
    }

    pub fn record_self_report(&self, agent_id: &str) {
        self.health.record_self_report(agent_id);
    }

    pub fn record_peer_attestation(&self, target: &str, peer: &str) {
        self.health.record_peer_attestation(target, peer);
    }

    pub fn is_healthy(&self, agent_id: &str) -> bool {
        self.directory
            .get(agent_id)
            .map(|identity| identity.is_active && !identity.is_revoked())
            .unwrap_or(false)
            && self.health.is_healthy(agent_id).unwrap_or(false)
    }

    pub fn healthy_agents(&self, vault_id: &str) -> Vec<String> {
        self.directory
            .list()
            .into_iter()
            .filter(|identity| identity.is_active && !identity.is_revoked())
            .map(|identity| identity.agent_id)
            .filter(|agent_id| self.health.is_healthy(agent_id).unwrap_or(false))
            .filter(|_| self.failover.can_execute(vault_id))
            .collect()
    }

    pub fn select_agent(&self, vault_id: &str, preferred: Option<&str>) -> Option<String> {
        let mut agents = self.healthy_agents(vault_id);
        if let Some(preferred) = preferred {
            if let Some(index) = agents.iter().position(|agent| agent == preferred) {
                let selected = agents.remove(index);
                agents.insert(0, selected);
            }
        }
        agents.into_iter().next()
    }

    pub fn record_failure(
        &self,
        vault_id: &str,
        agent_id: &str,
        replacement_agent_id: Option<&str>,
    ) -> bool {
        let triggered = self.failover.record_failure(agent_id);
        if triggered {
            if let Some(replacement) = replacement_agent_id {
                self.failover.initiate_failover(vault_id, replacement);
            }
        }
        triggered
    }

    pub fn record_success(&self, agent_id: &str) {
        self.failover.record_success(agent_id);
    }

    pub fn confirm_reauth(&self, vault_id: &str) -> bool {
        self.failover.confirm_reauth(vault_id)
    }

    pub fn reputation_score(&self, agent_id: &str) -> Option<f64> {
        self.reputation.score(agent_id, Utc::now())
    }

    pub fn adjust_reputation(&self, agent_id: &str, delta: f64) {
        self.reputation.adjust(agent_id, delta, Utc::now());
    }

    pub async fn execute<E>(
        &self,
        executor: &E,
        vault_id: &str,
        preferred: Option<&str>,
    ) -> Result<String, FleetError>
    where
        E: AgentExecutor + ?Sized,
    {
        let mut candidates = self.healthy_agents(vault_id);
        if let Some(preferred) = preferred {
            if let Some(index) = candidates.iter().position(|agent| agent == preferred) {
                let selected = candidates.remove(index);
                candidates.insert(0, selected);
            }
        }
        if candidates.is_empty() {
            return Err(FleetError::NoHealthyAgent);
        }

        let mut last_error = None;
        for (index, agent_id) in candidates.iter().enumerate() {
            match executor.execute(agent_id).await {
                Ok(()) => {
                    self.record_success(agent_id);
                    self.adjust_reputation(agent_id, 1.0);
                    return Ok(agent_id.clone());
                }
                Err(message) => {
                    let replacement = candidates.get(index + 1).cloned();
                    let _ = self.record_failure(vault_id, agent_id, replacement.as_deref());
                    last_error = Some(FleetError::Execution {
                        agent_id: agent_id.clone(),
                        message,
                    });
                }
            }
        }

        Err(last_error.unwrap_or(FleetError::NoHealthyAgent))
    }

    pub fn snapshot(&self, vault_id: &str) -> FleetSnapshot {
        FleetSnapshot {
            agents: self.directory.list(),
            healthy_agents: self.healthy_agents(vault_id),
        }
    }

    pub fn is_operational(&self, vault_id: &str) -> bool {
        let agents = self.directory.list();
        let healthy_agents = self.healthy_agents(vault_id);
        !agents.is_empty() && healthy_agents.len() == agents.len()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetSnapshot {
    pub agents: Vec<AgentIdentity>,
    pub healthy_agents: Vec<String>,
}
