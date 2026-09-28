use crate::errors::AppError;
use axum::{
    async_trait,
    body::Bytes,
    extract::{rejection::JsonRejection, FromRequest, Json, Path, Query, Request},
    http::header,
    response::{IntoResponse, Response},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use thiserror::Error;

pub const DEFAULT_MAX_STRING_LENGTH: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAX_COLLECTION_LENGTH: usize = 16_384;
pub const DEFAULT_MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct SanitizationLimits {
    pub max_string_length: usize,
    pub max_collection_length: usize,
    pub max_depth: usize,
}

impl Default for SanitizationLimits {
    fn default() -> Self {
        Self {
            max_string_length: DEFAULT_MAX_STRING_LENGTH,
            max_collection_length: DEFAULT_MAX_COLLECTION_LENGTH,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SanitizationError {
    #[error("input string exceeds the maximum length")]
    StringTooLong,
    #[error("input contains a control character")]
    ControlCharacter,
    #[error("input collection exceeds the maximum length")]
    CollectionTooLong,
    #[error("input nesting exceeds the maximum depth")]
    DepthExceeded,
    #[error("input contains duplicate object keys after normalization")]
    DuplicateKey,
    #[error("input could not be serialized")]
    Serialization,
    #[error("input could not be deserialized")]
    Deserialization,
}

pub fn sanitize_text(value: &str) -> Result<String, SanitizationError> {
    sanitize_text_with_limits(value, SanitizationLimits::default())
}

pub fn sanitize_text_with_limits(
    value: &str,
    limits: SanitizationLimits,
) -> Result<String, SanitizationError> {
    let normalized = value.trim();
    if normalized.chars().count() > limits.max_string_length {
        return Err(SanitizationError::StringTooLong);
    }
    if normalized
        .chars()
        .any(|character| character.is_control() || is_format_control(character))
    {
        return Err(SanitizationError::ControlCharacter);
    }
    Ok(normalized.to_owned())
}

fn is_format_control(character: char) -> bool {
    matches!(
        character as u32,
        0x200B..=0x200F | 0x2028..=0x202E | 0x2066..=0x2069 | 0xFEFF
    )
}

pub fn sanitize_json(value: &Value) -> Result<Value, SanitizationError> {
    sanitize_json_with_limits(value, SanitizationLimits::default())
}

pub fn sanitize_json_with_limits(
    value: &Value,
    limits: SanitizationLimits,
) -> Result<Value, SanitizationError> {
    sanitize_json_inner(value, 0, limits)
}

fn sanitize_json_inner(
    value: &Value,
    depth: usize,
    limits: SanitizationLimits,
) -> Result<Value, SanitizationError> {
    if depth > limits.max_depth {
        return Err(SanitizationError::DepthExceeded);
    }

    match value {
        Value::String(text) => Ok(Value::String(sanitize_text_with_limits(text, limits)?)),
        Value::Array(values) => {
            if values.len() > limits.max_collection_length {
                return Err(SanitizationError::CollectionTooLong);
            }
            values
                .iter()
                .map(|item| sanitize_json_inner(item, depth + 1, limits))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Value::Object(entries) => {
            if entries.len() > limits.max_collection_length {
                return Err(SanitizationError::CollectionTooLong);
            }
            let mut sanitized = Map::new();
            for (key, item) in entries {
                let normalized_key = sanitize_text_with_limits(key, limits)?;
                if sanitized.contains_key(&normalized_key) {
                    return Err(SanitizationError::DuplicateKey);
                }
                sanitized.insert(
                    normalized_key,
                    sanitize_json_inner(item, depth + 1, limits)?,
                );
            }
            Ok(Value::Object(sanitized))
        }
        _ => Ok(value.clone()),
    }
}

pub fn deserialize_sanitized<T>(value: Value) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    let sanitized = sanitize_json(&value).map_err(sanitization_error)?;
    serde_json::from_value(sanitized).map_err(|_| AppError::BadRequest("Invalid request data".into()))
}

pub fn sanitization_error(error: SanitizationError) -> AppError {
    AppError::BadRequest(error.to_string())
}

pub fn json_rejection_error(rejection: JsonRejection) -> AppError {
    let message = match &rejection {
        JsonRejection::JsonDataError(error) => {
            format!("Invalid JSON data: {}", error.body_text())
        }
        JsonRejection::JsonSyntaxError(error) => {
            format!("JSON syntax error: {}", error.body_text())
        }
        JsonRejection::MissingJsonContentType(_) => {
            "Content-Type must be application/json".to_string()
        }
        JsonRejection::BytesRejection(_) => "Failed to read request body".to_string(),
        _ => "Invalid request body".to_string(),
    };
    AppError::BadRequest(message)
}

fn is_json_content_type(value: &str) -> bool {
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type.starts_with("application/")
        && (media_type == "application/json" || media_type.ends_with("+json"))
}

fn reject_duplicate_json_keys(bytes: &[u8]) -> Result<(), SanitizationError> {
    let mut object_keys = Vec::<HashSet<String>>::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                object_keys.push(HashSet::new());
                index += 1;
            }
            b'}' => {
                let _ = object_keys.pop();
                index += 1;
            }
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index = index.saturating_add(2),
                        b'"' => break,
                        _ => index += 1,
                    }
                }
                if index >= bytes.len() {
                    return Err(SanitizationError::Deserialization);
                }
                let raw = &bytes[start..=index];
                let key = serde_json::from_slice::<String>(raw)
                    .map_err(|_| SanitizationError::Deserialization)?;
                index += 1;
                let mut next = index;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if bytes.get(next) == Some(&b':') {
                    if let Some(keys) = object_keys.last_mut() {
                        let normalized = sanitize_text_with_limits(
                            &key,
                            SanitizationLimits::default(),
                        )?;
                        if !keys.insert(normalized) {
                            return Err(SanitizationError::DuplicateKey);
                        }
                    }
                }
            }
            _ => index += 1,
        }
    }
    Ok(())
}

pub struct SanitizedJson<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for SanitizedJson<T>
where
    T: DeserializeOwned + Send + 'static,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(is_json_content_type)
            .unwrap_or(false);
        if !is_json {
            return Err(AppError::BadRequest(
                "Content-Type must be application/json".into(),
            ));
        }
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|_| AppError::BadRequest("Failed to read request body".into()))?;
        reject_duplicate_json_keys(&bytes).map_err(sanitization_error)?;
        let Json(value) = Json::<Value>::from_bytes(&bytes).map_err(json_rejection_error)?;
        deserialize_sanitized(value).map(SanitizedJson)
    }
}

pub struct SanitizedQuery<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for SanitizedQuery<T>
where
    T: DeserializeOwned + Serialize + Send + 'static,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request(req, state)
            .await
            .map_err(|_| AppError::BadRequest("Invalid query parameters".into()))?;
        let serialized = serde_json::to_value(&value)
            .map_err(|_| AppError::BadRequest("Invalid query parameters".into()))?;
        deserialize_sanitized(serialized).map(SanitizedQuery)
    }
}

pub struct SanitizedPath<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for SanitizedPath<T>
where
    T: DeserializeOwned + Serialize + Send + 'static,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Path(value) = Path::<T>::from_request(req, state)
            .await
            .map_err(|_| AppError::BadRequest("Invalid path parameters".into()))?;
        let serialized = serde_json::to_value(&value)
            .map_err(|_| AppError::BadRequest("Invalid path parameters".into()))?;
        deserialize_sanitized(serialized).map(SanitizedPath)
    }
}

pub fn sanitize_multipart_text(value: &str, field: &str) -> Result<String, AppError> {
    sanitize_text(value).map_err(|error| {
        AppError::BadRequest(format!("Invalid {}: {}", field, error))
    })
}

pub async fn sanitize_request_middleware(request: Request, next: axum::middleware::Next) -> Response {
    let is_json = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(is_json_content_type)
        .unwrap_or(false);

    if !is_json {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 2 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return AppError::BadRequest("Failed to read request body".into()).into_response();
        }
    };

    if let Err(error) = reject_duplicate_json_keys(&bytes) {
        return sanitization_error(error).into_response();
    }

    let value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(_) => {
            let request = rebuild_json_request(parts, bytes.to_vec());
            return next.run(request).await;
        }
    };

    let sanitized = match sanitize_json(&value) {
        Ok(value) => value,
        Err(error) => return sanitization_error(error).into_response(),
    };
    let encoded = match serde_json::to_vec(&sanitized) {
        Ok(encoded) => encoded,
        Err(_) => {
            return AppError::BadRequest("Failed to encode request body".into()).into_response();
        }
    };

    let request = rebuild_json_request(parts, encoded);
    next.run(request).await
}

fn rebuild_json_request(parts: axum::http::request::Parts, body: Vec<u8>) -> Request {
    let content_length = body.len();
    let mut request = Request::from_parts(parts, axum::body::Body::from(body));
    request.headers_mut().remove(header::CONTENT_LENGTH);
    if let Ok(value) = axum::http::HeaderValue::from_str(&content_length.to_string()) {
        request.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    request
}
