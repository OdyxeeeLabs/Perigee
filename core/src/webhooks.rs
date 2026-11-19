//! # Webhook delivery
//!
//! Reliable outbound webhooks for external integrations.
//!
//! * **Signing.** Every request carries `X-Perigee-Signature: t=<unix>,v1=<hex>`
//!   where `v1` is HMAC-SHA256 over `"<t>.<body>"` with the endpoint secret.
//!   Receivers recompute it, compare in constant time and reject stale
//!   timestamps ([`verify_signature`]), which also stops replays.
//! * **Retries.** A failed delivery (transport error or non-2xx) is retried after
//!   1 minute, 5 minutes, 15 minutes, 1 hour, 6 hours and 24 hours
//!   ([`RETRY_SCHEDULE_SECS`]). After the last retry it is dead-lettered.
//! * **Idempotency.** `X-Perigee-Event-Id` is stable across retries so receivers
//!   can deduplicate, and publishing the same event id twice to an endpoint
//!   creates one delivery, not two.
//! * **Delivery logs and replay.** Each attempt (time, status code, error) is
//!   kept; any delivery can be replayed by an admin.
//! * **Endpoints** have custom headers, an optional event-type filter, an
//!   enable switch and an optional payload transformation. Targets must be
//!   public HTTPS URLs ([`validate_endpoint_url`]).
//!
//! The HTTP client is the [`Transport`] trait so the delivery logic is testable
//! and the service can choose its client. See `docs/WEBHOOKS.md`.

use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

pub const SIGNATURE_HEADER: &str = "X-Perigee-Signature";
pub const EVENT_ID_HEADER: &str = "X-Perigee-Event-Id";
pub const DELIVERY_ID_HEADER: &str = "X-Perigee-Delivery-Id";
/// Wait before retry 1..=6 (1m, 5m, 15m, 1h, 6h, 24h).
pub const RETRY_SCHEDULE_SECS: [u64; 6] = [60, 300, 900, 3600, 6 * 3600, 24 * 3600];
/// How old a signed timestamp may be before a receiver should reject it.
pub const DEFAULT_TOLERANCE_SECS: u64 = 300;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebhookError {
    #[error("invalid endpoint url: {0}")]
    InvalidUrl(&'static str),
    #[error("unknown endpoint `{0}`")]
    UnknownEndpoint(String),
    #[error("unknown delivery `{0}`")]
    UnknownDelivery(String),
    #[error("webhook state unavailable")]
    Poisoned,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("malformed signature header")]
    Malformed,
    #[error("timestamp outside the tolerance window")]
    Stale,
    #[error("signature does not match")]
    Mismatch,
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

fn mac(secret: &[u8], timestamp: u64, body: &[u8]) -> Option<HmacSha256> {
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    Some(mac)
}

/// Value for the [`SIGNATURE_HEADER`].
pub fn signature_header(secret: &[u8], timestamp: u64, body: &[u8]) -> String {
    let tag = mac(secret, timestamp, body)
        .map(|m| hex::encode(m.finalize().into_bytes()))
        .unwrap_or_default();
    format!("t={timestamp},v1={tag}")
}

/// What a receiver runs: checks the timestamp window and the HMAC.
pub fn verify_signature(
    secret: &[u8],
    header: &str,
    body: &[u8],
    now: u64,
    tolerance_secs: u64,
) -> Result<(), SignatureError> {
    let mut timestamp = None;
    let mut candidates = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", value)) => timestamp = value.parse::<u64>().ok(),
            Some(("v1", value)) => candidates.push(value),
            _ => {}
        }
    }
    let timestamp = timestamp.ok_or(SignatureError::Malformed)?;
    if candidates.is_empty() {
        return Err(SignatureError::Malformed);
    }
    if now.abs_diff(timestamp) > tolerance_secs {
        return Err(SignatureError::Stale);
    }

    for candidate in candidates {
        let Ok(tag) = hex::decode(candidate) else { continue };
        if let Some(expected) = mac(secret, timestamp, body) {
            // `verify_slice` compares in constant time.
            if expected.verify_slice(&tag).is_ok() {
                return Ok(());
            }
        }
    }
    Err(SignatureError::Mismatch)
}

/// A fresh signing secret.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("whsec_{}", hex::encode(bytes))
}

// ---------------------------------------------------------------------------
// Endpoint validation
// ---------------------------------------------------------------------------

fn is_private_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".internal") || host == "::1" {
        return true;
    }
    if host.starts_with("fc") || host.starts_with("fd") || host.starts_with("fe80") {
        return host.contains(':');
    }
    let octets: Vec<u8> = host.split('.').filter_map(|p| p.parse().ok()).collect();
    if octets.len() == 4 {
        return matches!(
            octets.as_slice(),
            [10, ..] | [127, ..] | [0, ..] | [169, 254, ..] | [192, 168, ..]
        ) || (octets[0] == 172 && (16..=31).contains(&octets[1]));
    }
    false
}

/// Webhook targets must be public `https` URLs. Hostnames that merely *resolve*
/// to private addresses cannot be caught without DNS, so the HTTP client should
/// also refuse private addresses at connect time.
pub fn validate_endpoint_url(url: &str) -> Result<(), WebhookError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or(WebhookError::InvalidUrl("must use https"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(WebhookError::InvalidUrl("must not contain credentials"));
    }
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err(WebhookError::InvalidUrl("missing host"));
    }
    if is_private_host(host) {
        return Err(WebhookError::InvalidUrl("private or local addresses are not allowed"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

pub type PayloadTransform = Arc<dyn Fn(&Value) -> Value + Send + Sync>;

#[derive(Clone)]
pub struct Endpoint {
    pub id: String,
    pub url: String,
    pub secret: String,
    /// Retiring secret and its expiry (unix secs) during a key rotation.
    pub retiring: Option<(String, u64)>,
    pub headers: Vec<(String, String)>,
    /// Event types to receive; empty means all.
    pub events: Vec<String>,
    pub enabled: bool,
    pub transform: Option<PayloadTransform>,
}

impl Endpoint {
    pub fn new(id: &str, url: &str, secret: &str) -> Self {
        Endpoint {
            id: id.to_string(),
            url: url.to_string(),
            secret: secret.to_string(),
            retiring: None,
            headers: Vec::new(),
            events: Vec::new(),
            enabled: true,
            transform: None,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn events(mut self, events: &[&str]) -> Self {
        self.events = events.iter().map(|e| e.to_string()).collect();
        self
    }

    pub fn transform(mut self, transform: impl Fn(&Value) -> Value + Send + Sync + 'static) -> Self {
        self.transform = Some(Arc::new(transform));
        self
    }

    fn wants(&self, event_type: &str) -> bool {
        self.enabled && (self.events.is_empty() || self.events.iter().any(|e| e == event_type))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// Waiting for its first (or a replayed) attempt.
    Pending,
    Delivered,
    /// A retry is scheduled.
    Retrying,
    /// Out of retries; replay by hand.
    DeadLettered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attempt {
    pub number: u32,
    pub at: u64,
    pub status_code: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Delivery {
    pub id: String,
    pub endpoint_id: String,
    pub event_id: String,
    pub event_type: String,
    pub payload: Value,
    pub status: DeliveryStatus,
    pub attempts: Vec<Attempt>,
    pub next_attempt_at: Option<u64>,
    pub created_at: u64,
    /// Failures since the last success/replay; picks the next retry delay.
    #[serde(skip)]
    cycle_failures: usize,
}

/// Sends one HTTP POST and returns the status code.
pub trait Transport: Send + Sync {
    fn post(&self, url: &str, headers: &[(String, String)], body: &[u8]) -> Result<u16, String>;
}

#[derive(Default)]
struct State {
    next_id: u64,
    endpoints: HashMap<String, Endpoint>,
    deliveries: Vec<Delivery>,
}

pub struct WebhookService<T: Transport> {
    transport: T,
    state: Mutex<State>,
}

impl<T: Transport> WebhookService<T> {
    pub fn new(transport: T) -> Self {
        WebhookService {
            transport,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, WebhookError> {
        self.state.lock().map_err(|_| WebhookError::Poisoned)
    }

    pub fn register_endpoint(&self, endpoint: Endpoint) -> Result<(), WebhookError> {
        validate_endpoint_url(&endpoint.url)?;
        self.state()?.endpoints.insert(endpoint.id.clone(), endpoint);
        Ok(())
    }

    pub fn set_enabled(&self, endpoint_id: &str, enabled: bool) -> Result<(), WebhookError> {
        self.state()?
            .endpoints
            .get_mut(endpoint_id)
            .map(|e| e.enabled = enabled)
            .ok_or_else(|| WebhookError::UnknownEndpoint(endpoint_id.to_string()))
    }

    /// Queue `event` for every subscribed endpoint. Publishing the same
    /// `event_id` again returns the existing deliveries instead of duplicating.
    pub fn publish(&self, event_type: &str, event_id: &str, payload: Value, now: u64) -> Result<Vec<String>, WebhookError> {
        let mut state = self.state()?;
        let mut targets: Vec<String> = state
            .endpoints
            .values()
            .filter(|e| e.wants(event_type))
            .map(|e| e.id.clone())
            .collect();
        targets.sort();

        let mut ids = Vec::new();
        for endpoint_id in targets {
            if let Some(existing) = state
                .deliveries
                .iter()
                .find(|d| d.endpoint_id == endpoint_id && d.event_id == event_id)
            {
                ids.push(existing.id.clone());
                continue;
            }
            state.next_id += 1;
            let id = format!("dlv_{}", state.next_id);
            state.deliveries.push(Delivery {
                id: id.clone(),
                endpoint_id,
                event_id: event_id.to_string(),
                event_type: event_type.to_string(),
                payload: payload.clone(),
                status: DeliveryStatus::Pending,
                attempts: Vec::new(),
                next_attempt_at: Some(now),
                created_at: now,
                cycle_failures: 0,
            });
            ids.push(id);
        }
        Ok(ids)
    }

    /// Attempt every delivery that is due. Returns how many were attempted.
    pub fn run_due(&self, now: u64) -> Result<usize, WebhookError> {
        let due: Vec<(String, Delivery, Endpoint)> = {
            let state = self.state()?;
            state
                .deliveries
                .iter()
                .filter(|d| {
                    matches!(d.status, DeliveryStatus::Pending | DeliveryStatus::Retrying)
                        && d.next_attempt_at.is_some_and(|at| at <= now)
                })
                .filter_map(|d| {
                    let endpoint = state.endpoints.get(&d.endpoint_id)?;
                    endpoint.enabled.then(|| (d.id.clone(), d.clone(), endpoint.clone()))
                })
                .collect()
        };

        // The network calls run without holding the lock.
        let mut handled = 0;
        for (id, delivery, endpoint) in due {
            let outcome = self.attempt(&delivery, &endpoint, now);
            let mut state = self.state()?;
            if let Some(stored) = state.deliveries.iter_mut().find(|d| d.id == id) {
                Self::record(stored, outcome, now);
            }
            handled += 1;
        }
        Ok(handled)
    }

    fn attempt(&self, delivery: &Delivery, endpoint: &Endpoint, now: u64) -> Result<u16, String> {
        let data = match &endpoint.transform {
            Some(transform) => transform(&delivery.payload),
            None => delivery.payload.clone(),
        };
        let body = serde_json::to_vec(&json!({
            "id": delivery.event_id,
            "type": delivery.event_type,
            "created_at": delivery.created_at,
            "data": data,
        }))
        .map_err(|e| e.to_string())?;

        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("User-Agent".to_string(), "Perigee-Webhooks/1".to_string()),
            (EVENT_ID_HEADER.to_string(), delivery.event_id.clone()),
            (DELIVERY_ID_HEADER.to_string(), delivery.id.clone()),
            (SIGNATURE_HEADER.to_string(), crate::webhook_keys::signature_header_for(endpoint, now, &body)),
        ];
        headers.extend(endpoint.headers.iter().cloned());

        self.transport.post(&endpoint.url, &headers, &body)
    }

    fn record(delivery: &mut Delivery, outcome: Result<u16, String>, now: u64) {
        let number = delivery.attempts.len() as u32 + 1;
        let (status_code, error) = match &outcome {
            Ok(code) => (Some(*code), None),
            Err(message) => (None, Some(message.clone())),
        };
        let success = matches!(outcome, Ok(code) if (200..300).contains(&code));
        let error = error.or_else(|| (!success).then(|| format!("unexpected status {}", status_code.unwrap_or(0))));
        delivery.attempts.push(Attempt { number, at: now, status_code, error });

        if success {
            delivery.status = DeliveryStatus::Delivered;
            delivery.next_attempt_at = None;
            return;
        }
        match RETRY_SCHEDULE_SECS.get(delivery.cycle_failures) {
            Some(wait) => {
                delivery.status = DeliveryStatus::Retrying;
                delivery.next_attempt_at = Some(now + wait);
            }
            None => {
                delivery.status = DeliveryStatus::DeadLettered;
                delivery.next_attempt_at = None;
            }
        }
        delivery.cycle_failures += 1;
    }

    // -- admin ------------------------------------------------------------

    /// Delivery log, optionally filtered by endpoint and/or status.
    pub fn deliveries(&self, endpoint_id: Option<&str>, status: Option<DeliveryStatus>) -> Result<Vec<Delivery>, WebhookError> {
        Ok(self
            .state()?
            .deliveries
            .iter()
            .filter(|d| endpoint_id.is_none_or(|e| d.endpoint_id == e))
            .filter(|d| status.is_none_or(|s| d.status == s))
            .cloned()
            .collect())
    }

    pub fn delivery(&self, id: &str) -> Result<Option<Delivery>, WebhookError> {
        Ok(self.state()?.deliveries.iter().find(|d| d.id == id).cloned())
    }

    /// Replay a delivery from scratch (new retry cycle); the attempt history is kept.
    pub fn replay(&self, delivery_id: &str, now: u64) -> Result<(), WebhookError> {
        let mut state = self.state()?;
        let delivery = state
            .deliveries
            .iter_mut()
            .find(|d| d.id == delivery_id)
            .ok_or_else(|| WebhookError::UnknownDelivery(delivery_id.to_string()))?;
        delivery.status = DeliveryStatus::Pending;
        delivery.next_attempt_at = Some(now);
        delivery.cycle_failures = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Clone, Default)]
    struct Recorded {
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    /// Replies are popped front to back; the default is `200`.
    #[derive(Clone, Default)]
    struct FakeTransport {
        replies: Arc<Mutex<VecDeque<Result<u16, String>>>>,
        calls: Arc<Mutex<Vec<Recorded>>>,
    }

    impl FakeTransport {
        fn reply(&self, replies: &[Result<u16, String>]) {
            self.replies.lock().unwrap().extend(replies.iter().cloned());
        }
        fn calls(&self) -> Vec<Recorded> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Transport for FakeTransport {
        fn post(&self, url: &str, headers: &[(String, String)], body: &[u8]) -> Result<u16, String> {
            self.calls.lock().unwrap().push(Recorded {
                url: url.to_string(),
                headers: headers.to_vec(),
                body: body.to_vec(),
            });
            self.replies.lock().unwrap().pop_front().unwrap_or(Ok(200))
        }
    }

    fn header<'a>(call: &'a Recorded, name: &str) -> Option<&'a str> {
        call.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn service() -> (WebhookService<FakeTransport>, FakeTransport) {
        let transport = FakeTransport::default();
        let svc = WebhookService::new(transport.clone());
        svc.register_endpoint(Endpoint::new("ep1", "https://hooks.example.com/in", "shh")).unwrap();
        (svc, transport)
    }

    #[test]
    fn signatures_verify_and_reject_tampering_staleness_and_wrong_secrets() {
        let body = br#"{"a":1}"#;
        let header = signature_header(b"secret", 1_000, body);

        assert!(verify_signature(b"secret", &header, body, 1_100, DEFAULT_TOLERANCE_SECS).is_ok());
        assert_eq!(verify_signature(b"secret", &header, br#"{"a":2}"#, 1_100, 300), Err(SignatureError::Mismatch));
        assert_eq!(verify_signature(b"other", &header, body, 1_100, 300), Err(SignatureError::Mismatch));
        assert_eq!(verify_signature(b"secret", &header, body, 1_000 + 301, 300), Err(SignatureError::Stale));
        assert_eq!(verify_signature(b"secret", &header, body, 1_000 - 301, 300), Err(SignatureError::Stale));
        for bad in ["", "garbage", "t=abc,v1=00", "v1=00", "t=1000"] {
            assert_eq!(verify_signature(b"secret", bad, body, 1_000, 300), Err(SignatureError::Malformed), "{bad}");
        }
        assert_eq!(verify_signature(b"secret", "t=1000,v1=zz", body, 1_000, 300), Err(SignatureError::Mismatch));
    }

    #[test]
    fn urls_must_be_public_https() {
        for ok in ["https://hooks.example.com/in", "https://example.com:8443/a?b=1", "https://8.8.8.8/x"] {
            assert!(validate_endpoint_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com",
            "ftp://example.com",
            "https://localhost/x",
            "https://127.0.0.1/x",
            "https://10.1.2.3/x",
            "https://192.168.0.5/x",
            "https://172.20.0.1/x",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/x",
            "https://user:pw@example.com/x",
            "https://service.internal/x",
            "https:///nohost",
        ] {
            assert!(validate_endpoint_url(bad).is_err(), "{bad}");
        }
        assert!(validate_endpoint_url("https://172.32.0.1/x").is_ok(), "172.32 is public");
    }

    #[test]
    fn a_published_event_is_delivered_with_signature_idempotency_and_custom_headers() {
        let transport = FakeTransport::default();
        let svc = WebhookService::new(transport.clone());
        svc.register_endpoint(Endpoint::new("ep1", "https://hooks.example.com/in", "shh").header("X-Tenant", "acme")).unwrap();

        let ids = svc.publish("vault.created", "evt_1", json!({"vault": "v1"}), 1_000).unwrap();
        assert_eq!(svc.run_due(1_000).unwrap(), 1);

        let call = &transport.calls()[0];
        assert_eq!(call.url, "https://hooks.example.com/in");
        assert_eq!(header(call, EVENT_ID_HEADER), Some("evt_1"));
        assert_eq!(header(call, DELIVERY_ID_HEADER), Some(ids[0].as_str()));
        assert_eq!(header(call, "X-Tenant"), Some("acme"));
        let sig = header(call, SIGNATURE_HEADER).unwrap();
        assert!(verify_signature(b"shh", sig, &call.body, 1_000, 300).is_ok());

        let body: Value = serde_json::from_slice(&call.body).unwrap();
        assert_eq!(body["id"], "evt_1");
        assert_eq!(body["type"], "vault.created");
        assert_eq!(body["data"]["vault"], "v1");

        let delivery = svc.delivery(&ids[0]).unwrap().unwrap();
        assert_eq!(delivery.status, DeliveryStatus::Delivered);
        assert_eq!(delivery.attempts[0].status_code, Some(200));
    }

    #[test]
    fn publishing_the_same_event_twice_does_not_duplicate_deliveries() {
        let (svc, transport) = service();
        let first = svc.publish("e", "evt_1", json!({}), 0).unwrap();
        let second = svc.publish("e", "evt_1", json!({}), 5).unwrap();
        assert_eq!(first, second);
        svc.run_due(10).unwrap();
        assert_eq!(transport.calls().len(), 1);
    }

    #[test]
    fn failures_retry_on_the_documented_schedule_then_dead_letter() {
        let (svc, transport) = service();
        transport.reply(&vec![Ok(500); 7]);
        let id = svc.publish("e", "evt_1", json!({}), 0).unwrap().remove(0);

        let mut now = 0;
        svc.run_due(now).unwrap();
        for wait in RETRY_SCHEDULE_SECS {
            let delivery = svc.delivery(&id).unwrap().unwrap();
            assert_eq!(delivery.status, DeliveryStatus::Retrying);
            assert_eq!(delivery.next_attempt_at, Some(now + wait));

            assert_eq!(svc.run_due(now + wait - 1).unwrap(), 0, "not due yet");
            now += wait;
            assert_eq!(svc.run_due(now).unwrap(), 1);
        }

        let delivery = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(delivery.status, DeliveryStatus::DeadLettered);
        assert_eq!(delivery.attempts.len(), 7);
        assert!(delivery.attempts.iter().all(|a| a.error.is_some()));
        assert_eq!(svc.run_due(now + 10_000_000).unwrap(), 0, "no more attempts once dead");
        assert_eq!(svc.deliveries(None, Some(DeliveryStatus::DeadLettered)).unwrap().len(), 1);
    }

    #[test]
    fn transport_errors_and_a_later_success() {
        let (svc, transport) = service();
        transport.reply(&[Err("connection refused".to_string()), Ok(204)]);
        let id = svc.publish("e", "evt_1", json!({}), 0).unwrap().remove(0);

        svc.run_due(0).unwrap();
        let delivery = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(delivery.attempts[0].error.as_deref(), Some("connection refused"));
        assert_eq!(delivery.attempts[0].status_code, None);

        svc.run_due(60).unwrap();
        let delivery = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(delivery.status, DeliveryStatus::Delivered);
        assert_eq!(delivery.attempts.len(), 2);
        assert_eq!(delivery.next_attempt_at, None);
    }

    #[test]
    fn the_event_id_is_stable_across_retries() {
        let (svc, transport) = service();
        transport.reply(&[Ok(503)]);
        svc.publish("e", "evt_9", json!({}), 0).unwrap();
        svc.run_due(0).unwrap();
        svc.run_due(60).unwrap();
        let calls = transport.calls();
        assert_eq!(header(&calls[0], EVENT_ID_HEADER), header(&calls[1], EVENT_ID_HEADER));
        assert_ne!(header(&calls[0], SIGNATURE_HEADER), header(&calls[1], SIGNATURE_HEADER), "re-signed with a fresh timestamp");
    }

    #[test]
    fn dead_letters_can_be_replayed_with_history_kept() {
        let (svc, transport) = service();
        transport.reply(&vec![Ok(500); 7]);
        let id = svc.publish("e", "evt_1", json!({}), 0).unwrap().remove(0);
        let mut now = 0;
        svc.run_due(now).unwrap();
        for wait in RETRY_SCHEDULE_SECS {
            now += wait;
            svc.run_due(now).unwrap();
        }
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::DeadLettered);

        svc.replay(&id, now + 1).unwrap();
        assert_eq!(svc.run_due(now + 1).unwrap(), 1);

        let delivery = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(delivery.status, DeliveryStatus::Delivered);
        assert_eq!(delivery.attempts.len(), 8);
        assert!(svc.replay("nope", 0).is_err());
    }

    #[test]
    fn a_replay_starts_a_fresh_retry_cycle() {
        let (svc, transport) = service();
        transport.reply(&[Ok(500), Ok(500)]);
        let id = svc.publish("e", "evt_1", json!({}), 0).unwrap().remove(0);
        svc.run_due(0).unwrap();
        svc.run_due(60).unwrap(); // second failure -> next wait is 15 min
        svc.replay(&id, 61).unwrap();
        transport.reply(&[Ok(500)]);
        svc.run_due(61).unwrap();

        // Back to the first step of the schedule, not the third.
        assert_eq!(svc.delivery(&id).unwrap().unwrap().next_attempt_at, Some(61 + RETRY_SCHEDULE_SECS[0]));
    }

    #[test]
    fn endpoints_filter_events_and_can_be_disabled() {
        let transport = FakeTransport::default();
        let svc = WebhookService::new(transport.clone());
        svc.register_endpoint(Endpoint::new("only-fees", "https://a.example.com", "s").events(&["fee.updated"])).unwrap();
        svc.register_endpoint(Endpoint::new("all", "https://b.example.com", "s")).unwrap();

        assert_eq!(svc.publish("vault.created", "e1", json!({}), 0).unwrap().len(), 1);
        assert_eq!(svc.publish("fee.updated", "e2", json!({}), 0).unwrap().len(), 2);

        svc.set_enabled("all", false).unwrap();
        assert_eq!(svc.publish("fee.updated", "e3", json!({}), 0).unwrap().len(), 1);
        assert!(svc.set_enabled("missing", true).is_err());

        // A delivery queued before the endpoint was disabled waits instead of sending.
        assert_eq!(svc.run_due(0).unwrap(), 2);
        assert!(transport.calls().iter().all(|c| c.url == "https://a.example.com"));
        assert_eq!(svc.deliveries(Some("only-fees"), None).unwrap().len(), 2);

        // Re-enabling releases what was held back.
        svc.set_enabled("all", true).unwrap();
        assert_eq!(svc.run_due(0).unwrap(), 2);
        assert!(transport.calls().iter().any(|c| c.url == "https://b.example.com"));
    }

    #[test]
    fn payloads_can_be_transformed_per_endpoint() {
        let transport = FakeTransport::default();
        let svc = WebhookService::new(transport.clone());
        svc.register_endpoint(
            Endpoint::new("ep", "https://hooks.example.com", "s")
                .transform(|payload| json!({"vault_id": payload["id"], "redacted": true})),
        )
        .unwrap();

        svc.publish("vault.created", "evt", json!({"id": "v1", "secret": "x"}), 0).unwrap();
        svc.run_due(0).unwrap();

        let body: Value = serde_json::from_slice(&transport.calls()[0].body).unwrap();
        assert_eq!(body["data"], json!({"vault_id": "v1", "redacted": true}));
    }

    #[test]
    fn registering_a_private_endpoint_is_refused_and_secrets_are_random() {
        let svc = WebhookService::new(FakeTransport::default());
        assert!(svc.register_endpoint(Endpoint::new("x", "https://127.0.0.1/hook", "s")).is_err());

        let (a, b) = (generate_secret(), generate_secret());
        assert!(a.starts_with("whsec_") && a.len() == 6 + 64);
        assert_ne!(a, b);
    }
}
