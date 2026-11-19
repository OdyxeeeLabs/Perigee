use crate::errors::AppError;
use axum::{
    async_trait,
    body::Bytes,
    extract::{rejection::JsonRejection, FromRequest, Json, Path, Query, Request},
    http::{header, HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
use thiserror::Error;

pub const DEFAULT_MAX_STRING_LENGTH: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAX_COLLECTION_LENGTH: usize = 16_384;
pub const DEFAULT_MAX_DEPTH: usize = 64;

/// Default maximum JSON request body size (API-33), matching the
/// `DefaultBodyLimit` value already used across the router. Override with
/// `MAX_REQUEST_BODY_BYTES` (bytes).
pub const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// The configured max request body size in bytes, read fresh from
/// `MAX_REQUEST_BODY_BYTES` on every call (falls back to
/// [`DEFAULT_MAX_BODY_BYTES`] when unset, empty, non-numeric, or zero).
pub fn max_body_bytes() -> usize {
    env_limit("MAX_REQUEST_BODY_BYTES", DEFAULT_MAX_BODY_BYTES)
}

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

impl SanitizationLimits {
    /// Load limits from the environment (API-33), falling back to the
    /// defaults above whenever a variable is absent, empty, non-numeric, or
    /// zero. Keeping a single `from_env()` here means every call site that
    /// enforces these limits — [`sanitize_json`], [`check_structural_limits`],
    /// and `sanitize_request_middleware` — can be retuned per-deployment
    /// without a recompile.
    ///
    /// | Variable                 | Meaning                                   | Default |
    /// |--------------------------|--------------------------------------------|-------:|
    /// | `MAX_STRING_LENGTH`      | max characters per string field             | 2 MiB chars |
    /// | `MAX_COLLECTION_LENGTH`  | max items in an array / keys in an object   | 16384  |
    /// | `MAX_NESTING_DEPTH`      | max levels of nested arrays/objects         | 64     |
    pub fn from_env() -> Self {
        Self {
            max_string_length: env_limit("MAX_STRING_LENGTH", DEFAULT_MAX_STRING_LENGTH),
            max_collection_length: env_limit("MAX_COLLECTION_LENGTH", DEFAULT_MAX_COLLECTION_LENGTH),
            max_depth: env_limit("MAX_NESTING_DEPTH", DEFAULT_MAX_DEPTH),
        }
    }
}

/// Parses `key` as a positive `usize`, falling back to `default` when the
/// variable is unset, empty, non-numeric, or zero (zero would make every
/// request fail, which is never the intent of an unset/misconfigured limit).
fn env_limit(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|parsed| *parsed > 0)
        .unwrap_or(default)
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
    #[error("validation failed: {0}")]
    ValidationFailed(String),
    #[error("invalid character '{0}' at position {1}")]
    InvalidCharacter(char, usize),
    #[error("value is not in the allowed list: '{0}'")]
    ValueNotAllowed(String),
    #[error("path traversal detected: {0}")]
    PathTraversal(String),
    #[error("invalid filename: {0}")]
    InvalidFilename(String),
    #[error("invalid SQL identifier: '{0}'")]
    InvalidSqlIdentifier(String),
    #[error("unparameterized query error: {0}")]
    UnparameterizedQuery(String),
    #[error("file upload rejected: {0}")]
    FileUploadRejected(String),
    #[error("malicious file content detected: {0}")]
    MaliciousContent(String),
    #[error("MIME type mismatch: declared '{declared}', detected '{detected}'")]
    MimeTypeMismatch {
        declared: String,
        detected: String,
    },
}

// ============================================================================
// Text and JSON Sanitization
// ============================================================================

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
    sanitize_json_with_limits(value, SanitizationLimits::from_env())
}

pub fn sanitize_json_with_limits(
    value: &Value,
    limits: SanitizationLimits,
) -> Result<Value, SanitizationError> {
    sanitize_json_inner(value, 0, limits)
}

/// Checks only structure — array/object length and nesting depth — without
/// touching string content (API-33).
///
/// Used at the request-extractor boundary (`ApiJson`, `ValidatedJson` in
/// `errors.rs`) to reject oversized or deeply nested payloads *before* a
/// handler, or any DB-mutating call it makes, ever sees them. Unlike
/// [`sanitize_json_with_limits`], this never rewrites the value and never
/// rejects on string content (control characters, length) — it is a narrow
/// defence against memory blow-up from huge/deeply-nested collections, not a
/// replacement for the existing content sanitization pipelines.
pub fn check_structural_limits(
    value: &Value,
    limits: &SanitizationLimits,
) -> Result<(), SanitizationError> {
    check_structural_limits_at(value, 0, limits)
}

fn check_structural_limits_at(
    value: &Value,
    depth: usize,
    limits: &SanitizationLimits,
) -> Result<(), SanitizationError> {
    if depth > limits.max_depth {
        return Err(SanitizationError::DepthExceeded);
    }
    match value {
        Value::Array(items) => {
            if items.len() > limits.max_collection_length {
                return Err(SanitizationError::CollectionTooLong);
            }
            for item in items {
                check_structural_limits_at(item, depth + 1, limits)?;
            }
        }
        Value::Object(entries) => {
            if entries.len() > limits.max_collection_length {
                return Err(SanitizationError::CollectionTooLong);
            }
            for item in entries.values() {
                check_structural_limits_at(item, depth + 1, limits)?;
            }
        }
        _ => {}
    }
    Ok(())
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

// ============================================================================
// Axum Extractors
// ============================================================================

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

pub async fn sanitize_request_middleware(request: Request, next: Next) -> Response {
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
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        request.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    request
}

// ============================================================================
// Layer 1: Parameterized Queries (No String Concatenation)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
}

impl SqlValue {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }
    pub fn int(i: i64) -> Self {
        Self::Integer(i)
    }
    pub fn float(f: f64) -> Self {
        Self::Float(f)
    }
    pub fn bool(b: bool) -> Self {
        Self::Boolean(b)
    }
    pub fn bytes(b: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(b.into())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterizedQuery {
    sql: String,
    params: Vec<SqlValue>,
}

impl ParameterizedQuery {
    pub fn new(sql: impl Into<String>, params: Vec<SqlValue>) -> Result<Self, SanitizationError> {
        let sql = sql.into();
        let placeholder_count = count_sql_placeholders(&sql);
        if placeholder_count != params.len() {
            return Err(SanitizationError::UnparameterizedQuery(format!(
                "placeholder count ({}) does not match parameter count ({})",
                placeholder_count,
                params.len()
            )));
        }
        if contains_dangerous_raw_sql_patterns(&sql) {
            return Err(SanitizationError::UnparameterizedQuery(
                "query contains raw SQL comment or dangerous unparameterized token".into(),
            ));
        }
        Ok(Self { sql, params })
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn params(&self) -> &[SqlValue] {
        &self.params
    }
}

fn count_sql_placeholders(sql: &str) -> usize {
    let mut count = 0;
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            count += 1;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        } else if chars[i] == '?' {
            count += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    count
}

fn contains_dangerous_raw_sql_patterns(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    lower.contains("--") || lower.contains("/*") || lower.contains("*/")
}

// ============================================================================
// Layer 2: ORM / Query Builder with Built-in Protection
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlOp {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
}

impl SqlOp {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::NotEq => "!=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Like => "LIKE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

pub fn escape_like_pattern(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len() * 2);
    for c in pattern.chars() {
        if c == '%' || c == '_' || c == '\\' {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

pub fn validate_sql_identifier(identifier: &str) -> Result<(), SanitizationError> {
    let trimmed = identifier.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return Err(SanitizationError::InvalidSqlIdentifier(identifier.to_string()));
    }
    let mut chars = trimmed.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(SanitizationError::InvalidSqlIdentifier(identifier.to_string()));
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' {
            return Err(SanitizationError::InvalidSqlIdentifier(identifier.to_string()));
        }
    }
    if is_reserved_sql_keyword(trimmed) {
        return Err(SanitizationError::InvalidSqlIdentifier(format!(
            "reserved keyword: {}",
            identifier
        )));
    }
    Ok(())
}

fn is_reserved_sql_keyword(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "DROP" | "ALTER" | "CREATE" | "TABLE" |
        "FROM" | "WHERE" | "UNION" | "EXEC" | "EXECUTE" | "TRUNCATE" | "GRANT" | "REVOKE" |
        "INTO" | "VALUES" | "HAVING" | "GROUP" | "ORDER" | "BY" | "JOIN" | "INNER" | "OUTER"
    )
}

pub struct SqlQueryBuilder {
    table: String,
    columns: Vec<String>,
    where_clauses: Vec<String>,
    params: Vec<SqlValue>,
    order_by: Option<(String, SortOrder)>,
    limit_val: Option<usize>,
    offset_val: Option<usize>,
}

impl SqlQueryBuilder {
    pub fn select(table: &str) -> Result<Self, SanitizationError> {
        validate_sql_identifier(table)?;
        Ok(Self {
            table: table.to_string(),
            columns: vec!["*".to_string()],
            where_clauses: Vec::new(),
            params: Vec::new(),
            order_by: None,
            limit_val: None,
            offset_val: None,
        })
    }

    pub fn columns(mut self, cols: &[&str]) -> Result<Self, SanitizationError> {
        if cols.is_empty() {
            return Err(SanitizationError::ValidationFailed("column list cannot be empty".into()));
        }
        let mut checked = Vec::with_capacity(cols.len());
        for &col in cols {
            validate_sql_identifier(col)?;
            checked.push(col.to_string());
        }
        self.columns = checked;
        Ok(self)
    }

    pub fn and_where(mut self, column: &str, op: SqlOp, value: SqlValue) -> Result<Self, SanitizationError> {
        validate_sql_identifier(column)?;
        self.params.push(value);
        let param_idx = self.params.len();
        self.where_clauses.push(format!("{} {} ${}", column, op.as_sql(), param_idx));
        Ok(self)
    }

    pub fn and_where_in(mut self, column: &str, values: Vec<SqlValue>) -> Result<Self, SanitizationError> {
        validate_sql_identifier(column)?;
        if values.is_empty() {
            return Err(SanitizationError::ValidationFailed("IN values cannot be empty".into()));
        }
        let mut placeholders = Vec::with_capacity(values.len());
        for val in values {
            self.params.push(val);
            placeholders.push(format!("${}", self.params.len()));
        }
        self.where_clauses.push(format!("{} IN ({})", column, placeholders.join(", ")));
        Ok(self)
    }

    pub fn order_by(mut self, column: &str, order: SortOrder) -> Result<Self, SanitizationError> {
        validate_sql_identifier(column)?;
        self.order_by = Some((column.to_string(), order));
        Ok(self)
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit_val = Some(limit.min(1000));
        self
    }

    pub fn offset(mut self, offset: usize) -> Self {
        self.offset_val = Some(offset);
        self
    }

    pub fn build(self) -> Result<ParameterizedQuery, SanitizationError> {
        let mut sql = format!("SELECT {} FROM {}", self.columns.join(", "), self.table);
        if !self.where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.where_clauses.join(" AND "));
        }
        if let Some((col, order)) = self.order_by {
            sql.push_str(&format!(" ORDER BY {} {}", col, order.as_sql()));
        }
        if let Some(limit) = self.limit_val {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = self.offset_val {
            sql.push_str(&format!(" OFFSET {}", offset));
        }
        ParameterizedQuery::new(sql, self.params)
    }
}

// ============================================================================
// Layer 3: Input Validation (Allowlist & Pattern)
// ============================================================================

pub fn validate_allowlist<T: AsRef<str>>(value: &str, allowed: &[T]) -> Result<(), SanitizationError> {
    if allowed.iter().any(|item| item.as_ref() == value) {
        Ok(())
    } else {
        Err(SanitizationError::ValueNotAllowed(value.to_string()))
    }
}

pub fn validate_stellar_address(address: &str) -> Result<(), SanitizationError> {
    let trimmed = address.trim();
    if trimmed.len() != 56 {
        return Err(SanitizationError::ValidationFailed(format!(
            "Stellar address must be 56 characters, got {}",
            trimmed.len()
        )));
    }
    if !trimmed.starts_with('G') {
        return Err(SanitizationError::ValidationFailed(
            "Stellar public key must start with 'G'".into(),
        ));
    }
    for (i, c) in trimmed.chars().enumerate() {
        if !matches!(c, 'A'..='Z' | '2'..='7') {
            return Err(SanitizationError::InvalidCharacter(c, i));
        }
    }
    Ok(())
}

pub fn validate_contract_id(id: &str) -> Result<(), SanitizationError> {
    let trimmed = id.trim();
    if trimmed.len() == 56 && trimmed.starts_with('C') {
        for (i, c) in trimmed.chars().enumerate() {
            if !matches!(c, 'A'..='Z' | '2'..='7') {
                return Err(SanitizationError::InvalidCharacter(c, i));
            }
        }
        return Ok(());
    }
    if trimmed.len() == 64 {
        for (i, c) in trimmed.chars().enumerate() {
            if !c.is_ascii_hexdigit() {
                return Err(SanitizationError::InvalidCharacter(c, i));
            }
        }
        return Ok(());
    }
    Err(SanitizationError::ValidationFailed(
        "Contract ID must be a 56-character StrKey starting with 'C' or 64 hex characters".into(),
    ))
}

pub fn validate_hex(hex_str: &str, expected_len: Option<usize>) -> Result<(), SanitizationError> {
    let clean = hex_str.strip_prefix("0x").or_else(|| hex_str.strip_prefix("0X")).unwrap_or(hex_str);
    if clean.len() % 2 != 0 {
        return Err(SanitizationError::ValidationFailed("Hex string must have an even length".into()));
    }
    for (i, c) in clean.chars().enumerate() {
        if !c.is_ascii_hexdigit() {
            return Err(SanitizationError::InvalidCharacter(c, i));
        }
    }
    if let Some(expected) = expected_len {
        let byte_len = clean.len() / 2;
        if byte_len != expected {
            return Err(SanitizationError::ValidationFailed(format!(
                "Expected {} bytes, got {}",
                expected, byte_len
            )));
        }
    }
    Ok(())
}

pub fn validate_identifier(name: &str, min_len: usize, max_len: usize) -> Result<(), SanitizationError> {
    let trimmed = name.trim();
    if trimmed.len() < min_len || trimmed.len() > max_len {
        return Err(SanitizationError::ValidationFailed(format!(
            "Identifier length must be between {} and {}, got {}",
            min_len, max_len, trimmed.len()
        )));
    }
    let mut chars = trimmed.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(SanitizationError::InvalidCharacter(first, 0));
    }
    for (i, c) in chars.enumerate() {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
            return Err(SanitizationError::InvalidCharacter(c, i + 1));
        }
    }
    Ok(())
}

pub fn validate_email(email: &str) -> Result<(), SanitizationError> {
    let trimmed = email.trim();
    if trimmed.is_empty() || trimmed.len() > 254 {
        return Err(SanitizationError::ValidationFailed("Email exceeds length limits".into()));
    }
    let parts: Vec<&str> = trimmed.split('@').collect();
    if parts.len() != 2 {
        return Err(SanitizationError::ValidationFailed("Email must contain exactly one '@'".into()));
    }
    let (local, domain) = (parts[0], parts[1]);
    if local.is_empty() || local.len() > 64 || domain.is_empty() || domain.len() > 253 {
        return Err(SanitizationError::ValidationFailed("Email local or domain part invalid length".into()));
    }
    for (i, c) in local.chars().enumerate() {
        if !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-' | '_' | '+' | '!') {
            return Err(SanitizationError::InvalidCharacter(c, i));
        }
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err(SanitizationError::ValidationFailed("Email domain must contain a valid TLD".into()));
    }
    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return Err(SanitizationError::ValidationFailed("Email domain label invalid".into()));
        }
        for (i, c) in label.chars().enumerate() {
            if !c.is_ascii_alphanumeric() && c != '-' {
                return Err(SanitizationError::InvalidCharacter(c, i));
            }
        }
    }
    Ok(())
}

pub fn validate_url(url_str: &str, allowed_schemes: &[&str]) -> Result<(), SanitizationError> {
    let trimmed = url_str.trim();
    if trimmed.contains(|c: char| c.is_control() || c.is_whitespace()) {
        return Err(SanitizationError::ValidationFailed("URL contains whitespace or control characters".into()));
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("vbscript:") || lower.starts_with("file:") {
        return Err(SanitizationError::ValidationFailed("Disallowed URL scheme".into()));
    }
    let parts: Vec<&str> = trimmed.split("://").collect();
    if parts.len() != 2 {
        return Err(SanitizationError::ValidationFailed("URL must contain scheme separator '://'".into()));
    }
    let scheme = parts[0].to_ascii_lowercase();
    if !allowed_schemes.iter().any(|&s| s == scheme) {
        return Err(SanitizationError::ValueNotAllowed(scheme));
    }
    let rest = parts[1];
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() {
        return Err(SanitizationError::ValidationFailed("URL host is missing".into()));
    }
    Ok(())
}

pub fn validate_numeric_range<T: PartialOrd + Copy + std::fmt::Display>(
    value: T,
    min: T,
    max: T,
    field_name: &str,
) -> Result<(), SanitizationError> {
    if value < min || value > max {
        Err(SanitizationError::ValidationFailed(format!(
            "Field '{}' value {} is out of range [{}, {}]",
            field_name, value, min, max
        )))
    } else {
        Ok(())
    }
}

// ============================================================================
// Layer 4: Context-Aware Output Encoding
// ============================================================================

pub fn encode_html(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() + 16);
    for c in input.chars() {
        match c {
            '&' => encoded.push_str("&amp;"),
            '<' => encoded.push_str("&lt;"),
            '>' => encoded.push_str("&gt;"),
            '"' => encoded.push_str("&quot;"),
            '\'' => encoded.push_str("&#x27;"),
            '/' => encoded.push_str("&#x2F;"),
            _ => encoded.push(c),
        }
    }
    encoded
}

pub fn encode_html_attribute(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 3);
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            encoded.push(c);
        } else {
            let _ = std::fmt::Write::write_fmt(&mut encoded, format_args!("&#x{:02X};", c as u32));
        }
    }
    encoded
}

pub fn encode_javascript(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() + 16);
    for c in input.chars() {
        match c {
            '\\' => encoded.push_str("\\\\"),
            '"' => encoded.push_str("\\\""),
            '\'' => encoded.push_str("\\'"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '<' => encoded.push_str("\\u003c"),
            '>' => encoded.push_str("\\u003e"),
            '&' => encoded.push_str("\\u0026"),
            '\u{2028}' => encoded.push_str("\\u2028"),
            '\u{2029}' => encoded.push_str("\\u2029"),
            _ => encoded.push(c),
        }
    }
    encoded
}

pub fn encode_sql_literal(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() + 8);
    for c in input.chars() {
        match c {
            '\'' => encoded.push_str("''"),
            '\\' => encoded.push_str("\\\\"),
            '\0' => {},
            _ => encoded.push(c),
        }
    }
    encoded
}

pub fn encode_css_string(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 2);
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            encoded.push(c);
        } else {
            let _ = std::fmt::Write::write_fmt(&mut encoded, format_args!("\\{:X} ", c as u32));
        }
    }
    encoded
}

pub fn encode_url_component(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 3);
    for byte in input.as_bytes() {
        match *byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => {
                let _ = std::fmt::Write::write_fmt(&mut encoded, format_args!("%{:02X}", byte));
            }
        }
    }
    encoded
}

// ============================================================================
// Layer 5: CSP (Content Security Policy) Headers
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSecurityPolicy {
    pub default_src: Vec<String>,
    pub script_src: Vec<String>,
    pub style_src: Vec<String>,
    pub img_src: Vec<String>,
    pub connect_src: Vec<String>,
    pub font_src: Vec<String>,
    pub object_src: Vec<String>,
    pub frame_ancestors: Vec<String>,
    pub base_uri: Vec<String>,
    pub form_action: Vec<String>,
    pub upgrade_insecure_requests: bool,
    pub block_all_mixed_content: bool,
}

impl Default for ContentSecurityPolicy {
    fn default() -> Self {
        Self::strict_api()
    }
}

impl ContentSecurityPolicy {
    pub fn strict_api() -> Self {
        Self {
            default_src: vec!["'none'".to_string()],
            script_src: vec!["'none'".to_string()],
            style_src: vec!["'none'".to_string()],
            img_src: vec!["'none'".to_string()],
            connect_src: vec!["'none'".to_string()],
            font_src: vec!["'none'".to_string()],
            object_src: vec!["'none'".to_string()],
            frame_ancestors: vec!["'none'".to_string()],
            base_uri: vec!["'none'".to_string()],
            form_action: vec!["'none'".to_string()],
            upgrade_insecure_requests: true,
            block_all_mixed_content: true,
        }
    }

    pub fn strict_web() -> Self {
        Self {
            default_src: vec!["'self'".to_string()],
            script_src: vec!["'self'".to_string()],
            style_src: vec!["'self'".to_string()],
            img_src: vec!["'self'".to_string(), "data:".to_string()],
            connect_src: vec!["'self'".to_string()],
            font_src: vec!["'self'".to_string()],
            object_src: vec!["'none'".to_string()],
            frame_ancestors: vec!["'none'".to_string()],
            base_uri: vec!["'self'".to_string()],
            form_action: vec!["'self'".to_string()],
            upgrade_insecure_requests: true,
            block_all_mixed_content: true,
        }
    }

    pub fn with_nonce(&mut self, nonce: &str) -> &mut Self {
        let nonce_directive = format!("'nonce-{}'", nonce);
        if !self.script_src.contains(&nonce_directive) {
            self.script_src.push(nonce_directive.clone());
        }
        if !self.style_src.contains(&nonce_directive) {
            self.style_src.push(nonce_directive);
        }
        self
    }

    pub fn to_header_value(&self) -> String {
        let mut directives = Vec::new();
        if !self.default_src.is_empty() {
            directives.push(format!("default-src {}", self.default_src.join(" ")));
        }
        if !self.script_src.is_empty() {
            directives.push(format!("script-src {}", self.script_src.join(" ")));
        }
        if !self.style_src.is_empty() {
            directives.push(format!("style-src {}", self.style_src.join(" ")));
        }
        if !self.img_src.is_empty() {
            directives.push(format!("img-src {}", self.img_src.join(" ")));
        }
        if !self.connect_src.is_empty() {
            directives.push(format!("connect-src {}", self.connect_src.join(" ")));
        }
        if !self.font_src.is_empty() {
            directives.push(format!("font-src {}", self.font_src.join(" ")));
        }
        if !self.object_src.is_empty() {
            directives.push(format!("object-src {}", self.object_src.join(" ")));
        }
        if !self.frame_ancestors.is_empty() {
            directives.push(format!("frame-ancestors {}", self.frame_ancestors.join(" ")));
        }
        if !self.base_uri.is_empty() {
            directives.push(format!("base-uri {}", self.base_uri.join(" ")));
        }
        if !self.form_action.is_empty() {
            directives.push(format!("form-action {}", self.form_action.join(" ")));
        }
        if self.upgrade_insecure_requests {
            directives.push("upgrade-insecure-requests".to_string());
        }
        if self.block_all_mixed_content {
            directives.push("block-all-mixed-content".to_string());
        }
        directives.join("; ")
    }

    pub fn apply_to_headers(&self, headers: &mut HeaderMap) {
        if let Ok(value) = HeaderValue::from_str(&self.to_header_value()) {
            headers.insert(HeaderName::from_static("content-security-policy"), value);
        }
    }
}

pub async fn csp_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    ContentSecurityPolicy::strict_api().apply_to_headers(response.headers_mut());
    response
}

// ============================================================================
// Path Traversal Defense
// ============================================================================

pub fn sanitize_path(untrusted: &str, allowed_root: &StdPath) -> Result<PathBuf, SanitizationError> {
    if untrusted.contains('\0') || untrusted.contains("%00") {
        return Err(SanitizationError::PathTraversal("Path contains null byte".into()));
    }
    let decoded = untrusted
        .replace("%2f", "/")
        .replace("%2F", "/")
        .replace("%5c", "/")
        .replace("%5C", "/")
        .replace('\\', "/");

    let mut normalized_parts = Vec::new();
    for part in decoded.split('/') {
        let trimmed = part.trim();
        if trimmed.is_empty() || trimmed == "." {
            continue;
        }
        if trimmed == ".." {
            if normalized_parts.pop().is_none() {
                return Err(SanitizationError::PathTraversal(
                    "Path attempts to traverse above root".into(),
                ));
            }
        } else if trimmed.contains(':') {
            return Err(SanitizationError::PathTraversal(
                "Drive letters or URI schemes forbidden in relative path".into(),
            ));
        } else {
            normalized_parts.push(trimmed);
        }
    }

    let mut resolved = allowed_root.to_path_buf();
    for part in normalized_parts {
        resolved.push(part);
    }
    Ok(resolved)
}

pub fn sanitize_filename(filename: &str) -> Result<String, SanitizationError> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        return Err(SanitizationError::InvalidFilename("Filename cannot be empty".into()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        return Err(SanitizationError::InvalidFilename("Filename contains path separators or null bytes".into()));
    }
    let stripped = trimmed.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if stripped.is_empty() {
        return Err(SanitizationError::InvalidFilename("Filename cannot consist only of dots and spaces".into()));
    }
    let stem = stripped.split('.').next().unwrap_or(stripped).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" |
                               "COM5" | "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2" |
                               "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9") {
        return Err(SanitizationError::InvalidFilename(format!("Reserved device name: {}", stem)));
    }
    if stripped.len() > 255 {
        return Err(SanitizationError::InvalidFilename("Filename exceeds 255 characters".into()));
    }
    for (i, c) in stripped.chars().enumerate() {
        if !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-' | '_') {
            return Err(SanitizationError::InvalidCharacter(c, i));
        }
    }
    Ok(stripped.to_string())
}

// ============================================================================
// File Upload Sanitization
// ============================================================================

#[derive(Debug, Clone)]
pub struct FileUploadConfig {
    pub max_size_bytes: usize,
    pub allowed_mime_types: Vec<String>,
    pub allowed_extensions: Vec<String>,
    pub require_magic_bytes: bool,
    pub virus_scan_enabled: bool,
}

impl Default for FileUploadConfig {
    fn default() -> Self {
        Self {
            max_size_bytes: 10 * 1024 * 1024,
            allowed_mime_types: vec![
                "application/wasm".to_string(),
                "application/json".to_string(),
                "image/png".to_string(),
                "image/jpeg".to_string(),
                "text/plain".to_string(),
            ],
            allowed_extensions: vec![
                "wasm".to_string(),
                "json".to_string(),
                "png".to_string(),
                "jpg".to_string(),
                "jpeg".to_string(),
                "txt".to_string(),
            ],
            require_magic_bytes: true,
            virus_scan_enabled: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SanitizedUpload {
    pub original_filename: String,
    pub secure_filename: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub data: Vec<u8>,
}

pub fn detect_magic_bytes(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 4 && data[0..4] == [0x00, 0x61, 0x73, 0x6D] {
        return Some("application/wasm");
    }
    if data.len() >= 8 && data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("image/png");
    }
    if data.len() >= 3 && data[0..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg");
    }
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if data.len() >= 5 && &data[0..5] == b"%PDF-" {
        return Some("application/pdf");
    }
    let trimmed = data.iter().find(|&&b| !b.is_ascii_whitespace());
    if let Some(&first) = trimmed {
        if first == b'{' || first == b'[' {
            return Some("application/json");
        }
    }
    None
}

pub fn scan_for_malicious_content(data: &[u8]) -> Result<(), SanitizationError> {
    const MALWARE_TEST_SIGNATURE: &[u8] = b"PERIGEE_INJECTION_SCAN_TEST_SIGNATURE";
    if data.windows(MALWARE_TEST_SIGNATURE.len()).any(|w| w == MALWARE_TEST_SIGNATURE) {
        return Err(SanitizationError::MaliciousContent("Known test signature pattern detected".into()));
    }
    if data.len() >= 2 && &data[0..2] == b"MZ" {
        return Err(SanitizationError::MaliciousContent("PE Windows executable header detected".into()));
    }
    if data.len() >= 4 && &data[0..4] == b"\x7fELF" {
        return Err(SanitizationError::MaliciousContent("ELF executable header detected".into()));
    }
    if data.len() >= 4 && (&data[0..4] == b"\xca\xfe\xba\xbe" || &data[0..4] == b"\xcf\xfa\xed\xfe") {
        return Err(SanitizationError::MaliciousContent("Mach-O executable header detected".into()));
    }
    let dangerous_patterns: &[&[u8]] = &[
        b"<script",
        b"</script>",
        b"javascript:",
    ];
    for &pattern in dangerous_patterns {
        if data.windows(pattern.len()).any(|w| w.eq_ignore_ascii_case(pattern)) {
            return Err(SanitizationError::MaliciousContent(format!(
                "Suspicious script pattern detected: {:?}",
                String::from_utf8_lossy(pattern)
            )));
        }
    }
    Ok(())
}

pub fn sanitize_file_upload(
    filename: &str,
    declared_mime: &str,
    data: &[u8],
    config: &FileUploadConfig,
) -> Result<SanitizedUpload, SanitizationError> {
    if data.len() > config.max_size_bytes {
        return Err(SanitizationError::FileUploadRejected(format!(
            "File size {} exceeds limit {}",
            data.len(),
            config.max_size_bytes
        )));
    }
    let safe_name = sanitize_filename(filename)?;
    let ext = safe_name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !config.allowed_extensions.iter().any(|e| e == &ext) {
        return Err(SanitizationError::FileUploadRejected(format!(
            "Extension '{}' is not allowed",
            ext
        )));
    }
    let normalized_mime = declared_mime.to_ascii_lowercase();
    if !config.allowed_mime_types.iter().any(|m| m == &normalized_mime) {
        return Err(SanitizationError::FileUploadRejected(format!(
            "MIME type '{}' is not allowed",
            declared_mime
        )));
    }
    if config.require_magic_bytes {
        if let Some(detected) = detect_magic_bytes(data) {
            if detected != normalized_mime {
                return Err(SanitizationError::MimeTypeMismatch {
                    declared: normalized_mime,
                    detected: detected.to_string(),
                });
            }
        }
    }
    if config.virus_scan_enabled {
        scan_for_malicious_content(data)?;
    }
    let random_id = uuid::Uuid::new_v4().to_string();
    let secure_filename = format!("{}.{}", random_id, ext);

    Ok(SanitizedUpload {
        original_filename: safe_name,
        secure_filename,
        mime_type: normalized_mime,
        size_bytes: data.len(),
        data: data.to_vec(),
    })
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sanitize_text_normal() {
        assert_eq!(sanitize_text("  hello world  ").unwrap(), "hello world");
        assert!(sanitize_text("hello\x00world").is_err());
        assert!(sanitize_text("hello\u{200B}world").is_err());
    }

    #[test]
    fn test_parameterized_query_builder() {
        let query = SqlQueryBuilder::select("vaults")
            .unwrap()
            .columns(&["id", "name", "owner", "balance"])
            .unwrap()
            .and_where("owner", SqlOp::Eq, SqlValue::text("GABC..."))
            .unwrap()
            .and_where("balance", SqlOp::Gte, SqlValue::int(1000))
            .unwrap()
            .order_by("created_at", SortOrder::Desc)
            .unwrap()
            .limit(50)
            .offset(0)
            .build()
            .unwrap();

        assert_eq!(
            query.sql(),
            "SELECT id, name, owner, balance FROM vaults WHERE owner = $1 AND balance >= $2 ORDER BY created_at DESC LIMIT 50 OFFSET 0"
        );
        assert_eq!(query.params().len(), 2);
    }

    #[test]
    fn test_sql_injection_rejection() {
        assert!(SqlQueryBuilder::select("vaults; DROP TABLE users;").is_err());
        assert!(validate_sql_identifier("vault_name").is_ok());
        assert!(validate_sql_identifier("SELECT").is_err());
        assert!(validate_sql_identifier("123bad").is_err());
        assert!(validate_sql_identifier("user-table").is_err());
    }

    #[test]
    fn test_escape_like_pattern() {
        assert_eq!(escape_like_pattern("100%_growth"), "100\\%\\_growth");
    }

    #[test]
    fn test_input_validation() {
        assert!(validate_stellar_address("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN").is_ok());
        assert!(validate_stellar_address("BA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN").is_err());
        assert!(validate_contract_id("CA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN").is_ok());
        assert!(validate_hex("deadbeef0123456789abcdef", Some(12)).is_ok());
        assert!(validate_hex("deadbeef_invalid", None).is_err());
        assert!(validate_email("user.name+tag@example.com").is_ok());
        assert!(validate_email("invalid@@example.com").is_err());
        assert!(validate_url("https://perigee.io/api/v1", &["https"]).is_ok());
        assert!(validate_url("javascript:alert(1)", &["https"]).is_err());
        assert!(validate_numeric_range(50, 1, 100, "limit").is_ok());
        assert!(validate_numeric_range(150, 1, 100, "limit").is_err());
    }

    #[test]
    fn test_output_encoding() {
        assert_eq!(
            encode_html("<script>alert('xss & \"hack\"')</script>"),
            "&lt;script&gt;alert(&#x27;xss &amp; &quot;hack&quot;&#x27;)&lt;&#x2F;script&gt;"
        );
        assert_eq!(
            encode_javascript("</script><script>alert('xss')</script>"),
            "\\u003c/script\\u003e\\u003cscript\\u003ealert(\\'xss\\')\\u003c/script\\u003e"
        );
        assert_eq!(
            encode_sql_literal("O'Reilly; DROP TABLE users;"),
            "O''Reilly; DROP TABLE users;"
        );
        assert_eq!(
            encode_url_component("foo bar+baz?"),
            "foo%20bar%2Bbaz%3F"
        );
    }

    #[test]
    fn test_csp_headers() {
        let api_csp = ContentSecurityPolicy::strict_api();
        assert_eq!(
            api_csp.to_header_value(),
            "default-src 'none'; script-src 'none'; style-src 'none'; img-src 'none'; connect-src 'none'; font-src 'none'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; upgrade-insecure-requests; block-all-mixed-content"
        );

        let mut web_csp = ContentSecurityPolicy::strict_web();
        web_csp.with_nonce("secure-random-nonce");
        let header = web_csp.to_header_value();
        assert!(header.contains("'nonce-secure-random-nonce'"));
    }

    #[test]
    fn test_path_traversal() {
        let base = StdPath::new("/var/vault_data");
        let safe = sanitize_path("uploads/images/photo.png", base).unwrap();
        assert_eq!(safe, PathBuf::from("/var/vault_data/uploads/images/photo.png"));

        assert!(sanitize_path("../../etc/passwd", base).is_err());
        assert!(sanitize_path("uploads/../../..//etc/shadow", base).is_err());
        assert!(sanitize_path("uploads/%2e%2e/secret", base).is_err());
        assert!(sanitize_path("uploads/\0/secret", base).is_err());

        assert!(sanitize_filename("valid_report_2026.json").is_ok());
        assert!(sanitize_filename("../../evil.exe").is_err());
        assert!(sanitize_filename("CON.txt").is_err());
    }

    #[test]
    fn test_file_upload_sanitization() {
        let config = FileUploadConfig::default();
        let wasm_header = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let upload = sanitize_file_upload("contract.wasm", "application/wasm", &wasm_header, &config).unwrap();
        assert_eq!(upload.mime_type, "application/wasm");
        assert!(upload.secure_filename.ends_with(".wasm"));

        // MIME mismatch
        assert!(sanitize_file_upload("image.png", "image/png", &wasm_header, &config).is_err());

        // Antivirus test detection
        let av_sig = b"PERIGEE_INJECTION_SCAN_TEST_SIGNATURE";
        assert!(sanitize_file_upload("test.txt", "text/plain", av_sig, &config).is_err());

        // Suspicious script detection
        let script = b"<script>alert(1)</script>";
        assert!(sanitize_file_upload("script.txt", "text/plain", script, &config).is_err());
    }

    // ── API-33: structural limits (array/collection length, nesting depth) ──

    /// Serializes tests that touch process-global env vars so they don't race
    /// `errors.rs`/`config.rs` tests that also mutate `APP_ENV` et al.
    /// Reuses the crate-wide lock rather than adding a second one.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::errors::env_guard()
    }

    fn nested_array(depth: usize) -> Value {
        let mut value = json!([]);
        for _ in 0..depth {
            value = json!([value]);
        }
        value
    }

    #[test]
    fn collection_length_just_below_at_and_above_the_limit_is_allowed_then_rejected() {
        let limits = SanitizationLimits {
            max_string_length: DEFAULT_MAX_STRING_LENGTH,
            max_collection_length: 3,
            max_depth: DEFAULT_MAX_DEPTH,
        };

        let below = Value::Array((0..2).map(Value::from).collect());
        let at = Value::Array((0..3).map(Value::from).collect());
        let above = Value::Array((0..4).map(Value::from).collect());

        assert!(check_structural_limits(&below, &limits).is_ok(), "2 items (below 3) must pass");
        assert!(check_structural_limits(&at, &limits).is_ok(), "3 items (at the limit) must pass");
        assert_eq!(
            check_structural_limits(&above, &limits).unwrap_err(),
            SanitizationError::CollectionTooLong,
            "4 items (above 3) must be rejected"
        );
    }

    #[test]
    fn object_key_count_is_checked_with_the_same_collection_limit() {
        let limits = SanitizationLimits {
            max_string_length: DEFAULT_MAX_STRING_LENGTH,
            max_collection_length: 2,
            max_depth: DEFAULT_MAX_DEPTH,
        };

        assert!(check_structural_limits(&json!({"a": 1, "b": 2}), &limits).is_ok());
        assert_eq!(
            check_structural_limits(&json!({"a": 1, "b": 2, "c": 3}), &limits).unwrap_err(),
            SanitizationError::CollectionTooLong
        );
    }

    #[test]
    fn nesting_depth_just_below_at_and_above_the_limit_is_allowed_then_rejected() {
        let limits = SanitizationLimits {
            max_string_length: DEFAULT_MAX_STRING_LENGTH,
            max_collection_length: DEFAULT_MAX_COLLECTION_LENGTH,
            max_depth: 3,
        };

        // `nested_array(n)` wraps `n` arrays around an empty one, so the
        // deepest child sits at depth `n`.
        assert!(check_structural_limits(&nested_array(2), &limits).is_ok(), "depth 2 (below 3) must pass");
        assert!(check_structural_limits(&nested_array(3), &limits).is_ok(), "depth 3 (at the limit) must pass");
        assert_eq!(
            check_structural_limits(&nested_array(4), &limits).unwrap_err(),
            SanitizationError::DepthExceeded,
            "depth 4 (above 3) must be rejected"
        );
    }

    #[test]
    fn structural_limits_never_rewrite_or_reject_string_content() {
        // Control characters and over-length strings are a *content*
        // sanitization concern (`sanitize_json`), not a structural one —
        // `check_structural_limits` must leave them alone.
        let limits = SanitizationLimits {
            max_string_length: 1,
            max_collection_length: DEFAULT_MAX_COLLECTION_LENGTH,
            max_depth: DEFAULT_MAX_DEPTH,
        };
        let value = json!({"name": "a very long string that exceeds max_string_length\u{0007}"});
        assert!(check_structural_limits(&value, &limits).is_ok());
    }

    #[test]
    fn sanitization_limits_from_env_overrides_defaults_and_falls_back_on_bad_values() {
        let _env = env_guard();
        unsafe {
            std::env::set_var("MAX_COLLECTION_LENGTH", "7");
            std::env::set_var("MAX_NESTING_DEPTH", "0"); // invalid: falls back to default
            std::env::remove_var("MAX_STRING_LENGTH");
        }

        let limits = SanitizationLimits::from_env();
        assert_eq!(limits.max_collection_length, 7);
        assert_eq!(limits.max_depth, DEFAULT_MAX_DEPTH);
        assert_eq!(limits.max_string_length, DEFAULT_MAX_STRING_LENGTH);

        unsafe {
            std::env::remove_var("MAX_COLLECTION_LENGTH");
            std::env::remove_var("MAX_NESTING_DEPTH");
        }
    }
}
