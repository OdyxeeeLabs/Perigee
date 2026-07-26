use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub manager_id: String,
    pub action: String,
    pub actor: String,
    pub timestamp: DateTime<Utc>,
    pub details: Option<String>,
    pub ip_address: Option<String>,
}

pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn record(&mut self, entry: AuditEntry) {
        self.entries.push(entry);
    }

    pub fn query_by_manager(&self, manager_id: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.manager_id == manager_id)
            .collect()
    }

    pub fn query_by_action(&self, action: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.action == action)
            .collect()
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_audit_entry(manager_id: &str, action: &str, actor: &str) -> AuditEntry {
    AuditEntry {
        id: Uuid::new_v4().to_string(),
        manager_id: manager_id.to_string(),
        action: action.to_string(),
        actor: actor.to_string(),
        timestamp: Utc::now(),
        details: None,
        ip_address: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_audit_entry() {
        let entry = create_audit_entry("mgr1", "approve_payment", "admin");
        assert_eq!(entry.manager_id, "mgr1");
        assert_eq!(entry.action, "approve_payment");
        assert_eq!(entry.actor, "admin");
        assert!(!entry.id.is_empty());
    }

    #[test]
    fn test_audit_log_record_and_query() {
        let mut log = AuditLog::new();
        log.record(create_audit_entry("mgr1", "approve_payment", "admin"));
        log.record(create_audit_entry("mgr2", "reject_payment", "admin"));
        log.record(create_audit_entry("mgr1", "update_settings", "admin"));

        assert_eq!(log.query_by_manager("mgr1").len(), 2);
        assert_eq!(log.query_by_manager("mgr2").len(), 1);
        assert_eq!(log.query_by_action("approve_payment").len(), 1);
    }
}
