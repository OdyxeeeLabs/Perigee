use crate::error_codes::ErrorCode;

pub fn stable_code(value: &str) -> Option<ErrorCode> {
    value.parse().ok()
}

pub fn legacy_code(code: ErrorCode) -> ErrorCode {
    match code {
        ErrorCode::Unauthorized
        | ErrorCode::InvalidApiKey
        | ErrorCode::TokenExpired
        | ErrorCode::InvalidSignature => ErrorCode::Unauthorized,
        ErrorCode::ManagerNotFound
        | ErrorCode::VaultNotFound
        | ErrorCode::JobNotFound
        | ErrorCode::ReconciliationNotFound => ErrorCode::NotFound,
        ErrorCode::InvalidInput
        | ErrorCode::InvalidJson
        | ErrorCode::InvalidContractId
        | ErrorCode::InvalidWasm
        | ErrorCode::InvalidParameters
        | ErrorCode::InvalidBase64
        | ErrorCode::InvalidXdr
        | ErrorCode::ParseError
        | ErrorCode::ValidationFailed
        | ErrorCode::SimulationFailed
        | ErrorCode::ContractExecutionFailed
        | ErrorCode::NodeError
        | ErrorCode::RpcNodeError
        | ErrorCode::InsufficientBalance
        | ErrorCode::InsufficientLiquidity
        | ErrorCode::InsufficientShares
        | ErrorCode::InsufficientAllowance
        | ErrorCode::SlippageExceeded
        | ErrorCode::InvalidFee
        | ErrorCode::ContractPaused
        | ErrorCode::RequestBodyReadFailed => ErrorCode::BadRequest,
        ErrorCode::AlreadyExists
        | ErrorCode::ManagerAlreadyExists
        | ErrorCode::StateMismatch => ErrorCode::Conflict,
        ErrorCode::JobCannotBeCancelled => ErrorCode::BadRequest,
        ErrorCode::RateLimitExceeded => ErrorCode::TooManyRequests,
        ErrorCode::CircuitBreakerOpen | ErrorCode::NoHealthyRpcProviders => {
            ErrorCode::ServiceUnavailable
        }
        ErrorCode::RpcTimeout | ErrorCode::NodeTimeout => ErrorCode::InternalServerError,
        ErrorCode::RpcRequestFailed
        | ErrorCode::LocalUnavailable
        | ErrorCode::ConsensusMismatch
        | ErrorCode::InsufficientConsensus
        | ErrorCode::OracleNotConfigured
        | ErrorCode::InvalidOraclePrice
        | ErrorCode::DatabaseError
        | ErrorCode::NetworkError
        | ErrorCode::IoError
        | ErrorCode::SerializationError
        | ErrorCode::ConfigurationError
        | ErrorCode::ReconciliationFailed => ErrorCode::InternalServerError,
        ErrorCode::BadRequest
        | ErrorCode::PayloadTooLarge
        | ErrorCode::MethodNotAllowed
        | ErrorCode::UnsupportedApiVersion
        | ErrorCode::UnsupportedMediaType
        | ErrorCode::NotFound
        | ErrorCode::Conflict
        | ErrorCode::TooManyRequests
        | ErrorCode::ServiceUnavailable
        | ErrorCode::Forbidden
        | ErrorCode::PolicyExpired
        | ErrorCode::InternalServerError => code,
    }
}
