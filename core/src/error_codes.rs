use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::fmt;
use utoipa::ToSchema;

/// Structured, machine-readable error codes for the Perigee platform.
///
/// Enables frontend clients, SDKs, and API consumers to programmatically map
/// error conditions to user-friendly messages, localized alerts, and specific
/// recovery workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // ── Authentication & Authorization (401 / 403) ───────────────────────────
    Unauthorized,
    Forbidden,
    InvalidApiKey,
    TokenExpired,
    InvalidSignature,
    ManagerNotFound,
    PolicyExpired,

    // ── Client Input & Request Validation (400 / 422) ────────────────────────
    BadRequest,
    InvalidInput,
    InvalidJson,
    InvalidContractId,
    InvalidWasm,
    InvalidParameters,
    InvalidBase64,
    InvalidXdr,
    PayloadTooLarge,
    UnsupportedMediaType,
    NotAcceptable,
    ParseError,
    ValidationFailed,

    // ── Resource Lifecycle & Conflict (404 / 409 / 429) ──────────────────────
    NotFound,
    AlreadyExists,
    Conflict,
    StateMismatch,
    RateLimitExceeded,
    TooManyRequests,
    CircuitBreakerOpen,

    // ── Blockchain / Soroban / Simulation (400 / 500 / 502 / 504) ────────────
    SimulationFailed,
    ContractExecutionFailed,
    NodeError,
    RpcNodeError,
    RpcRequestFailed,
    RpcTimeout,
    NodeTimeout,
    LocalUnavailable,
    ConsensusMismatch,
    InsufficientConsensus,
    InsufficientBalance,
    InsufficientLiquidity,
    InsufficientShares,
    InsufficientAllowance,
    SlippageExceeded,
    InvalidFee,
    OracleNotConfigured,
    InvalidOraclePrice,
    ContractPaused,

    // ── Server & Infrastructure (500 / 503) ──────────────────────────────────
    InternalServerError,
    DatabaseError,
    NetworkError,
    IoError,
    SerializationError,
    ServiceUnavailable,
    ConfigurationError,
}

impl ErrorCode {
    /// Return the canonical UPPER_SNAKE_CASE string identifier for this error code.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::InvalidApiKey => "INVALID_API_KEY",
            Self::TokenExpired => "TOKEN_EXPIRED",
            Self::InvalidSignature => "INVALID_SIGNATURE",
            Self::ManagerNotFound => "MANAGER_NOT_FOUND",
            Self::PolicyExpired => "POLICY_EXPIRED",

            Self::BadRequest => "BAD_REQUEST",
            Self::InvalidInput => "INVALID_INPUT",
            Self::InvalidJson => "INVALID_JSON",
            Self::InvalidContractId => "INVALID_CONTRACT_ID",
            Self::InvalidWasm => "INVALID_WASM",
            Self::InvalidParameters => "INVALID_PARAMETERS",
            Self::InvalidBase64 => "INVALID_BASE64",
            Self::InvalidXdr => "INVALID_XDR",
            Self::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            Self::UnsupportedMediaType => "UNSUPPORTED_MEDIA_TYPE",
            Self::NotAcceptable => "NOT_ACCEPTABLE",
            Self::ParseError => "PARSE_ERROR",
            Self::ValidationFailed => "VALIDATION_FAILED",

            Self::NotFound => "NOT_FOUND",
            Self::AlreadyExists => "ALREADY_EXISTS",
            Self::Conflict => "CONFLICT",
            Self::StateMismatch => "STATE_MISMATCH",
            Self::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
            Self::TooManyRequests => "TOO_MANY_REQUESTS",
            Self::CircuitBreakerOpen => "CIRCUIT_BREAKER_OPEN",

            Self::SimulationFailed => "SIMULATION_FAILED",
            Self::ContractExecutionFailed => "CONTRACT_EXECUTION_FAILED",
            Self::NodeError => "NODE_ERROR",
            Self::RpcNodeError => "RPC_NODE_ERROR",
            Self::RpcRequestFailed => "RPC_REQUEST_FAILED",
            Self::RpcTimeout => "RPC_TIMEOUT",
            Self::NodeTimeout => "NODE_TIMEOUT",
            Self::LocalUnavailable => "LOCAL_UNAVAILABLE",
            Self::ConsensusMismatch => "CONSENSUS_MISMATCH",
            Self::InsufficientConsensus => "INSUFFICIENT_CONSENSUS",
            Self::InsufficientBalance => "INSUFFICIENT_BALANCE",
            Self::InsufficientLiquidity => "INSUFFICIENT_LIQUIDITY",
            Self::InsufficientShares => "INSUFFICIENT_SHARES",
            Self::InsufficientAllowance => "INSUFFICIENT_ALLOWANCE",
            Self::SlippageExceeded => "SLIPPAGE_EXCEEDED",
            Self::InvalidFee => "INVALID_FEE",
            Self::OracleNotConfigured => "ORACLE_NOT_CONFIGURED",
            Self::InvalidOraclePrice => "INVALID_ORACLE_PRICE",
            Self::ContractPaused => "CONTRACT_PAUSED",

            Self::InternalServerError => "INTERNAL_SERVER_ERROR",
            Self::DatabaseError => "DATABASE_ERROR",
            Self::NetworkError => "NETWORK_ERROR",
            Self::IoError => "IO_ERROR",
            Self::SerializationError => "SERIALIZATION_ERROR",
            Self::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            Self::ConfigurationError => "CONFIGURATION_ERROR",
        }
    }

    /// Map the error code to its standard HTTP [`StatusCode`].
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized
            | Self::InvalidApiKey
            | Self::TokenExpired
            | Self::InvalidSignature => StatusCode::UNAUTHORIZED,

            Self::Forbidden | Self::PolicyExpired | Self::ManagerNotFound => StatusCode::FORBIDDEN,

            Self::BadRequest
            | Self::InvalidInput
            | Self::InvalidJson
            | Self::InvalidContractId
            | Self::InvalidWasm
            | Self::InvalidParameters
            | Self::InvalidBase64
            | Self::InvalidXdr
            | Self::ParseError
            | Self::ValidationFailed
            | Self::SimulationFailed
            | Self::ContractExecutionFailed
            | Self::NodeError
            | Self::RpcNodeError
            | Self::InsufficientBalance
            | Self::InsufficientLiquidity
            | Self::InsufficientShares
            | Self::InsufficientAllowance
            | Self::SlippageExceeded
            | Self::InvalidFee
            | Self::ContractPaused => StatusCode::BAD_REQUEST,

            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::NotAcceptable => StatusCode::NOT_ACCEPTABLE,

            Self::NotFound => StatusCode::NOT_FOUND,
            Self::AlreadyExists | Self::Conflict | Self::StateMismatch => StatusCode::CONFLICT,
            Self::RateLimitExceeded | Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,

            Self::RpcTimeout | Self::NodeTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::ServiceUnavailable | Self::CircuitBreakerOpen => StatusCode::SERVICE_UNAVAILABLE,

            Self::InternalServerError
            | Self::DatabaseError
            | Self::NetworkError
            | Self::IoError
            | Self::SerializationError
            | Self::ConfigurationError
            | Self::LocalUnavailable
            | Self::ConsensusMismatch
            | Self::InsufficientConsensus
            | Self::RpcRequestFailed
            | Self::OracleNotConfigured
            | Self::InvalidOraclePrice => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<ErrorCode> for StatusCode {
    fn from(code: ErrorCode) -> Self {
        code.status_code()
    }
}

impl From<&ErrorCode> for StatusCode {
    fn from(code: &ErrorCode) -> Self {
        code.status_code()
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Standard structured JSON error response envelope.
///
/// Contains machine-readable `code`, human-readable `message`, and optional
/// `details` payload, as well as an `error` alias for backwards compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ErrorResponse {
    /// Machine-readable error code string (e.g. "BAD_REQUEST", "INVALID_WASM")
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Structured context details or field errors (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// Error code alias for backwards compatibility with legacy clients
    pub error: String,
}

impl ErrorResponse {
    /// Create a new error response with no extra details.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let code_str = code.into();
        Self {
            error: code_str.clone(),
            code: code_str,
            message: message.into(),
            details: None,
        }
    }

    /// Create an error response with structured context details.
    pub fn with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: impl Into<serde_json::Value>,
    ) -> Self {
        let code_str = code.into();
        Self {
            error: code_str.clone(),
            code: code_str,
            message: message.into(),
            details: Some(details.into()),
        }
    }

    /// Create an error response from an [`ErrorCode`].
    pub fn from_error_code(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(code.as_str(), message)
    }

    /// Create an error response from an [`ErrorCode`] with structured details.
    pub fn from_error_code_with_details(
        code: ErrorCode,
        message: impl Into<String>,
        details: impl Into<serde_json::Value>,
    ) -> Self {
        Self::with_details(code.as_str(), message, details)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_status_mapping() {
        assert_eq!(StatusCode::from(ErrorCode::Unauthorized), StatusCode::UNAUTHORIZED);
        assert_eq!(StatusCode::from(ErrorCode::Forbidden), StatusCode::FORBIDDEN);
        assert_eq!(StatusCode::from(ErrorCode::PolicyExpired), StatusCode::FORBIDDEN);
        assert_eq!(StatusCode::from(ErrorCode::BadRequest), StatusCode::BAD_REQUEST);
        assert_eq!(StatusCode::from(ErrorCode::InvalidWasm), StatusCode::BAD_REQUEST);
        assert_eq!(StatusCode::from(ErrorCode::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(StatusCode::from(ErrorCode::Conflict), StatusCode::CONFLICT);
        assert_eq!(StatusCode::from(ErrorCode::TooManyRequests), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(StatusCode::from(ErrorCode::InternalServerError), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(StatusCode::from(ErrorCode::ServiceUnavailable), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(StatusCode::from(ErrorCode::RpcTimeout), StatusCode::GATEWAY_TIMEOUT);
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse::from_error_code_with_details(
            ErrorCode::InvalidWasm,
            "Invalid WASM format",
            serde_json::json!({ "field": "bytecode", "reason": "corrupted header" }),
        );

        let json_str = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(value["code"], "INVALID_WASM");
        assert_eq!(value["error"], "INVALID_WASM");
        assert_eq!(value["message"], "Invalid WASM format");
        assert_eq!(value["details"]["field"], "bytecode");
        assert_eq!(value["details"]["reason"], "corrupted header");
    }

    #[test]
    fn test_error_response_without_details() {
        let resp = ErrorResponse::from_error_code(ErrorCode::NotFound, "Vault not found");
        let json_str = serde_json::to_string(&resp).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(value["code"], "NOT_FOUND");
        assert_eq!(value["message"], "Vault not found");
        assert!(value.get("details").is_none() || value["details"].is_null());
    }
}
