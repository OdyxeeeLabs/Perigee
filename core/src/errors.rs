use axum::{
    async_trait,
    extract::{FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::de::DeserializeOwned;
use std::env;
use thiserror::Error;

use crate::input_sanitization::{deserialize_sanitized, SanitizedJson};
use crate::simulation::SimulationError;

/// Returns `true` when `APP_ENV=production`.
///
/// The check is intentionally strict: every value other than the literal
/// string `"production"` is treated as non-production so that missing or
/// misspelled values are safe by default in test/staging environments.
///
/// The value is read fresh on every call so that tests can override it with
/// `std::env::set_var` without needing a process restart.
pub fn is_production() -> bool {
    env::var("APP_ENV")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("production"))
        .unwrap_or(false)
}

/// Serialises every test that mutates process-global environment variables.
///
/// `config.rs` and `errors.rs` both toggle `APP_ENV`, so a per-module lock
/// would still let the two modules race. Tests passed under
/// `--test-threads=1` and failed intermittently otherwise.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire [`ENV_LOCK`], ignoring poisoning — a poisoned lock only means some
/// other env test panicked, and the rest still need to run serially.
#[cfg(test)]
pub(crate) fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum AppError {
    /// Internal server errors.
    ///
    /// The inner string contains full diagnostic detail suitable for logging
    /// but **must not** be forwarded to HTTP clients in production.  The
    /// [`IntoResponse`] implementation redacts it when [`is_production`]
    /// returns `true`.
    #[error("Internal server error")]
    Internal(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Unauthorized errors.
    ///
    /// The inner string may reveal internal auth logic (e.g. JWT parsing
    /// details) so it is also redacted in production.
    #[error("Unauthorized")]
    Unauthorized(String),

    #[error("Too many requests: {0}")]
    TooManyRequests(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    /// Forbidden errors (e.g. role or vault-scoped authorization failure).
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// The vault's policy has expired, so it no longer authorises the
    /// operation (BE-023). 403 rather than 400: the request is well formed
    /// and the caller is authenticated — the authority behind it has lapsed.
    #[error("Policy expired: {0}")]
    PolicyExpired(String),
}

impl AppError {
    /// Return the inner diagnostic string for use in **server-side logs only**.
    ///
    /// Callers should never forward this value to HTTP response bodies; use
    /// [`IntoResponse`] which applies the production-redaction policy.
    pub fn diagnostic(&self) -> &str {
        match self {
            Self::Internal(msg)
            | Self::NotFound(msg)
            | Self::BadRequest(msg)
            | Self::Unauthorized(msg)
            | Self::Forbidden(msg)
            | Self::TooManyRequests(msg)
            | Self::Conflict(msg)
            | Self::PolicyExpired(msg) => msg.as_str(),
        }
    }

    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Internal(_) => ErrorCode::InternalServerError,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::BadRequest(_) => ErrorCode::BadRequest,
            Self::Unauthorized(_) => ErrorCode::Unauthorized,
            Self::Forbidden(_) => ErrorCode::Forbidden,
            Self::TooManyRequests(_) => ErrorCode::TooManyRequests,
            Self::Conflict(_) => ErrorCode::Conflict,
            Self::PolicyExpired(_) => ErrorCode::PolicyExpired,
        }
    }

    fn status_code(&self) -> StatusCode {
        self.error_code().status_code()
    }

    fn error_type(&self) -> &'static str {
        self.error_code().as_str()
    }

    /// The client-visible message for this error.
    ///
    /// In production, `Internal` and `Unauthorized` variants return a static
    /// opaque string so that stack traces, DB errors, and auth internals are
    /// never forwarded to HTTP clients.  In non-production the full diagnostic
    /// string is returned to aid debugging.
    fn client_message(&self) -> String {
        match self {
            // Safe variants — their detail is always client-appropriate.
            Self::NotFound(msg) => format!("Not found: {}", msg),
            Self::BadRequest(msg) => format!("Bad request: {}", msg),
            Self::Forbidden(msg) => format!("Forbidden: {}", msg),

            // Sensitive variants — redact in production.
            Self::Internal(msg) => {
                if is_production() {
                    "An internal server error occurred. Please try again later.".to_string()
                } else {
                    format!("Internal server error: {}", msg)
                }
            }
            Self::Unauthorized(msg) => {
                if is_production() {
                    "Unauthorized.".to_string()
                } else {
                    format!("Unauthorized: {}", msg)
                }
            }

            // Safe variants — the caller needs the detail to act on them.
            // A rate-limited client needs to know it was rate limited, and a
            // conflicted write needs to know which version it lost to.
            Self::TooManyRequests(msg) => format!("Too many requests: {}", msg),
            Self::Conflict(msg) => format!("Conflict: {}", msg),

            // The caller needs the expiry timestamp to understand why, and it
            // is not sensitive — it is their own policy.
            Self::PolicyExpired(msg) => format!("Policy expired: {}", msg),
        }
    }
}

pub use crate::error_codes::{ErrorCode, ErrorResponse};

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let code = self.error_code();

        // Log the full diagnostic detail server-side regardless of environment.
        // Sensitive details never reach the HTTP response body in production.
        tracing::error!(
            error_type = code.as_str(),
            status = status.as_u16(),
            detail = self.diagnostic(),
            "Request failed"
        );

        let body = Json(ErrorResponse {
            code: code.as_str().to_string(),
            error: code.as_str().to_string(),
            message: self.client_message(),
            details: None,
        });

        (status, body).into_response()
    }
}

/// Convert SimulationError to AppError with appropriate HTTP status codes.
///
/// Maps client errors (4xx) to BadRequest and server errors (5xx) to Internal.
impl From<SimulationError> for AppError {
    fn from(err: SimulationError) -> Self {
        match err {
            // Client errors (HTTP 400)
            SimulationError::NodeError(msg) => {
                // NodeError covers invalid contract IDs, bad parameters
                AppError::BadRequest(format!("RPC node error: {}", msg))
            }
            SimulationError::InvalidContract(msg) => {
                AppError::BadRequest(format!("Invalid contract: {}", msg))
            }
            // BE-020: malformed WASM is the caller's input, so 400 rather than
            // 500, and the message says what was wrong with it.
            SimulationError::InvalidWasm(msg) => {
                AppError::BadRequest(format!("Invalid WASM: {}", msg))
            }
            SimulationError::ParseError(e) => {
                AppError::BadRequest(format!("Argument parse error: {}", e))
            }
            SimulationError::XdrError(msg) => {
                AppError::BadRequest(format!("XDR encoding error: {}", msg))
            }
            SimulationError::Base64Error(e) => {
                AppError::BadRequest(format!("Base64 decode error: {}", e))
            }

            // Server errors (HTTP 500)
            SimulationError::NodeTimeout => AppError::Internal("RPC request timed out".to_string()),
            SimulationError::Cancelled => AppError::Internal("Request cancelled".to_string()),
            SimulationError::RpcRequestFailed(msg) => {
                AppError::Internal(format!("RPC request failed: {}", msg))
            }
            SimulationError::NetworkError(e) => AppError::Internal(format!("Network error: {}", e)),
            SimulationError::Io(e) => AppError::Internal(format!("IO error: {}", e)),
            SimulationError::SerializationError(e) => {
                AppError::Internal(format!("Serialization error: {}", e))
            }

            // Local-runner errors. `LocalUnavailable` should normally be
            // handled upstream by falling back to RPC, so if it reaches the
            // HTTP boundary treat it as an internal misconfiguration.
            SimulationError::LocalUnavailable => AppError::Internal(
                "Local WASM execution unavailable and no RPC fallback succeeded".to_string(),
            ),
            SimulationError::ExecutionFailed(msg) => {
                AppError::BadRequest(format!("Contract execution failed: {}", msg))
            }
            SimulationError::InsufficientConsensusProviders(msg) => {
                AppError::Internal(format!("Insufficient consensus providers: {}", msg))
            }
            SimulationError::ConsensusMismatch(msg) => {
                AppError::Internal(format!("Consensus mismatch: {}", msg))
            }
        }
    }
}

impl From<crate::parser::ParserError> for AppError {
    fn from(err: crate::parser::ParserError) -> Self {
        AppError::BadRequest(err.to_string())
    }
}

// ── Validate trait ────────────────────────────────────────────────────────────

/// Field-level validation for request structs.
///
/// Implement this on every request DTO that needs field presence / format
/// checks beyond what `serde` provides.  The [`ValidatedJson`] extractor
/// calls this after successful JSON deserialisation and converts any error
/// into a `400 Bad Request` response using the standard [`ErrorResponse`]
/// envelope.
pub trait Validate {
    /// Return an error message describing the first validation failure, or
    /// `Ok(())` when all fields are valid.
    fn validate(&self) -> Result<(), String>;
}

// ── ValidatedJson extractor ───────────────────────────────────────────────────

/// Drop-in replacement for `axum::extract::Json` that normalises **all**
/// error cases — bad JSON syntax, wrong types, and field-level validation
/// failures — into the same `{ error, message }` response envelope used by
/// [`AppError`].
///
/// # Usage
/// ```ignore
/// async fn my_handler(
///     State(state): State<Arc<AppState>>,
///     ValidatedJson(payload): ValidatedJson<MyRequest>,
/// ) -> Result<Json<MyResponse>, AppError> { … }
/// ```
///
/// where `MyRequest: serde::de::DeserializeOwned + Validate + Send + 'static`.
pub struct ValidatedJson<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let SanitizedJson(value) = SanitizedJson::<serde_json::Value>::from_request(req, state).await?;
        let value: T = deserialize_sanitized(value)?;
        value.validate().map_err(AppError::BadRequest)?;
        Ok(ValidatedJson(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env(value: &str, f: impl FnOnce()) {
        // Rust runs tests in parallel by default, so this needs the shared
        // lock — the previous comment claiming otherwise was the reason these
        // tests failed intermittently.
        let _env = crate::errors::env_guard();

        unsafe { env::set_var("APP_ENV", value) };
        f();
        unsafe { env::remove_var("APP_ENV") };
    }

    #[test]
    fn is_production_true_for_production() {
        with_env("production", || assert!(is_production()));
    }

    #[test]
    fn is_production_case_insensitive() {
        with_env("Production", || assert!(is_production()));
        with_env("PRODUCTION", || assert!(is_production()));
    }

    #[test]
    fn is_production_false_for_staging() {
        with_env("staging", || assert!(!is_production()));
    }

    #[test]
    fn is_production_false_when_unset() {
        unsafe { env::remove_var("APP_ENV") };
        assert!(!is_production());
    }

    #[test]
    fn internal_error_redacted_in_production() {
        with_env("production", || {
            let err = AppError::Internal("DB error: password=hunter2".to_string());
            let msg = err.client_message();
            assert!(!msg.contains("hunter2"), "DB detail leaked: {}", msg);
            assert!(!msg.contains("DB error"), "Internal detail leaked: {}", msg);
        });
    }

    #[test]
    fn internal_error_exposed_in_dev() {
        unsafe { env::remove_var("APP_ENV") };
        let err = AppError::Internal("debug info".to_string());
        let msg = err.client_message();
        assert!(msg.contains("debug info"));
    }

    #[test]
    fn unauthorized_redacted_in_production() {
        with_env("production", || {
            let err = AppError::Unauthorized("JWT parse failed at byte 42".to_string());
            let msg = err.client_message();
            assert!(!msg.contains("JWT"), "Auth detail leaked: {}", msg);
            assert!(!msg.contains("byte 42"), "Auth detail leaked: {}", msg);
        });
    }

    #[test]
    fn bad_request_always_exposes_detail() {
        with_env("production", || {
            let err = AppError::BadRequest("invalid contract ID format".to_string());
            let msg = err.client_message();
            assert!(msg.contains("invalid contract ID format"));
        });
    }

    #[test]
    fn not_found_always_exposes_detail() {
        with_env("production", || {
            let err = AppError::NotFound("contract ABC not deployed".to_string());
            let msg = err.client_message();
            assert!(msg.contains("contract ABC not deployed"));
        });
    }
}

#[cfg(test)]
mod validated_json_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{header, Method, Request, StatusCode},
        routing::post,
        Router,
    };
    use tower::ServiceExt;

    // ── Minimal request structs for testing ──────────────────────────────────

    #[derive(serde::Deserialize)]
    struct SimpleRequest {
        name: String,
    }

    impl Validate for SimpleRequest {
        fn validate(&self) -> Result<(), String> {
            if self.name.trim().is_empty() {
                return Err("name must be a non-empty string".to_string());
            }
            Ok(())
        }
    }

    async fn test_handler(
        ValidatedJson(payload): ValidatedJson<SimpleRequest>,
    ) -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({ "name": payload.name }))
    }

    fn test_app() -> Router {
        Router::new().route("/test", post(test_handler))
    }

    /// Sends a POST to /test, returns (status, body_text).
    async fn send(body: &str, content_type: &str) -> (StatusCode, String) {
        let app = test_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/test")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body.to_owned()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn valid_request_passes_through() {
        let (status, body) = send(r#"{"name":"alice"}"#, "application/json").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("alice"));
    }

    #[tokio::test]
    async fn malformed_json_returns_400_envelope() {
        let (status, body) = send("{not valid json}", "application/json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {}", body);
        let v: serde_json::Value = serde_json::from_str(&body).expect("response must be JSON");
        assert_eq!(v["error"], "BAD_REQUEST");
        assert!(
            v["message"].as_str().unwrap_or("").len() > 0,
            "message must not be empty"
        );
    }

    #[tokio::test]
    async fn wrong_content_type_returns_400_envelope() {
        let (status, body) = send(r#"{"name":"alice"}"#, "text/plain").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {}", body);
        let v: serde_json::Value = serde_json::from_str(&body).expect("response must be JSON");
        assert_eq!(v["error"], "BAD_REQUEST");
        assert!(
            v["message"]
                .as_str()
                .unwrap_or("")
                .contains("application/json"),
            "message should mention content-type requirement: {}",
            v["message"]
        );
    }

    #[tokio::test]
    async fn wrong_field_type_returns_400_envelope() {
        // `name` must be a string, sending a number should fail deserialization.
        let (status, body) = send(r#"{"name":42}"#, "application/json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {}", body);
        let v: serde_json::Value = serde_json::from_str(&body).expect("response must be JSON");
        assert_eq!(v["error"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn field_validation_empty_string_returns_400() {
        let (status, body) = send(r#"{"name":"   "}"#, "application/json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {}", body);
        let v: serde_json::Value = serde_json::from_str(&body).expect("response must be JSON");
        assert_eq!(v["error"], "BAD_REQUEST");
        assert!(
            v["message"].as_str().unwrap_or("").contains("name"),
            "message should name the failing field: {}",
            v["message"]
        );
    }

    // ── Validate trait unit tests (no HTTP layer needed) ──────────────────────

    #[test]
    fn validate_rejects_empty_name() {
        let req = SimpleRequest {
            name: "".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_whitespace_only_name() {
        let req = SimpleRequest {
            name: "   ".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_non_empty_name() {
        let req = SimpleRequest {
            name: "alice".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn parser_error_converts_to_bad_request() {
        use crate::parser::ParserError;
        let err = ParserError::InvalidXdr {
            location: "$".to_string(),
            details: "Invalid base64 encoding: invalid symbol".to_string(),
        };
        let app_err: AppError = err.into();
        assert_eq!(app_err.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(app_err.error_type(), "BAD_REQUEST");
        assert!(app_err.client_message().contains("Invalid XDR at $"));
    }
}
