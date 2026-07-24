use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::{info, Span};
use uuid::Uuid;

const CORRELATION_ID_HEADER: &str = "x-correlation-id";
const REQUEST_ID_HEADER: &str = "x-request-id";

pub async fn correlation_id_middleware(request: Request, next: Next) -> Response {
    let correlation_id = request
        .headers()
        .get(CORRELATION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let request_id = Uuid::new_v4().to_string();

    let span = tracing::info_span!(
        "http_request",
        correlation_id = %correlation_id,
        request_id = %request_id,
        method = %request.method(),
        uri = %request.uri(),
    );

    let mut request = request;
    request.headers_mut().insert(
        HeaderName::from_static(CORRELATION_ID_HEADER),
        HeaderValue::from_str(&correlation_id).unwrap(),
    );
    request.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        HeaderValue::from_str(&request_id).unwrap(),
    );

    let _enter = span.enter();
    let start = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();

    let mut response = next.run(request).await;

    let latency = start.elapsed();
    let status = response.status();

    info!(
        correlation_id = %correlation_id,
        request_id = %request_id,
        method = %method,
        uri = %uri,
        status = %status,
        latency_ms = latency.as_millis(),
        "Request completed"
    );

    response.headers_mut().insert(
        HeaderName::from_static(CORRELATION_ID_HEADER),
        HeaderValue::from_str(&correlation_id).unwrap(),
    );
    response.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        HeaderValue::from_str(&request_id).unwrap(),
    );

    response
}