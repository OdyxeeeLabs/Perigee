//! Retry classification for webhook deliveries (API-07).
//!
//! Transport errors, `408`, `425`, `429` and `5xx` are transient and follow
//! [`crate::webhooks::RETRY_SCHEDULE_SECS`] (exponential backoff, capped at
//! that many retries). Any other non-2xx answer (e.g. `400`, `404`, `410`) is
//! terminal: the delivery is dead-lettered immediately and can be replayed.

/// Whether a failed attempt is worth retrying.
pub fn is_retryable(outcome: &Result<u16, String>) -> bool {
    match outcome {
        Err(_) => true,
        Ok(code) => matches!(code, 408 | 425 | 429) || (500..=599).contains(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhooks::{DeliveryStatus, Endpoint, Transport, WebhookService, RETRY_SCHEDULE_SECS};
    use serde_json::json;

    struct Fixed(Result<u16, String>);
    impl Transport for Fixed {
        fn post(&self, _: &str, _: &[(String, String)], _: &[u8]) -> Result<u16, String> {
            self.0.clone()
        }
    }

    fn run(reply: Result<u16, String>) -> (DeliveryStatus, usize, Option<u64>) {
        let svc = WebhookService::new(Fixed(reply));
        svc.register_endpoint(Endpoint::new("ep", "https://hooks.example.com/in", "s")).unwrap();
        svc.publish("t", "evt1", json!({}), 100).unwrap();
        svc.run_due(100).unwrap();
        let d = svc.deliveries(None, None).unwrap().remove(0);
        (d.status, d.attempts.len(), d.next_attempt_at)
    }

    #[test]
    fn classifies_response_classes() {
        for retry in [Err("timeout".to_string()), Ok(408), Ok(429), Ok(500), Ok(503)] {
            assert!(is_retryable(&retry), "{retry:?}");
        }
        for terminal in [Ok(301), Ok(400), Ok(401), Ok(404), Ok(410)] {
            assert!(!is_retryable(&terminal), "{terminal:?}");
        }
    }

    #[test]
    fn transient_failure_schedules_backoff_and_terminal_failure_dead_letters() {
        let (status, attempts, next) = run(Ok(503));
        assert_eq!((status, attempts, next), (DeliveryStatus::Retrying, 1, Some(100 + RETRY_SCHEDULE_SECS[0])));

        let (status, attempts, next) = run(Ok(410));
        assert_eq!((status, attempts, next), (DeliveryStatus::DeadLettered, 1, None));
    }
}
