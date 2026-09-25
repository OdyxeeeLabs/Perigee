//! Automatic failover with health-check integration.
//!
//! When an agent fails health checks for N consecutive attempts,
//! automatic failover is triggered and an event is emitted.

pub struct FailoverManager {
    pending_reauth: Vec<String>,
    consecutive_failures: std::collections::HashMap<String, u32>,
    failure_threshold: u32,
    events: Vec<FailoverEvent>,
}

#[derive(Debug, Clone)]
pub struct FailoverEvent {
    pub vault_id: String,
    pub agent_id: String,
    pub reason: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Default for FailoverManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FailoverManager {
    pub fn new() -> Self {
        Self {
            pending_reauth: Vec::new(),
            consecutive_failures: std::collections::HashMap::new(),
            failure_threshold: 3,
            events: Vec::new(),
        }
    }

    pub fn with_failure_threshold(threshold: u32) -> Self {
        Self {
            pending_reauth: Vec::new(),
            consecutive_failures: std::collections::HashMap::new(),
            failure_threshold: threshold,
            events: Vec::new(),
        }
    }

    pub fn initiate_failover(&mut self, vault_id: &str, new_agent_id: &str) {
        let key = format!("{}:{}", vault_id, new_agent_id);
        if !self.pending_reauth.contains(&key) {
            self.pending_reauth.push(key);
        }
    }

    pub fn confirm_reauth(&mut self, vault_id: &str) -> bool {
        let prefix = format!("{}:", vault_id);
        let initial_len = self.pending_reauth.len();
        self.pending_reauth.retain(|entry| !entry.starts_with(&prefix));
        self.pending_reauth.len() < initial_len
    }

    pub fn can_execute(&self, vault_id: &str) -> bool {
        let prefix = format!("{}:", vault_id);
        !self.pending_reauth.iter().any(|entry| entry.starts_with(&prefix))
    }

    pub fn record_health_failure(&mut self, agent_id: &str) -> bool {
        let count = self
            .consecutive_failures
            .entry(agent_id.to_string())
            .or_insert(0);
        *count += 1;
        *count >= self.failure_threshold
    }

    pub fn reset_health_failures(&mut self, agent_id: &str) {
        self.consecutive_failures.remove(agent_id);
    }

    pub fn record_health_success(&mut self, agent_id: &str) {
        self.consecutive_failures.remove(agent_id);
    }

    pub fn remove_agent(&mut self, agent_id: &str) {
        self.consecutive_failures.remove(agent_id);
        let suffix = format!(":{}", agent_id);
        self.pending_reauth
            .retain(|entry| !entry.as_str().ends_with(suffix.as_str()));
        self.events.retain(|event| event.agent_id != agent_id);
    }

    pub fn check_and_trigger_failover(
        &mut self,
        vault_id: &str,
        agent_id: &str,
        replacement_agent_id: &str,
    ) -> bool {
        let should_failover = self.record_health_failure(agent_id);
        if should_failover {
            self.initiate_failover(vault_id, replacement_agent_id);
            self.events.push(FailoverEvent {
                vault_id: vault_id.to_string(),
                agent_id: agent_id.to_string(),
                reason: format!(
                    "Health check failed {} consecutive times",
                    self.failure_threshold
                ),
                timestamp: chrono::Utc::now(),
            });
            true
        } else {
            false
        }
    }

    pub fn events(&self) -> &[FailoverEvent] {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failover_lifecycle() {
        let mut mgr = FailoverManager::new();
        assert!(mgr.can_execute("v1"));
        mgr.initiate_failover("v1", "a1");
        assert!(!mgr.can_execute("v1"));
        assert!(mgr.confirm_reauth("v1"));
        assert!(mgr.can_execute("v1"));
    }

    #[test]
    fn test_confirm_reauth_returns_false_when_no_pending() {
        let mut mgr = FailoverManager::new();
        assert!(!mgr.confirm_reauth("v1"));
    }

    #[test]
    fn test_no_duplicate_pending() {
        let mut mgr = FailoverManager::new();
        mgr.initiate_failover("v1", "a1");
        mgr.initiate_failover("v1", "a1");
        assert_eq!(mgr.pending_reauth.len(), 1);
    }

    #[test]
    fn test_health_check_triggers_failover() {
        let mut mgr = FailoverManager::with_failure_threshold(3);
        assert!(!mgr.check_and_trigger_failover("v1", "bad_agent", "good_agent"));
        assert!(!mgr.check_and_trigger_failover("v1", "bad_agent", "good_agent"));
        assert!(mgr.check_and_trigger_failover("v1", "bad_agent", "good_agent"));
        assert!(!mgr.can_execute("v1"));
        assert_eq!(mgr.events().len(), 1);
    }

    #[test]
    fn test_health_success_resets_failures() {
        let mut mgr = FailoverManager::with_failure_threshold(3);
        mgr.record_health_failure("agent1");
        mgr.record_health_failure("agent1");
        mgr.record_health_success("agent1");
        assert!(!mgr.record_health_failure("agent1"));
    }

    #[test]
    fn test_health_failure_counts() {
        let mut mgr = FailoverManager::new();
        assert!(!mgr.record_health_failure("a1"));
        assert!(!mgr.record_health_failure("a1"));
        assert!(mgr.record_health_failure("a1"));
    }
}
