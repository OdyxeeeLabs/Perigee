//! Read-only admin overview of service operations (API-31).
//!
//! `GET /admin/overview` gives operators one bounded, read-only snapshot of
//! queue, outbox and dependency health without querying internal stores by
//! hand. It is gated on the administrator permission; managers, operators and
//! viewers receive `403`.
//!
//! The response carries **aggregates only**: per-component counts from a
//! documented allowlist ([`DOCUMENTED_COUNT_KEYS`]), a status, and timestamps.
//! Probes cannot leak raw records, identifiers or customer data because any
//! count key outside the allowlist is dropped before the response is built,
//! and probe error messages are never echoed back.
//!
//! Every probe runs concurrently under its own timeout. A probe that does not
//! answer in time is reported explicitly as `timeout` (with the budget it
//! exceeded) instead of stalling the whole request or silently disappearing.
//! The number of components is capped at [`MAX_COMPONENTS`].
//!
//! See `docs/ADMIN_OVERVIEW.md` for the documented response fields.

use crate::auth::AuthenticatedUser;
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use crate::timestamps::{rfc3339_utc, rfc3339_utc_option};
use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Budget for a single probe when none is configured.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Upper bound on registered components so the response stays bounded.
pub const MAX_COMPONENTS: usize = 32;

/// Count keys a component may report. Anything else is discarded.
pub const DOCUMENTED_COUNT_KEYS: &[&str] = &[
    // Job queues
    "waiting",
    "delayed",
    "active",
    "completed",
    "dead",
    "throughput_per_min",
    // Outbox
    "pending",
    "published",
    "quarantined",
    // Dependencies
    "latency_ms",
    "healthy_instances",
    "total_instances",
];

/// The class of component a probe describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    Queue,
    Outbox,
    Dependency,
}

/// Health verdict for one component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    Healthy,
    Degraded,
    /// The probe ran and reported a failure.
    Unavailable,
    /// The probe did not answer within its budget.
    Timeout,
}

/// What a probe hands back: a verdict plus aggregate figures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub status: ComponentStatus,
    pub counts: BTreeMap<String, u64>,
    /// When the oldest outstanding item (job, event) was created.
    pub oldest_item_at: Option<DateTime<Utc>>,
}

impl ProbeReport {
    pub fn new(status: ComponentStatus) -> Self {
        Self {
            status,
            counts: BTreeMap::new(),
            oldest_item_at: None,
        }
    }

    pub fn count(mut self, key: &str, value: u64) -> Self {
        self.counts.insert(key.to_string(), value);
        self
    }

    pub fn oldest_item_at(mut self, at: Option<DateTime<Utc>>) -> Self {
        self.oldest_item_at = at;
        self
    }
}

/// Boxed future returned by [`OverviewProbe::probe`].
pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = Result<ProbeReport, String>> + Send + 'a>>;

/// A read-only source of aggregate health figures.
pub trait OverviewProbe: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> ComponentKind;
    /// Must not mutate the observed system.
    fn probe(&self) -> ProbeFuture<'_>;
}

/// One component in the admin overview response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComponentOverview {
    pub name: String,
    pub kind: ComponentKind,
    pub status: ComponentStatus,
    pub counts: BTreeMap<String, u64>,
    #[serde(with = "rfc3339_utc_option")]
    pub oldest_item_at: Option<DateTime<Utc>>,
    #[serde(with = "rfc3339_utc")]
    pub checked_at: DateTime<Utc>,
    /// Probe budget in milliseconds; reported so a `timeout` is actionable.
    pub timeout_ms: u64,
    /// Fixed, non-sensitive explanation for `timeout` / `unavailable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Body of `GET /admin/overview`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdminOverview {
    #[serde(with = "rfc3339_utc")]
    pub generated_at: DateTime<Utc>,
    /// `healthy` only when every component is healthy, else `degraded`.
    pub overall_status: ComponentStatus,
    pub components: Vec<ComponentOverview>,
}

/// Collects component probes and runs them with per-probe timeouts.
pub struct AdminOverviewService {
    probes: Vec<Arc<dyn OverviewProbe>>,
    probe_timeout: Duration,
}

impl Default for AdminOverviewService {
    fn default() -> Self {
        Self::new(DEFAULT_PROBE_TIMEOUT)
    }
}

impl AdminOverviewService {
    pub fn new(probe_timeout: Duration) -> Self {
        Self {
            probes: Vec::new(),
            probe_timeout,
        }
    }

    /// Register a probe. Registrations past [`MAX_COMPONENTS`] are ignored.
    pub fn with_probe(mut self, probe: Arc<dyn OverviewProbe>) -> Self {
        if self.probes.len() < MAX_COMPONENTS {
            self.probes.push(probe);
        } else {
            tracing::warn!(
                probe = probe.name(),
                "admin overview component limit reached; probe ignored"
            );
        }
        self
    }

    pub fn component_count(&self) -> usize {
        self.probes.len()
    }

    /// Run all probes concurrently and assemble the aggregate snapshot.
    pub async fn snapshot(&self) -> AdminOverview {
        let timeout = self.probe_timeout;
        let mut runs = tokio::task::JoinSet::new();
        for (index, probe) in self.probes.iter().enumerate() {
            let probe = Arc::clone(probe);
            runs.spawn(async move {
                let outcome = tokio::time::timeout(timeout, probe.probe()).await;
                (
                    index,
                    build_component(probe.as_ref(), outcome, timeout, Utc::now()),
                )
            });
        }
        let mut slots: Vec<Option<ComponentOverview>> = vec![None; self.probes.len()];
        while let Some(joined) = runs.join_next().await {
            match joined {
                Ok((index, component)) => slots[index] = Some(component),
                Err(error) => tracing::warn!(error = %error, "admin overview probe task failed"),
            }
        }
        // A probe task that panicked is reported as unavailable, not dropped.
        let components: Vec<ComponentOverview> = slots
            .into_iter()
            .zip(self.probes.iter())
            .map(|(slot, probe)| {
                slot.unwrap_or_else(|| {
                    build_component(
                        probe.as_ref(),
                        Ok(Err("probe task failed".to_string())),
                        timeout,
                        Utc::now(),
                    )
                })
            })
            .collect();

        let overall_status = if components
            .iter()
            .all(|c| c.status == ComponentStatus::Healthy)
        {
            ComponentStatus::Healthy
        } else {
            ComponentStatus::Degraded
        };

        AdminOverview {
            generated_at: Utc::now(),
            overall_status,
            components,
        }
    }
}

fn build_component(
    probe: &dyn OverviewProbe,
    outcome: Result<Result<ProbeReport, String>, tokio::time::error::Elapsed>,
    timeout: Duration,
    checked_at: DateTime<Utc>,
) -> ComponentOverview {
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    let mut component = ComponentOverview {
        name: probe.name().to_string(),
        kind: probe.kind(),
        status: ComponentStatus::Unavailable,
        counts: BTreeMap::new(),
        oldest_item_at: None,
        checked_at,
        timeout_ms,
        detail: None,
    };
    match outcome {
        Ok(Ok(report)) => {
            component.status = report.status;
            component.oldest_item_at = report.oldest_item_at;
            for (key, value) in report.counts {
                if DOCUMENTED_COUNT_KEYS.contains(&key.as_str()) {
                    component.counts.insert(key, value);
                } else {
                    tracing::warn!(
                        probe = probe.name(),
                        key = %key,
                        "dropping undocumented admin overview count"
                    );
                }
            }
        }
        Ok(Err(error)) => {
            // The raw error may contain connection strings or record data;
            // it goes to the server log only.
            tracing::warn!(probe = probe.name(), error = %error, "admin overview probe failed");
            component.status = ComponentStatus::Unavailable;
            component.detail = Some("probe reported a failure; see server logs".to_string());
        }
        Err(_) => {
            tracing::warn!(
                probe = probe.name(),
                timeout_ms,
                "admin overview probe timed out"
            );
            component.status = ComponentStatus::Timeout;
            component.detail = Some(format!("no response within {timeout_ms} ms"));
        }
    }
    component
}

// ---------------------------------------------------------------------------
// Built-in probes
// ---------------------------------------------------------------------------

fn epoch_now() -> u64 {
    u64::try_from(Utc::now().timestamp()).unwrap_or(0)
}

fn seconds_ago(now: DateTime<Utc>, secs: Option<u64>) -> Option<DateTime<Utc>> {
    let secs = i64::try_from(secs?).ok()?;
    now.checked_sub_signed(ChronoDuration::seconds(secs))
}

/// Aggregates for one queue of the in-memory [`crate::job_queue::JobQueue`].
pub struct JobQueueProbe {
    name: String,
    queue: String,
    jobs: Arc<crate::job_queue::JobQueue>,
}

impl JobQueueProbe {
    pub fn new(queue: &str, jobs: Arc<crate::job_queue::JobQueue>) -> Self {
        Self {
            name: format!("queue:{queue}"),
            queue: queue.to_string(),
            jobs,
        }
    }
}

impl OverviewProbe for JobQueueProbe {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ComponentKind {
        ComponentKind::Queue
    }

    fn probe(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let now = Utc::now();
            let metrics = self
                .jobs
                .metrics(&self.queue, epoch_now())
                .map_err(|e| e.to_string())?;
            let status = match metrics.health {
                crate::job_queue::QueueHealth::Healthy => ComponentStatus::Healthy,
                crate::job_queue::QueueHealth::Degraded => ComponentStatus::Degraded,
            };
            Ok(ProbeReport::new(status)
                .count("waiting", metrics.waiting as u64)
                .count("delayed", metrics.delayed as u64)
                .count("active", metrics.active as u64)
                .count("completed", metrics.completed as u64)
                .count("dead", metrics.dead as u64)
                .count("throughput_per_min", metrics.throughput_per_min as u64)
                .oldest_item_at(seconds_ago(now, metrics.oldest_wait_secs)))
        })
    }
}

/// Aggregates for the transactional [`crate::outbox::InMemoryOutbox`].
pub struct OutboxProbe {
    outbox: crate::outbox::InMemoryOutbox,
    /// Pending events older than this mark the outbox `degraded`.
    lag_threshold_secs: u64,
}

impl OutboxProbe {
    pub fn new(outbox: crate::outbox::InMemoryOutbox, lag_threshold_secs: u64) -> Self {
        Self {
            outbox,
            lag_threshold_secs,
        }
    }
}

impl OverviewProbe for OutboxProbe {
    fn name(&self) -> &str {
        "outbox"
    }

    fn kind(&self) -> ComponentKind {
        ComponentKind::Outbox
    }

    fn probe(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let now = Utc::now();
            let stats = self.outbox.stats(epoch_now()).map_err(|e| e.to_string())?;
            let status = if stats.quarantined > 0 || stats.is_lagging(self.lag_threshold_secs) {
                ComponentStatus::Degraded
            } else {
                ComponentStatus::Healthy
            };
            Ok(ProbeReport::new(status)
                .count("pending", stats.pending as u64)
                .count("published", stats.published as u64)
                .count("quarantined", stats.quarantined as u64)
                .oldest_item_at(seconds_ago(now, stats.oldest_pending_age_secs)))
        })
    }
}

type CheckFn = dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync;

/// A dependency reachability check (database, RPC, cache, ...).
///
/// The check closure returns `Ok(())` when the dependency answers; the probe
/// records its latency.
pub struct DependencyProbe {
    name: String,
    check: Box<CheckFn>,
}

impl DependencyProbe {
    pub fn new<F, Fut>(name: &str, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        Self {
            name: format!("dependency:{name}"),
            check: Box::new(move || Box::pin(check())),
        }
    }
}

impl OverviewProbe for DependencyProbe {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ComponentKind {
        ComponentKind::Dependency
    }

    fn probe(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let started = Instant::now();
            (self.check)().await?;
            let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            Ok(ProbeReport::new(ComponentStatus::Healthy).count("latency_ms", latency_ms))
        })
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn problem(status: StatusCode, detail: &str) -> Response {
    let body = ProblemDetails::from_status(status.as_u16(), detail)
        .with_instance("/admin/overview")
        .to_json();
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

/// `GET /admin/overview` – administrator-only, read-only aggregate snapshot.
pub async fn admin_overview_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(service): Extension<Arc<AdminOverviewService>>,
) -> Response {
    if !user.is_admin() {
        tracing::warn!(
            role = ?user.role,
            "admin overview denied: administrator permission required"
        );
        return problem(
            StatusCode::FORBIDDEN,
            "The admin overview requires the administrator permission",
        );
    }

    let overview = service.snapshot().await;
    let mut response = (StatusCode::OK, Json(overview)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use axum::{body::Body, http::Request, routing::get, Router};
    use std::collections::BTreeSet;
    use tower::ServiceExt;

    struct FixedProbe {
        name: &'static str,
        report: Result<ProbeReport, String>,
        delay: Duration,
    }

    impl OverviewProbe for FixedProbe {
        fn name(&self) -> &str {
            self.name
        }
        fn kind(&self) -> ComponentKind {
            ComponentKind::Dependency
        }
        fn probe(&self) -> ProbeFuture<'_> {
            Box::pin(async move {
                tokio::time::sleep(self.delay).await;
                self.report.clone()
            })
        }
    }

    fn fixed(
        name: &'static str,
        report: Result<ProbeReport, String>,
        delay: Duration,
    ) -> Arc<dyn OverviewProbe> {
        Arc::new(FixedProbe {
            name,
            report,
            delay,
        })
    }

    fn user(role: Role) -> AuthenticatedUser {
        AuthenticatedUser::new(format!("G{}", role_label(role)), role, vec![])
    }

    fn role_label(role: Role) -> &'static str {
        match role {
            Role::Admin => "ADMIN",
            Role::Manager => "MANAGER",
            Role::Operator => "OPERATOR",
            Role::Viewer => "VIEWER",
        }
    }

    fn app(service: AdminOverviewService, caller: AuthenticatedUser) -> Router {
        Router::new()
            .route("/admin/overview", get(admin_overview_handler))
            .layer(Extension(Arc::new(service)))
            .layer(Extension(caller))
    }

    async fn call(router: Router) -> (StatusCode, Option<String>, serde_json::Value) {
        let response = router
            .oneshot(Request::get("/admin/overview").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string());
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            content_type,
            serde_json::from_slice(&bytes).unwrap(),
        )
    }

    fn sample_service() -> AdminOverviewService {
        let jobs = Arc::new(crate::job_queue::JobQueue::new());
        jobs.enqueue(
            crate::job_queue::JobSpec::new(
                "reports",
                "nightly",
                serde_json::json!({"customer": "secret@example.com"}),
            ),
            epoch_now(),
        )
        .unwrap();
        let outbox = crate::outbox::InMemoryOutbox::new();
        let mut tx = outbox.begin(epoch_now());
        tx.append(crate::outbox::NewEvent::new(
            "vault",
            "vault-123",
            "vault.created",
            serde_json::json!({"owner": "GPRIVATE"}),
            "k1",
        ));
        tx.commit().unwrap();

        AdminOverviewService::new(Duration::from_millis(200))
            .with_probe(Arc::new(JobQueueProbe::new("reports", jobs)))
            .with_probe(Arc::new(OutboxProbe::new(outbox, 300)))
            .with_probe(Arc::new(DependencyProbe::new("database", || async {
                Ok(())
            })))
    }

    #[tokio::test]
    async fn ordinary_users_are_denied() {
        for role in [Role::Manager, Role::Operator, Role::Viewer] {
            let (status, content_type, body) = call(app(sample_service(), user(role))).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{role:?} must be denied");
            assert_eq!(content_type.as_deref(), Some(PROBLEM_CONTENT_TYPE));
            assert_eq!(body["status"], 403);
            assert!(body.get("components").is_none());
        }
    }

    #[tokio::test]
    async fn admins_receive_only_documented_aggregates() {
        let (status, _, body) = call(app(sample_service(), user(Role::Admin))).await;
        assert_eq!(status, StatusCode::OK);

        let top: BTreeSet<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            top,
            BTreeSet::from(["components", "generated_at", "overall_status"])
        );
        assert!(body["generated_at"].as_str().unwrap().ends_with('Z'));

        let allowed_component_keys = BTreeSet::from([
            "name",
            "kind",
            "status",
            "counts",
            "oldest_item_at",
            "checked_at",
            "timeout_ms",
            "detail",
        ]);
        let components = body["components"].as_array().unwrap();
        assert_eq!(components.len(), 3);
        for component in components {
            for key in component.as_object().unwrap().keys() {
                assert!(
                    allowed_component_keys.contains(key.as_str()),
                    "unexpected key {key}"
                );
            }
            for (key, value) in component["counts"].as_object().unwrap() {
                assert!(
                    DOCUMENTED_COUNT_KEYS.contains(&key.as_str()),
                    "undocumented count {key}"
                );
                assert!(value.is_u64());
            }
        }

        let queue = &components[0];
        assert_eq!(queue["name"], "queue:reports");
        assert_eq!(queue["kind"], "queue");
        assert_eq!(queue["counts"]["waiting"], 1);
        let outbox = &components[1];
        assert_eq!(outbox["counts"]["pending"], 1);
        assert!(outbox["oldest_item_at"].as_str().unwrap().ends_with('Z'));

        // Payload contents of jobs and events never appear.
        let rendered = body.to_string();
        for secret in ["secret@example.com", "GPRIVATE", "vault-123", "nightly"] {
            assert!(!rendered.contains(secret), "leaked {secret}");
        }
    }

    #[tokio::test]
    async fn undocumented_counts_are_dropped() {
        let service = AdminOverviewService::new(Duration::from_millis(200)).with_probe(fixed(
            "dependency:cache",
            Ok(ProbeReport::new(ComponentStatus::Healthy)
                .count("latency_ms", 3)
                .count("customer_emails", 7)),
            Duration::ZERO,
        ));
        let overview = service.snapshot().await;
        let counts = &overview.components[0].counts;
        assert_eq!(counts.get("latency_ms"), Some(&3));
        assert!(!counts.contains_key("customer_emails"));
    }

    #[tokio::test]
    async fn downstream_timeouts_are_reported_explicitly() {
        let service = AdminOverviewService::new(Duration::from_millis(50))
            .with_probe(fixed(
                "dependency:rpc",
                Ok(ProbeReport::new(ComponentStatus::Healthy)),
                Duration::from_secs(5),
            ))
            .with_probe(fixed(
                "dependency:database",
                Ok(ProbeReport::new(ComponentStatus::Healthy).count("latency_ms", 1)),
                Duration::ZERO,
            ));

        let started = Instant::now();
        let (status, _, body) = call(app(service, user(Role::Admin))).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a slow probe must not stall the request"
        );
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["overall_status"], "degraded");

        let rpc = &body["components"][0];
        assert_eq!(rpc["status"], "timeout");
        assert_eq!(rpc["timeout_ms"], 50);
        assert_eq!(rpc["detail"], "no response within 50 ms");
        assert!(rpc["counts"].as_object().unwrap().is_empty());

        assert_eq!(body["components"][1]["status"], "healthy");
    }

    #[tokio::test]
    async fn probe_failures_do_not_echo_internal_errors() {
        let service = AdminOverviewService::new(Duration::from_millis(100)).with_probe(fixed(
            "dependency:database",
            Err("connect postgres://admin:hunter2@db/prod failed".to_string()),
            Duration::ZERO,
        ));
        let overview = service.snapshot().await;
        let rendered = serde_json::to_string(&overview).unwrap();
        assert_eq!(overview.components[0].status, ComponentStatus::Unavailable);
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("postgres://"));
    }

    #[tokio::test]
    async fn the_component_list_is_bounded() {
        let mut service = AdminOverviewService::new(Duration::from_millis(10));
        for _ in 0..(MAX_COMPONENTS + 5) {
            service = service.with_probe(fixed(
                "dependency:x",
                Ok(ProbeReport::new(ComponentStatus::Healthy)),
                Duration::ZERO,
            ));
        }
        assert_eq!(service.component_count(), MAX_COMPONENTS);
        assert_eq!(service.snapshot().await.components.len(), MAX_COMPONENTS);
    }

    #[tokio::test]
    async fn all_healthy_components_give_a_healthy_overall_status() {
        let service = AdminOverviewService::new(Duration::from_millis(100)).with_probe(Arc::new(
            DependencyProbe::new("database", || async { Ok(()) }),
        ));
        let overview = service.snapshot().await;
        assert_eq!(overview.overall_status, ComponentStatus::Healthy);
        assert!(overview.components[0].counts.contains_key("latency_ms"));
    }
}
