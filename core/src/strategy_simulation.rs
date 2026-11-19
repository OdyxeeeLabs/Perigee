//! Read-only strategy simulation (API-29).
//!
//! `POST /strategies/:id/simulate` previews how an **approved** strategy would
//! react to caller-supplied market inputs before execution is requested.
//!
//! * **Side-effect free.** The simulation copies the strategy's approved
//!   thresholds into a fresh [`HysteresisGuard`] (the same trigger engine the
//!   live strategy uses) and reads the vault position. It never writes the
//!   strategy's live trigger state, the vault position, or anything else.
//! * **Bounded and validated.** At most [`MAX_SCENARIO_STEPS`] steps, finite
//!   positive prices in a fixed range, a matching asset code, label limits,
//!   and a market-data timestamp that is neither in the future nor stale.
//!   Unknown fields are rejected.
//! * **Clearly an estimate.** Every response has `kind: "simulation_estimate"`,
//!   `status: "simulated"`, `ledger_effect: "none"`, a disclaimer, the
//!   assumptions used, and the timestamps of every data source. It never
//!   carries a transaction hash or ledger sequence.
//!
//! See `docs/STRATEGY_SIMULATION.md`.

use crate::auth::AuthenticatedUser;
use crate::hysteresis::{HysteresisGuard, TriggerEvent, TriggerState};
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use axum::{
    extract::{rejection::JsonRejection, Path},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub const MAX_SCENARIO_STEPS: usize = 100;
/// Prices are fixed-point with 7 decimals (Stellar convention).
pub const PRICE_SCALE: i128 = 10_000_000;
pub const MAX_PRICE_E7: i64 = 1_000_000_000_000_000;
/// Market data older than this is rejected as stale.
pub const MAX_MARKET_DATA_AGE_HOURS: i64 = 24;
/// Allowed clock skew for `market_data_as_of`.
pub const MAX_FUTURE_SKEW_SECS: i64 = 300;
const MAX_LABEL_LEN: usize = 64;
const DISCLAIMER: &str = "Estimate only. This is not an on-chain transaction, has not been submitted, and is not a financial guarantee.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Draft,
    Approved,
    Suspended,
}

/// A strategy definition as approved for a vault.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovedStrategy {
    pub strategy_id: String,
    pub vault_id: String,
    pub manager_id: String,
    pub asset: String,
    pub approval: ApprovalStatus,
    /// Drawdown (basis points below the reference price) that triggers `on_enter`.
    pub enter_drawdown_bps: u32,
    /// Drawdown at or below which the trigger releases (`on_exit`).
    pub exit_drawdown_bps: u32,
    pub on_enter: String,
    pub on_exit: String,
    /// The live trigger state. Simulation must never change it.
    pub live_state: TriggerState,
}

/// The vault's recorded position in the strategy asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultPosition {
    pub balance_stroops: i64,
    pub as_of: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    #[serde(default)]
    pub label: Option<String>,
    pub price_e7: i64,
}

/// Body of `POST /strategies/:id/simulate`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationRequest {
    pub asset: String,
    pub reference_price_e7: i64,
    pub market_data_as_of: DateTime<Utc>,
    pub scenario: Vec<ScenarioStep>,
}

fn rfc3339<S: Serializer>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DataTimestamps {
    #[serde(serialize_with = "rfc3339")]
    pub market_data_as_of: DateTime<Utc>,
    #[serde(serialize_with = "rfc3339")]
    pub position_as_of: DateTime<Utc>,
    #[serde(serialize_with = "rfc3339")]
    pub simulated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SimulatedStep {
    pub index: usize,
    pub label: Option<String>,
    pub price_e7: i64,
    /// Drawdown versus the reference price in basis points (negative = above it).
    pub drawdown_bps: i64,
    /// `entered` / `exited` when the trigger would fire at this step.
    pub trigger: Option<&'static str>,
    /// The approved action that would be *requested* (not executed).
    pub would_request_action: Option<String>,
    pub strategy_state: &'static str,
    /// Estimated position value at this price, stroops (string: may exceed 2^53).
    pub estimated_value_stroops: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SimulationSummary {
    pub triggers_fired: usize,
    pub final_strategy_state: &'static str,
    pub min_estimated_value_stroops: String,
    pub max_estimated_value_stroops: String,
}

/// The simulation response. Every field marks it as an estimate.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SimulationEstimate {
    pub kind: &'static str,
    pub status: &'static str,
    pub ledger_effect: &'static str,
    pub disclaimer: &'static str,
    pub strategy_id: String,
    pub vault_id: String,
    pub asset: String,
    pub assumptions: Vec<String>,
    pub data_timestamps: DataTimestamps,
    pub steps: Vec<SimulatedStep>,
    pub summary: SimulationSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulationError {
    NotFound,
    NotApproved,
    Invalid(Vec<(String, String)>),
}

#[derive(Default)]
struct Inner {
    strategies: HashMap<String, ApprovedStrategy>,
    positions: HashMap<String, VaultPosition>,
}

/// Holds approved strategies and positions; simulation only reads them.
#[derive(Default)]
pub struct StrategySimulationService {
    inner: RwLock<Inner>,
}

fn state_name(state: TriggerState) -> &'static str {
    match state {
        TriggerState::Below => "idle",
        TriggerState::Above => "triggered",
    }
}

fn valid_asset(code: &str) -> bool {
    (1..=12).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl StrategySimulationService {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn upsert_strategy(&self, strategy: ApprovedStrategy) {
        self.write()
            .strategies
            .insert(strategy.strategy_id.clone(), strategy);
    }

    pub fn set_position(&self, vault_id: &str, position: VaultPosition) {
        self.write()
            .positions
            .insert(vault_id.to_string(), position);
    }

    pub fn strategy(&self, strategy_id: &str) -> Option<ApprovedStrategy> {
        self.read().strategies.get(strategy_id).cloned()
    }

    pub fn position(&self, vault_id: &str) -> Option<VaultPosition> {
        self.read().positions.get(vault_id).cloned()
    }

    /// Validate a request against the strategy without touching any state.
    pub fn validate(
        strategy: &ApprovedStrategy,
        request: &SimulationRequest,
        now: DateTime<Utc>,
    ) -> Result<(), SimulationError> {
        let mut errors = Vec::new();
        let mut fail = |field: &str, message: String| errors.push((field.to_string(), message));

        if !valid_asset(&request.asset) {
            fail("asset", "asset must be 1-12 alphanumeric characters".into());
        } else if !request.asset.eq_ignore_ascii_case(&strategy.asset) {
            fail(
                "asset",
                format!(
                    "this strategy trades {}, not {}",
                    strategy.asset, request.asset
                ),
            );
        }
        if !(1..=MAX_PRICE_E7).contains(&request.reference_price_e7) {
            fail(
                "reference_price_e7",
                format!("must be between 1 and {MAX_PRICE_E7}"),
            );
        }
        if request.market_data_as_of > now + Duration::seconds(MAX_FUTURE_SKEW_SECS) {
            fail("market_data_as_of", "must not be in the future".into());
        } else if request.market_data_as_of < now - Duration::hours(MAX_MARKET_DATA_AGE_HOURS) {
            fail(
                "market_data_as_of",
                format!("market data older than {MAX_MARKET_DATA_AGE_HOURS} hours is too stale to simulate"),
            );
        }
        if request.scenario.is_empty() {
            fail("scenario", "provide at least one step".into());
        } else if request.scenario.len() > MAX_SCENARIO_STEPS {
            fail(
                "scenario",
                format!(
                    "at most {MAX_SCENARIO_STEPS} steps are allowed, got {}",
                    request.scenario.len()
                ),
            );
        } else {
            for (index, step) in request.scenario.iter().enumerate() {
                if !(1..=MAX_PRICE_E7).contains(&step.price_e7) {
                    fail(
                        &format!("scenario[{index}].price_e7"),
                        format!("must be between 1 and {MAX_PRICE_E7}"),
                    );
                }
                if let Some(label) = &step.label {
                    if label.chars().count() > MAX_LABEL_LEN || label.chars().any(char::is_control)
                    {
                        fail(
                            &format!("scenario[{index}].label"),
                            format!("must be at most {MAX_LABEL_LEN} printable characters"),
                        );
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(SimulationError::Invalid(errors))
        }
    }

    /// Run a simulation for `caller`. Reads state; never writes it.
    pub fn simulate(
        &self,
        strategy_id: &str,
        caller: &str,
        request: &SimulationRequest,
        now: DateTime<Utc>,
    ) -> Result<SimulationEstimate, SimulationError> {
        let (strategy, position) = {
            let inner = self.read();
            let strategy = inner
                .strategies
                .get(strategy_id)
                .filter(|s| s.manager_id == caller)
                .cloned()
                .ok_or(SimulationError::NotFound)?;
            let position =
                inner
                    .positions
                    .get(&strategy.vault_id)
                    .cloned()
                    .unwrap_or(VaultPosition {
                        balance_stroops: 0,
                        as_of: now,
                    });
            (strategy, position)
        };
        if strategy.approval != ApprovalStatus::Approved {
            return Err(SimulationError::NotApproved);
        }
        Self::validate(&strategy, request, now)?;

        // A fresh guard with the approved thresholds; the live one is untouched.
        let mut guard = HysteresisGuard::new(
            f64::from(strategy.enter_drawdown_bps),
            f64::from(strategy.exit_drawdown_bps),
        );
        let reference = i128::from(request.reference_price_e7);
        let balance = i128::from(position.balance_stroops);
        let mut steps = Vec::with_capacity(request.scenario.len());
        let mut triggers_fired = 0;
        let mut min_value = i128::MAX;
        let mut max_value = i128::MIN;

        for (index, step) in request.scenario.iter().enumerate() {
            let price = i128::from(step.price_e7);
            let drawdown_bps =
                i64::try_from((reference - price) * 10_000 / reference).unwrap_or(i64::MAX);
            let event = guard.evaluate(drawdown_bps as f64);
            let (trigger, action) = match event {
                Some(TriggerEvent::Entered) => (Some("entered"), Some(strategy.on_enter.clone())),
                Some(TriggerEvent::Exited) => (Some("exited"), Some(strategy.on_exit.clone())),
                None => (None, None),
            };
            if trigger.is_some() {
                triggers_fired += 1;
            }
            let value = balance.saturating_mul(price) / PRICE_SCALE;
            min_value = min_value.min(value);
            max_value = max_value.max(value);
            steps.push(SimulatedStep {
                index,
                label: step.label.clone(),
                price_e7: step.price_e7,
                drawdown_bps,
                trigger,
                would_request_action: action,
                strategy_state: state_name(guard.current_state()),
                estimated_value_stroops: value.to_string(),
            });
        }

        Ok(SimulationEstimate {
            kind: "simulation_estimate",
            status: "simulated",
            ledger_effect: "none",
            disclaimer: DISCLAIMER,
            strategy_id: strategy.strategy_id.clone(),
            vault_id: strategy.vault_id.clone(),
            asset: strategy.asset.clone(),
            assumptions: vec![
                "Prices are the caller-supplied scenario values, applied in order.".into(),
                "Drawdown is measured against the supplied reference price.".into(),
                "Triggers use the strategy's approved thresholds, starting from an untriggered state.".into(),
                "The vault's recorded position is held constant; fees, slippage, liquidity and execution delay are not modelled.".into(),
                "Actions are what the strategy would request; nothing is submitted or executed.".into(),
            ],
            data_timestamps: DataTimestamps {
                market_data_as_of: request.market_data_as_of,
                position_as_of: position.as_of,
                simulated_at: now,
            },
            summary: SimulationSummary {
                triggers_fired,
                final_strategy_state: state_name(guard.current_state()),
                min_estimated_value_stroops: min_value.to_string(),
                max_estimated_value_stroops: max_value.to_string(),
            },
            steps,
        })
    }
}

fn problem(status: StatusCode, detail: &str, errors: Option<&[(String, String)]>) -> Response {
    let mut body = ProblemDetails::from_status(status.as_u16(), detail);
    if let Some(errors) = errors {
        let list: Vec<(&str, &str, &str)> = errors
            .iter()
            .map(|(field, message)| (field.as_str(), "invalid", message.as_str()))
            .collect();
        body = body.with_field_errors(&list);
    }
    let mut response = (status, Json(body.to_json())).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

/// `POST /strategies/:id/simulate`
pub async fn simulate_strategy_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(service): Extension<Arc<StrategySimulationService>>,
    Path(strategy_id): Path<String>,
    body: Result<Json<SimulationRequest>, JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return problem(
                StatusCode::BAD_REQUEST,
                &format!("malformed scenario: {}", rejection.body_text()),
                None,
            )
        }
    };
    match service.simulate(&strategy_id, &user.stellar_address, &request, Utc::now()) {
        Ok(estimate) => {
            if !user.can_access_vault(&estimate.vault_id) {
                return problem(StatusCode::NOT_FOUND, "strategy not found", None);
            }
            let mut response = Json(estimate).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(SimulationError::NotFound) => {
            problem(StatusCode::NOT_FOUND, "strategy not found", None)
        }
        Err(SimulationError::NotApproved) => problem(
            StatusCode::CONFLICT,
            "only approved strategies can be simulated",
            None,
        ),
        Err(SimulationError::Invalid(errors)) => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the scenario failed validation",
            Some(&errors),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use axum::{body::Body, http::Request, routing::post, Router};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    const ALICE: &str = "GALICE";
    const BOB: &str = "GBOB";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-19T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn strategy(id: &str, owner: &str, approval: ApprovalStatus) -> ApprovedStrategy {
        ApprovedStrategy {
            strategy_id: id.into(),
            vault_id: format!("vault-{id}"),
            manager_id: owner.into(),
            asset: "XLM".into(),
            approval,
            enter_drawdown_bps: 1_000,
            exit_drawdown_bps: 300,
            on_enter: "rotate_to_stable".into(),
            on_exit: "rotate_back".into(),
            live_state: TriggerState::Below,
        }
    }

    fn service() -> StrategySimulationService {
        let service = StrategySimulationService::new();
        service.upsert_strategy(strategy("s1", ALICE, ApprovalStatus::Approved));
        service.upsert_strategy(strategy("draft", ALICE, ApprovalStatus::Draft));
        service.upsert_strategy(strategy("bob", BOB, ApprovalStatus::Approved));
        service.set_position(
            "vault-s1",
            VaultPosition {
                balance_stroops: 1_000 * PRICE_SCALE as i64,
                as_of: now() - Duration::minutes(5),
            },
        );
        service
    }

    fn request(prices: &[i64]) -> SimulationRequest {
        SimulationRequest {
            asset: "XLM".into(),
            reference_price_e7: 1_200_000,
            market_data_as_of: now() - Duration::minutes(1),
            scenario: prices
                .iter()
                .map(|p| ScenarioStep {
                    label: None,
                    price_e7: *p,
                })
                .collect(),
        }
    }

    #[test]
    fn simulation_reports_triggers_and_estimates() {
        let service = service();
        // 1.20 -> 1.14 (5% down) -> 1.02 (15% down: enter) -> 1.18 (1.7% down: exit)
        let estimate = service
            .simulate(
                "s1",
                ALICE,
                &request(&[1_140_000, 1_020_000, 1_180_000]),
                now(),
            )
            .unwrap();
        let triggers: Vec<_> = estimate.steps.iter().map(|s| s.trigger).collect();
        assert_eq!(triggers, vec![None, Some("entered"), Some("exited")]);
        assert_eq!(estimate.steps[1].drawdown_bps, 1_500);
        assert_eq!(
            estimate.steps[1].would_request_action.as_deref(),
            Some("rotate_to_stable")
        );
        // 1000 XLM at 0.102 XLM-quote = 102 units = 1_020_000_000 stroops.
        assert_eq!(estimate.steps[1].estimated_value_stroops, "1020000000");
        assert_eq!(estimate.summary.triggers_fired, 2);
        assert_eq!(estimate.summary.final_strategy_state, "idle");
    }

    #[test]
    fn simulation_does_not_mutate_vault_or_strategy_state() {
        let service = service();
        let strategy_before = service.strategy("s1").unwrap();
        let position_before = service.position("vault-s1").unwrap();
        // This scenario ends in the triggered state.
        let estimate = service
            .simulate("s1", ALICE, &request(&[900_000]), now())
            .unwrap();
        assert_eq!(estimate.summary.final_strategy_state, "triggered");
        assert_eq!(service.strategy("s1").unwrap(), strategy_before);
        assert_eq!(
            service.strategy("s1").unwrap().live_state,
            TriggerState::Below
        );
        assert_eq!(service.position("vault-s1").unwrap(), position_before);
        // Re-running gives the same answer: no hidden state carried over.
        let again = service
            .simulate("s1", ALICE, &request(&[900_000]), now())
            .unwrap();
        assert_eq!(again.steps, estimate.steps);
    }

    #[test]
    fn malformed_or_oversized_scenarios_are_rejected() {
        let service = service();
        let too_many = request(&vec![1_000_000; MAX_SCENARIO_STEPS + 1]);
        let empty = request(&[]);
        let bad_prices = request(&[0, -5, MAX_PRICE_E7 + 1]);
        let mut wrong_asset = request(&[1_000_000]);
        wrong_asset.asset = "USDC".into();
        let mut bad_asset = request(&[1_000_000]);
        bad_asset.asset = "XLM;DROP".into();
        let mut stale = request(&[1_000_000]);
        stale.market_data_as_of = now() - Duration::hours(MAX_MARKET_DATA_AGE_HOURS + 1);
        let mut future = request(&[1_000_000]);
        future.market_data_as_of = now() + Duration::hours(1);
        let mut zero_reference = request(&[1_000_000]);
        zero_reference.reference_price_e7 = 0;
        let mut long_label = request(&[1_000_000]);
        long_label.scenario[0].label = Some("x".repeat(65));

        for (name, req) in [
            ("too many", too_many),
            ("empty", empty),
            ("bad prices", bad_prices),
            ("wrong asset", wrong_asset),
            ("bad asset", bad_asset),
            ("stale", stale),
            ("future", future),
            ("zero reference", zero_reference),
            ("long label", long_label),
        ] {
            assert!(
                matches!(
                    service.simulate("s1", ALICE, &req, now()),
                    Err(SimulationError::Invalid(_))
                ),
                "{name} must be rejected"
            );
        }
        let at_limit = request(&vec![1_000_000; MAX_SCENARIO_STEPS]);
        assert!(service.simulate("s1", ALICE, &at_limit, now()).is_ok());
    }

    #[test]
    fn only_the_owners_approved_strategies_can_be_simulated() {
        let service = service();
        assert_eq!(
            service
                .simulate("bob", ALICE, &request(&[1_000_000]), now())
                .unwrap_err(),
            SimulationError::NotFound
        );
        assert_eq!(
            service
                .simulate("draft", ALICE, &request(&[1_000_000]), now())
                .unwrap_err(),
            SimulationError::NotApproved
        );
    }

    fn app(service: Arc<StrategySimulationService>, user: AuthenticatedUser) -> Router {
        Router::new()
            .route("/strategies/:id/simulate", post(simulate_strategy_handler))
            .layer(Extension(service))
            .layer(Extension(user))
    }

    async fn send(router: Router, uri: &str, body: String) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::post(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn the_response_is_labelled_as_an_estimate_not_a_ledger_result() {
        let service = Arc::new(service());
        let alice = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);
        let as_of = (Utc::now() - Duration::minutes(1)).to_rfc3339();
        let body = json!({
            "asset": "XLM",
            "reference_price_e7": 1_200_000,
            "market_data_as_of": as_of,
            "scenario": [{"label": "crash", "price_e7": 1_000_000}]
        });
        let (status, json) = send(
            app(service.clone(), alice.clone()),
            "/strategies/s1/simulate",
            body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["kind"], "simulation_estimate");
        assert_eq!(json["status"], "simulated");
        assert_eq!(json["ledger_effect"], "none");
        assert!(json["disclaimer"]
            .as_str()
            .unwrap()
            .contains("not an on-chain transaction"));
        assert!(!json["assumptions"].as_array().unwrap().is_empty());
        for field in ["market_data_as_of", "position_as_of", "simulated_at"] {
            assert!(
                json["data_timestamps"][field]
                    .as_str()
                    .unwrap()
                    .ends_with('Z'),
                "{field}"
            );
        }
        let rendered = json.to_string();
        for confirmed in [
            "transaction_hash",
            "ledger_sequence",
            "\"finalized\"",
            "\"submitted\"",
        ] {
            assert!(!rendered.contains(confirmed), "{confirmed} must not appear");
        }
        assert_eq!(json["steps"][0]["label"], "crash");
        assert_eq!(json["steps"][0]["trigger"], "entered");

        // Malformed JSON and unknown fields.
        let (status, _) = send(
            app(service.clone(), alice.clone()),
            "/strategies/s1/simulate",
            "{not json".into(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let mut extra = body.clone();
        extra["execute"] = json!(true);
        let (status, _) = send(
            app(service.clone(), alice.clone()),
            "/strategies/s1/simulate",
            extra.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Validation errors are field-level problems.
        let mut oversized = body.clone();
        oversized["scenario"] = Value::Array(vec![json!({"price_e7": 1}); MAX_SCENARIO_STEPS + 1]);
        let (status, json) = send(
            app(service.clone(), alice),
            "/strategies/s1/simulate",
            oversized.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json["errors"][0]["field"], "scenario");

        // Other tenants' strategies are not found.
        let bob = AuthenticatedUser::new(BOB.into(), Role::Manager, vec![]);
        let (status, _) = send(
            app(service, bob),
            "/strategies/s1/simulate",
            body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
