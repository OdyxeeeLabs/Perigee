# Error Variants and HTTP Conversion Guide

This guide is the contract for error handling in the Perigee HTTP API. Error responses use stable, human-readable identifiers so clients can make retry, authentication, validation, and support decisions without parsing English messages.

## Response envelope

Structured application errors use this shape:

```json
{
  "code": "INVALID_BASE64",
  "error": "BAD_REQUEST",
  "message": "Invalid base64 WASM data",
  "details": {
    "field": "wasm_bytes"
  }
}
```

`code` is the stable named code. `error` is the legacy broad category retained for existing clients; it is not removed when a more specific `code` is available. `message` is safe for the caller to display. `details` is optional and is omitted when it is not present.

A response that has no more specific classification can use the same value in both fields:

```json
{
  "code": "NOT_FOUND",
  "error": "NOT_FOUND",
  "message": "Vault not found"
}
```

Production responses redact internal, database, network, serialization, and configuration diagnostics. The original detail remains available to server-side logging.

The shared `ApiJson` and `ValidatedJson` extractors map malformed JSON to `INVALID_JSON` with legacy `BAD_REQUEST`; the legacy content-type rejection remains HTTP 400 for compatibility.

## AppError variants

`AppError` is the application boundary type. New handlers should prefer `AppError::with_code(ErrorCode, message)` when a condition has a specific code.

| Rust variant | Typical condition | HTTP status | Legacy `error` value |
| --- | --- | ---: | --- |
| `Internal(String)` | Unexpected server or infrastructure failure | 500 | `INTERNAL_SERVER_ERROR` |
| `NotFound(String)` | Requested resource does not exist | 404 | `NOT_FOUND` |
| `BadRequest(String)` | General malformed or semantically invalid request | 400 | `BAD_REQUEST` |
| `Unauthorized(String)` | Authentication is missing or invalid | 401 | `UNAUTHORIZED` |
| `TooManyRequests(String)` | Existing tenant limiter rejected a request | 429 | `TOO_MANY_REQUESTS` |
| `Conflict(String)` | Optimistic-lock or state conflict | 409 | `CONFLICT` |
| `Forbidden(String)` | Authenticated principal lacks permission | 403 | `FORBIDDEN` |
| `PolicyExpired(String)` | Vault policy authority has expired | 403 | `POLICY_EXPIRED` |
| `Named(ErrorCode, String)` | A condition with a stable specific code | Code-defined | Broad legacy category |

`AppError::status_code()` and `AppError::error_code()` expose the selected HTTP status and named code. `AppError::diagnostic()` is for logs, not response bodies.

## Named error codes

The canonical spelling is uppercase snake case. The values below are the stable identifiers emitted in the `code` field.

| Code | HTTP status | Meaning and recovery guidance |
| --- | ---: | --- |
| `UNAUTHORIZED` | 401 | Authentication is required or the presented credential is not accepted. |
| `FORBIDDEN` | 403 | The principal is authenticated but is not allowed to perform the operation. |
| `INVALID_API_KEY` | 401 | The API key is absent, malformed, or not accepted. |
| `TOKEN_EXPIRED` | 401 | The access or refresh token has expired; obtain a new credential. |
| `INVALID_SIGNATURE` | 401 | A token, transaction, or request signature could not be verified. |
| `MANAGER_NOT_FOUND` | 404 | No manager record exists for the supplied identifier. |
| `POLICY_EXPIRED` | 403 | The policy no longer authorises the requested operation. |
| `BAD_REQUEST` | 400 | General request cannot be processed. |
| `INVALID_INPUT` | 400 | A field or parameter has an invalid value. |
| `INVALID_JSON` | 400 | JSON syntax or JSON data shape is invalid. |
| `INVALID_CONTRACT_ID` | 400 | Contract identifier is not valid. |
| `INVALID_WASM` | 400 | WASM bytes are malformed or incompatible. |
| `INVALID_PARAMETERS` | 400 | JSON-RPC or operation parameters are invalid. |
| `INVALID_BASE64` | 400 | Base64 input could not be decoded. |
| `INVALID_XDR` | 400 | XDR input could not be decoded or encoded. |
| `PAYLOAD_TOO_LARGE` | 413 | Request body exceeds the applicable route limit. |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | Request content type is not supported. |
| `PARSE_ERROR` | 400 | A parser rejected a request value. |
| `VALIDATION_FAILED` | 400 | DTO validation failed. |
| `NOT_FOUND` | 404 | Generic resource was not found. |
| `VAULT_NOT_FOUND` | 404 | Vault does not exist or is not visible to the caller. |
| `MANAGER_ALREADY_EXISTS` | 409 | A manager is already registered for the address. |
| `JOB_NOT_FOUND` | 404 | Job does not exist. |
| `JOB_CANNOT_BE_CANCELLED` | 400 | Job is in a state that cannot be cancelled. |
| `RECONCILIATION_NOT_FOUND` | 404 | Reconciliation job or report was not found. |
| `ALREADY_EXISTS` | 409 | Resource already exists. |
| `CONFLICT` | 409 | Request lost a version or state race; reload and retry. |
| `STATE_MISMATCH` | 409 | Resource state does not permit the operation. |
| `RATE_LIMIT_EXCEEDED` | 429 | Client or endpoint token bucket is exhausted. |
| `TOO_MANY_REQUESTS` | 429 | General rate limit was exceeded. |
| `CIRCUIT_BREAKER_OPEN` | 503 | RPC provider circuit is open or being probed in half-open mode. |
| `SIMULATION_FAILED` | 400 | Simulation could not be completed. |
| `CONTRACT_EXECUTION_FAILED` | 400 | Contract execution failed. |
| `NODE_ERROR` | 400 | RPC node rejected the request. |
| `RPC_NODE_ERROR` | 400 | RPC node returned a request-level error. |
| `RPC_REQUEST_FAILED` | 500 | RPC request failed after transport or retry handling. |
| `RPC_TIMEOUT` | 504 | RPC request exceeded its deadline. |
| `NODE_TIMEOUT` | 504 | RPC node did not respond before its deadline. |
| `NO_HEALTHY_RPC_PROVIDERS` | 503 | No RPC provider is currently available. |
| `LOCAL_UNAVAILABLE` | 500 | Local execution is unavailable and no fallback succeeded. |
| `CONSENSUS_MISMATCH` | 500 | Providers returned divergent simulation results. |
| `INSUFFICIENT_CONSENSUS` | 500 | Too few providers are available for consensus mode. |
| `INSUFFICIENT_BALANCE` | 400 | Account balance is insufficient. |
| `INSUFFICIENT_LIQUIDITY` | 400 | Pool liquidity is insufficient. |
| `INSUFFICIENT_SHARES` | 400 | Share balance is insufficient. |
| `INSUFFICIENT_ALLOWANCE` | 400 | Token allowance is insufficient. |
| `SLIPPAGE_EXCEEDED` | 400 | Price movement exceeds the accepted slippage. |
| `INVALID_FEE` | 400 | Fee value is invalid. |
| `ORACLE_NOT_CONFIGURE` | 500 | No usable oracle is configured. |
| `INVALID_ORACLE_PRICE` | 500 | Oracle returned an unusable price. |
| `CONTRACT_PAUSED` | 400 | Contract is paused. |
| `INTERNAL_SERVER_ERROR` | 500 | Unexpected server failure. |
| `DATABASE_ERROR` | 500 | Database operation failed. |
| `NETWORK_ERROR` | 500 | Network operation failed. |
| `IO_ERROR` | 500 | Filesystem or stream I/O failed. |
| `SERIALIZATION_ERROR` | 500 | Response or request serialization failed. |
| `SERVICE_UNAVAILABLE` | 503 | Dependency or service is temporarily unavailable. |
| `CONFIGURATION_ERROR` | 500 | Required configuration is missing or invalid. |
| `RECONCILIATION_FAILED` | 500 | Reconciliation could not be completed. |
| `METHOD_NOT_ALLOWED` | 405 | HTTP method is not supported for the route. |
| `UNSUPPORTED_API_VERSION` | 400 | Requested API version is not supported. |
| `REQUEST_BODY_READ_FAILED` | 400 | Request body could not be read. |

`ErrorCode` also implements `FromStr`, so an SDK or internal component can parse a canonical code without maintaining a second string table.

## Conversion guide

### Simulation errors

| Source variant | `ErrorCode` | HTTP status |
| --- | --- | ---: |
| `SimulationError::NodeError` | `NODE_ERROR` | 400 |
| `SimulationError::InvalidContract` | `INVALID_CONTRACT_ID` | 400 |
| `SimulationError::InvalidWasm` | `INVALID_WASM` | 400 |
| `SimulationError::ParseError` | `PARSE_ERROR` | 400 |
| `SimulationError::XdrError` | `INVALID_XDR` | 400 |
| `SimulationError::Base64Error` | `INVALID_BASE64` | 400 |
| `SimulationError::NodeTimeout` | `RPC_TIMEOUT` | 504 |
| `SimulationError::RpcRequestFailed` | `RPC_REQUEST_FAILED` | 500 |
| `SimulationError::AllAttemptsFailed` | `RPC_REQUEST_FAILED` | 500 |
| `SimulationError::CircuitBreakerOpen` | `CIRCUIT_BREAKER_OPEN` | 503 |
| `SimulationError::NetworkError` | `NETWORK_ERROR` | 500 |
| `SimulationError::Io` | `IO_ERROR` | 500 |
| `SimulationError::SerializationError` | `SERIALIZATION_ERROR` | 500 |
| `SimulationError::LocalUnavailable` | `LOCAL_UNAVAILABLE` | 500 |
| `SimulationError::ExecutionFailed` | `CONTRACT_EXECUTION_FAILED` | 400 |
| `SimulationError::InsufficientConsensusProviders` | `INSUFFICIENT_CONSENSUS` | 500 |
| `SimulationError::ConsensusMismatch` | `CONSENSUS_MISMATCH` | 500 |

### Stellar RPC errors

| Source variant | `ErrorCode` | HTTP status |
| --- | --- | ---: |
| `StellarServiceError::CircuitOpen` | `CIRCUIT_BREAKER_OPEN` | 503 |
| `StellarServiceError::NoHealthyProviders` | `NO_HEALTHY_RPC_PROVIDERS` | 503 |
| `StellarServiceError::Timeout` | `RPC_TIMEOUT` | 504 |
| `StellarServiceError::Network` | `NETWORK_ERROR` | 500 |
| `StellarServiceError::HttpError` | `RPC_REQUEST_FAILED` | 500 |
| `StellarServiceError::ParseError` | `SERIALIZATION_ERROR` | 500 |
| `StellarServiceError::AllAttemptsFailed` | `RPC_REQUEST_FAILED` | 500 |
| `StellarServiceError::MissingNetworkPassphrase` | `CONFIGURATION_ERROR` | 500 |
| `StellarServiceError::NetworkPassphraseMismatch` | `CONFIGURATION_ERROR` | 500 |
| `StellarServiceError::ClientBuild` | `CONFIGURATION_ERROR` | 500 |

### Resource and job errors

| Source | Named code |
| --- | --- |
| `VaultStoreError::NotFound` | `VAULT_NOT_FOUND` |
| `VaultStoreError::Conflict` | `CONFLICT` |
| `VaultStoreError::InvalidData` | `INVALID_INPUT` |
| `ManagerStoreError::NotFound` | `MANAGER_NOT_FOUND` |
| `ManagerStoreError::DuplicateAddress` | `MANAGER_ALREADY_EXISTS` |
| `ManagerStoreError::InvalidData` | `INVALID_INPUT` |
| `JobError::NotFound` | `JOB_NOT_FOUND` |
| `JobError::CannotCancel` | `JOB_CANNOT_BE_CANCELLED` |
| `ReconciliationError::NoData` | `INVALID_INPUT` |
| `ReconciliationError::InvalidRange` | `INVALID_INPUT` |
| `ReconciliationError::StoreError` | `DATABASE_ERROR` |

## Client handling rules

1. Branch on `code`, not on the human-readable `message`.
2. Retry `RPC_TIMEOUT`, `NETWORK_ERROR`, `SERVICE_UNAVAILABLE`, and `CIRCUIT_BREAKER_OPEN` only with bounded backoff and jitter.
3. Do not retry `INVALID_JSON`, `INVALID_INPUT`, `INVALID_SIGNATURE`, or other deterministic client errors without changing the request.
4. Treat `RATE_LIMIT_EXCEEDED` as retryable only after the advertised `Retry-After` interval.
5. Preserve `error` when forwarding or displaying legacy responses; new integrations should use `code`.
6. Log `message` and `details` as untrusted input, and never treat them as executable markup or commands.
