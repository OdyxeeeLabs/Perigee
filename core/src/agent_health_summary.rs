//! Manager-facing agent health summary (API-28).
//!
//! Managers need to know whether the agent assigned to their vault is working
//! without seeing operator-only fleet internals. This module keeps the vault →
//! agent assignment and the last heartbeat per agent, and turns them into a
//! small, stable summary:
//!
//! * a [`AgentHealthStatus`] from a fixed set,
//! * the last heartbeat time and its age,
//! * the freshness thresholds used, and
//! * actionable guidance whenever the status is not `active`.
//!
//! Peer attestations, reputation, failover history, host details and the raw
//! diagnostic payload an agent reports stay internal: they are never part of
//! [`AgentHealthSummary`]. Summaries are tenant-scoped; another manager's vault
//! is reported as not found. See `docs/AGENT_HEALTH_SUMMARY.md`.

use crate::auth::AuthenticatedUser;
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use axum::{
    extract::Path,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A heartbeat at most this old counts as fresh.
pub const DEFAULT_FRESH_SECS: u64 = 120;
/// With no heartbeat for longer than this the agent is reported offline.
pub const DEFAULT_OFFLINE_SECS: u64 = 600;
/// Upper bound on summaries returned by the list endpoint.
pub const MAX_LISTED_VAULTS: usize = 200;

/// The stable status values managers can build on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHealthStatus {
    /// Fresh heartbeat and the fleet attests the agent is healthy.
    Active,
    /// Heartbeat is late, or the agent reports in but is not attested healthy.
    Degraded,
    /// No heartbeat within the offline threshold.
    Offline,
    /// The vault has no agent assigned.
    Unassigned,
    /// Health data is not available (no heartbeat recorded yet).
    Unknown,
}

/// Freshness thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshnessPolicy {
    pub fresh_secs: u64,
    pub offline_secs: u64,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        Self {
            fresh_secs: DEFAULT_FRESH_SECS,
            offline_secs: DEFAULT_OFFLINE_SECS,
        }
    }
}

fn rfc3339<S: Serializer>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

fn rfc3339_opt<S: Serializer>(
    value: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => rfc3339(value, serializer),
        None => serializer.serialize_none(),
    }
}

/// What a manager sees for one vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentHealthSummary {
    pub vault_id: String,
    /// Assigned agent, or `null` when unassigned.
    pub agent_id: Option<String>,
    pub status: AgentHealthStatus,
    #[serde(serialize_with = "rfc3339_opt")]
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub heartbeat_age_secs: Option<u64>,
    pub fresh_within_secs: u64,
    pub offline_after_secs: u64,
    /// What the manager can do about a non-`active` status.
    pub guidance: Option<String>,
    #[serde(serialize_with = "rfc3339")]
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct VaultAgent {
    manager_id: String,
    agent_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Heartbeat {
    at: DateTime<Utc>,
    /// Operator-only payload (versions, hosts, errors). Never summarised.
    #[allow(dead_code)]
    diagnostics: Option<String>,
}

#[derive(Default)]
struct Inner {
    vaults: HashMap<String, VaultAgent>,
    heartbeats: HashMap<String, Heartbeat>,
}

type AttestationLookup = dyn Fn(&str) -> Option<bool> + Send + Sync;

/// Tracks assignments and heartbeats and produces tenant-scoped summaries.
pub struct AgentHealthRegistry {
    inner: RwLock<Inner>,
    policy: FreshnessPolicy,
    /// Fleet attestation (`AgentFleet::is_healthy`); `None` answer = unknown.
    attestation: Option<Box<AttestationLookup>>,
}

impl Default for AgentHealthRegistry {
    fn default() -> Self {
        Self::new(FreshnessPolicy::default())
    }
}

/// Lookup failed for the caller: missing, or owned by another manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultNotFound;

impl AgentHealthRegistry {
    pub fn new(policy: FreshnessPolicy) -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            policy,
            attestation: None,
        }
    }

    /// Use the fleet's health/identity services to attest heartbeating agents.
    pub fn with_attestation(
        mut self,
        lookup: impl Fn(&str) -> Option<bool> + Send + Sync + 'static,
    ) -> Self {
        self.attestation = Some(Box::new(lookup));
        self
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record which agent (if any) serves `vault_id` for `manager_id`.
    pub fn assign(&self, vault_id: &str, manager_id: &str, agent_id: Option<&str>) {
        self.write().vaults.insert(
            vault_id.to_string(),
            VaultAgent {
                manager_id: manager_id.to_string(),
                agent_id: agent_id.map(str::to_string),
            },
        );
    }

    /// Record a heartbeat. `diagnostics` is stored for operators only.
    pub fn record_heartbeat(&self, agent_id: &str, at: DateTime<Utc>, diagnostics: Option<String>) {
        let mut inner = self.write();
        let newer = inner.heartbeats.get(agent_id).is_none_or(|h| h.at <= at);
        if newer {
            inner
                .heartbeats
                .insert(agent_id.to_string(), Heartbeat { at, diagnostics });
        }
    }

    fn summarise(
        &self,
        inner: &Inner,
        vault_id: &str,
        record: &VaultAgent,
        now: DateTime<Utc>,
    ) -> AgentHealthSummary {
        let mut summary = AgentHealthSummary {
            vault_id: vault_id.to_string(),
            agent_id: record.agent_id.clone(),
            status: AgentHealthStatus::Unknown,
            last_heartbeat_at: None,
            heartbeat_age_secs: None,
            fresh_within_secs: self.policy.fresh_secs,
            offline_after_secs: self.policy.offline_secs,
            guidance: None,
            checked_at: now,
        };
        let Some(agent_id) = record.agent_id.as_deref() else {
            summary.status = AgentHealthStatus::Unassigned;
            summary.guidance = Some(
                "No agent is assigned to this vault. Assign an agent before enabling automated strategy execution."
                    .into(),
            );
            return summary;
        };
        let Some(heartbeat) = inner.heartbeats.get(agent_id) else {
            summary.guidance = Some(
                "Health data for this agent is not available yet. Retry shortly; if it stays unknown for more than 10 minutes, contact support with the vault id."
                    .into(),
            );
            return summary;
        };

        // A heartbeat stamped slightly in the future (clock skew) counts as age 0.
        let age = u64::try_from((now - heartbeat.at).num_seconds()).unwrap_or(0);
        summary.last_heartbeat_at = Some(heartbeat.at);
        summary.heartbeat_age_secs = Some(age);
        let attested = self
            .attestation
            .as_ref()
            .and_then(|lookup| lookup(agent_id));

        let (status, guidance) = if age > self.policy.offline_secs {
            (
                AgentHealthStatus::Offline,
                Some(format!(
                    "The agent has not reported for over {} seconds. Automated actions for this vault are paused; restart the agent or request a reassignment.",
                    self.policy.offline_secs
                )),
            )
        } else if age > self.policy.fresh_secs {
            (
                AgentHealthStatus::Degraded,
                Some(format!(
                    "The last heartbeat was {age} seconds ago (expected within {} seconds). Actions may be delayed; check the agent's connectivity.",
                    self.policy.fresh_secs
                )),
            )
        } else if attested == Some(false) {
            (
                AgentHealthStatus::Degraded,
                Some(
                    "The agent is reporting but has not passed fleet health attestation, so automated actions may be paused. Contact support if this lasts more than 10 minutes."
                        .to_string(),
                ),
            )
        } else {
            (AgentHealthStatus::Active, None)
        };
        summary.status = status;
        summary.guidance = guidance;
        summary
    }

    /// Summary for one vault, visible to its manager (or an administrator).
    pub fn summary_for(
        &self,
        vault_id: &str,
        caller: &str,
        caller_is_admin: bool,
        now: DateTime<Utc>,
    ) -> Result<AgentHealthSummary, VaultNotFound> {
        let inner = self.read();
        let record = inner.vaults.get(vault_id).ok_or(VaultNotFound)?;
        if record.manager_id != caller && !caller_is_admin {
            return Err(VaultNotFound);
        }
        Ok(self.summarise(&inner, vault_id, record, now))
    }

    /// Summaries for every vault the caller manages, ordered by vault id.
    pub fn summaries_for_manager(
        &self,
        manager_id: &str,
        now: DateTime<Utc>,
    ) -> Vec<AgentHealthSummary> {
        let inner = self.read();
        let mut vaults: Vec<(&String, &VaultAgent)> = inner
            .vaults
            .iter()
            .filter(|(_, record)| record.manager_id == manager_id)
            .collect();
        vaults.sort_by(|a, b| a.0.cmp(b.0));
        vaults
            .into_iter()
            .take(MAX_LISTED_VAULTS)
            .map(|(vault_id, record)| self.summarise(&inner, vault_id, record, now))
            .collect()
    }
}

fn not_found(vault_id: &str) -> Response {
    let body = ProblemDetails::from_status(404, "vault not found")
        .with_instance(&format!("/vaults/{vault_id}/agent-health"))
        .to_json();
    let mut response = (StatusCode::NOT_FOUND, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

/// `GET /vaults/:id/agent-health`
pub async fn vault_agent_health_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(registry): Extension<Arc<AgentHealthRegistry>>,
    Path(vault_id): Path<String>,
) -> Response {
    if !user.can_access_vault(&vault_id) {
        return not_found(&vault_id);
    }
    match registry.summary_for(
        &vault_id,
        &user.stellar_address,
        user.is_admin(),
        Utc::now(),
    ) {
        Ok(summary) => Json(summary).into_response(),
        Err(VaultNotFound) => not_found(&vault_id),
    }
}

/// Body of `GET /agents/health`.
#[derive(Debug, Serialize)]
pub struct AgentHealthList {
    pub vaults: Vec<AgentHealthSummary>,
}

/// `GET /agents/health` – summaries for all of the caller's vaults.
pub async fn manager_agent_health_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(registry): Extension<Arc<AgentHealthRegistry>>,
) -> Response {
    let vaults = registry
        .summaries_for_manager(&user.stellar_address, Utc::now())
        .into_iter()
        .filter(|summary| user.can_access_vault(&summary.vault_id))
        .collect();
    Json(AgentHealthList { vaults }).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use axum::{body::Body, http::Request, routing::get, Router};
    use chrono::Duration;
    use tower::ServiceExt;

    const ALICE: &str = "GALICE";
    const BOB: &str = "GBOB";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-19T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn registry() -> AgentHealthRegistry {
        let registry = AgentHealthRegistry::default().with_attestation(|agent| match agent {
            "agent-unattested" => Some(false),
            "agent-unregistered" => None,
            _ => Some(true),
        });
        registry.assign("v-active", ALICE, Some("agent-a"));
        registry.assign("v-late", ALICE, Some("agent-late"));
        registry.assign("v-offline", ALICE, Some("agent-off"));
        registry.assign("v-unattested", ALICE, Some("agent-unattested"));
        registry.assign("v-unknown", ALICE, Some("agent-silent"));
        registry.assign("v-unassigned", ALICE, None);
        registry.assign("v-bob", BOB, Some("agent-bob"));
        let secret =
            Some("host=10.0.0.7 peers=[agent-x,agent-y] err=ECONNRESET token=abc".to_string());
        registry.record_heartbeat("agent-a", now() - Duration::seconds(30), secret.clone());
        registry.record_heartbeat("agent-late", now() - Duration::seconds(300), secret.clone());
        registry.record_heartbeat(
            "agent-off",
            now() - Duration::seconds(3_600),
            secret.clone(),
        );
        registry.record_heartbeat(
            "agent-unattested",
            now() - Duration::seconds(10),
            secret.clone(),
        );
        registry.record_heartbeat("agent-bob", now() - Duration::seconds(5), secret);
        registry
    }

    fn status(registry: &AgentHealthRegistry, vault: &str) -> AgentHealthSummary {
        registry.summary_for(vault, ALICE, false, now()).unwrap()
    }

    #[test]
    fn statuses_map_from_freshness_and_attestation() {
        let registry = registry();
        let active = status(&registry, "v-active");
        assert_eq!(active.status, AgentHealthStatus::Active);
        assert_eq!(active.heartbeat_age_secs, Some(30));
        assert!(active.guidance.is_none());

        let late = status(&registry, "v-late");
        assert_eq!(late.status, AgentHealthStatus::Degraded);
        assert!(late
            .guidance
            .as_deref()
            .unwrap()
            .contains("300 seconds ago"));

        let offline = status(&registry, "v-offline");
        assert_eq!(offline.status, AgentHealthStatus::Offline);
        assert!(offline.guidance.as_deref().unwrap().contains("paused"));

        let unattested = status(&registry, "v-unattested");
        assert_eq!(unattested.status, AgentHealthStatus::Degraded);
        assert!(unattested
            .guidance
            .as_deref()
            .unwrap()
            .contains("attestation"));

        let unknown = status(&registry, "v-unknown");
        assert_eq!(unknown.status, AgentHealthStatus::Unknown);
        assert_eq!(unknown.last_heartbeat_at, None);
        assert!(unknown
            .guidance
            .as_deref()
            .unwrap()
            .contains("not available"));

        let unassigned = status(&registry, "v-unassigned");
        assert_eq!(unassigned.status, AgentHealthStatus::Unassigned);
        assert_eq!(unassigned.agent_id, None);
    }

    #[test]
    fn freshness_boundaries_are_inclusive_of_the_threshold() {
        let registry = AgentHealthRegistry::default();
        registry.assign("v", ALICE, Some("a"));
        registry.record_heartbeat(
            "a",
            now() - Duration::seconds(DEFAULT_FRESH_SECS as i64),
            None,
        );
        assert_eq!(status(&registry, "v").status, AgentHealthStatus::Active);
        registry.record_heartbeat(
            "a",
            now() - Duration::seconds(DEFAULT_FRESH_SECS as i64 - 1),
            None,
        );
        registry.assign("w", ALICE, Some("b"));
        registry.record_heartbeat(
            "b",
            now() - Duration::seconds(DEFAULT_OFFLINE_SECS as i64),
            None,
        );
        assert_eq!(status(&registry, "w").status, AgentHealthStatus::Degraded);
        registry.assign("x", ALICE, Some("c"));
        registry.record_heartbeat(
            "c",
            now() - Duration::seconds(DEFAULT_OFFLINE_SECS as i64 + 1),
            None,
        );
        assert_eq!(status(&registry, "x").status, AgentHealthStatus::Offline);
    }

    #[test]
    fn missing_attestation_does_not_downgrade_a_fresh_agent() {
        let registry = registry();
        registry.assign("v-unreg", ALICE, Some("agent-unregistered"));
        registry.record_heartbeat("agent-unregistered", now(), None);
        assert_eq!(
            status(&registry, "v-unreg").status,
            AgentHealthStatus::Active
        );
    }

    #[test]
    fn stale_heartbeats_never_overwrite_newer_ones_and_future_skew_is_age_zero() {
        let registry = AgentHealthRegistry::default();
        registry.assign("v", ALICE, Some("a"));
        registry.record_heartbeat("a", now() + Duration::seconds(5), None);
        registry.record_heartbeat("a", now() - Duration::seconds(900), None);
        let summary = status(&registry, "v");
        assert_eq!(summary.heartbeat_age_secs, Some(0));
        assert_eq!(summary.status, AgentHealthStatus::Active);
    }

    #[test]
    fn tenants_only_see_their_own_vaults() {
        let registry = registry();
        assert_eq!(
            registry.summary_for("v-bob", ALICE, false, now()),
            Err(VaultNotFound)
        );
        assert_eq!(
            registry.summary_for("v-missing", ALICE, false, now()),
            Err(VaultNotFound)
        );
        assert!(registry.summary_for("v-bob", BOB, false, now()).is_ok());
        assert!(registry.summary_for("v-bob", "GADMIN", true, now()).is_ok());

        let alice: Vec<String> = registry
            .summaries_for_manager(ALICE, now())
            .into_iter()
            .map(|s| s.vault_id)
            .collect();
        assert_eq!(alice.len(), 6);
        assert!(!alice.contains(&"v-bob".to_string()));
        assert!(alice.windows(2).all(|w| w[0] <= w[1]), "stable order");
    }

    #[test]
    fn sensitive_diagnostics_are_never_exposed() {
        let registry = registry();
        let rendered =
            serde_json::to_string(&registry.summaries_for_manager(ALICE, now())).unwrap();
        for secret in ["10.0.0.7", "agent-x", "ECONNRESET", "token=abc", "peers"] {
            assert!(!rendered.contains(secret), "leaked {secret}");
        }
        let json = serde_json::to_value(status(&registry, "v-active")).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "agent_id",
                "checked_at",
                "fresh_within_secs",
                "guidance",
                "heartbeat_age_secs",
                "last_heartbeat_at",
                "offline_after_secs",
                "status",
                "vault_id",
            ]
        );
        assert_eq!(json["last_heartbeat_at"], "2026-11-19T08:29:30Z");
        assert_eq!(json["status"], "active");
    }

    fn app(registry: Arc<AgentHealthRegistry>, user: AuthenticatedUser) -> Router {
        Router::new()
            .route("/vaults/:id/agent-health", get(vault_agent_health_handler))
            .route("/agents/health", get(manager_agent_health_handler))
            .layer(Extension(registry))
            .layer(Extension(user))
    }

    async fn get_json(router: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn http_endpoints_enforce_tenant_boundaries() {
        let registry = Arc::new(registry());
        let alice = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);

        let (status, body) = get_json(
            app(registry.clone(), alice.clone()),
            "/vaults/v-bob/agent-health",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["detail"], "vault not found");

        let (status, body) = get_json(
            app(registry.clone(), alice.clone()),
            "/vaults/v-unassigned/agent-health",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "unassigned");

        let (status, body) = get_json(app(registry.clone(), alice), "/agents/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["vaults"].as_array().unwrap().len(), 6);

        // A token scoped to one vault sees only that vault.
        let scoped = AuthenticatedUser::new(ALICE.into(), Role::Viewer, vec!["v-active".into()]);
        let (_, body) = get_json(app(registry.clone(), scoped.clone()), "/agents/health").await;
        assert_eq!(body["vaults"].as_array().unwrap().len(), 1);
        let (status, _) = get_json(app(registry, scoped), "/vaults/v-late/agent-health").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
