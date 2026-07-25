//! Global exception-filter style handlers for Axum.
//!
//! Covers gaps that [`crate::errors::AppError`] alone does not:
//! unknown routes (404 fallback) and panicking handlers (`CatchPanicLayer`).

use std::any::Any;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use tracing::error;

use crate::errors::{AppError, ErrorResponse};

/// 404 fallback that returns the standard `{ error, message }` envelope.
pub async fn fallback_handler() -> impl IntoResponse {
    AppError::NotFound("The requested resource was not found".into())
}

/// Panic response builder for [`tower_http::catch_panic::CatchPanicLayer`].
pub fn handle_panic(err: Box<dyn Any + Send + 'static>) -> Response {
    let panic_message = if let Some(msg) = err.downcast_ref::<&str>() {
        (*msg).to_string()
    } else if let Some(msg) = err.downcast_ref::<String>() {
        msg.clone()
    } else {
        "Unknown panic".to_string()
    };

    error!(panic = %panic_message, "Request handler panicked");

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "INTERNAL_SERVER_ERROR".to_string(),
            message: "An unexpected error occurred".to_string(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt;
    use tower_http::catch_panic::CatchPanicLayer;

    async fn ok() -> &'static str {
        "ok"
    }

    async fn boom() -> &'static str {
        panic!("intentional test panic");
    }

    fn test_router() -> Router {
        Router::new()
            .route("/ok", get(ok))
            .route("/boom", get(boom))
            .fallback(fallback_handler)
            .layer(CatchPanicLayer::custom(handle_panic))
    }

    #[tokio::test]
    async fn unknown_route_returns_not_found_envelope() {
        let app = test_router();
        let req = Request::builder()
            .uri("/does-not-exist")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "NOT_FOUND");
        assert!(body["message"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn panic_returns_internal_error_envelope() {
        let app = test_router();
        let req = Request::builder()
            .uri("/boom")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "INTERNAL_SERVER_ERROR");
        assert_eq!(body["message"], "An unexpected error occurred");
    }
}
