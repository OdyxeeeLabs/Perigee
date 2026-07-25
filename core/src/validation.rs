//! Request validation — Axum analogue of NestJS `ValidationPipe`.
//!
//! * **transform** — serde deserializes JSON/query into typed DTOs
//! * **whitelist / forbidNonWhitelisted** — `#[serde(deny_unknown_fields)]` on DTOs
//! * **validate** — [`validator::Validate`] field rules via [`ValidatedJson`] /
//!   [`ValidatedQuery`]

use axum::{
    async_trait,
    extract::{
        rejection::{JsonRejection, QueryRejection},
        FromRequest, FromRequestParts, Query, Request,
    },
    http::request::Parts,
    Json,
};
use serde::de::DeserializeOwned;
use validator::Validate;

use crate::errors::AppError;

/// Extractor that deserializes JSON and runs [`Validate`], mapping failures to
/// the shared [`AppError`] envelope (400 for malformed JSON, 422 for schema /
/// field validation errors).
#[derive(Debug, Clone, Copy, Default)]
pub struct ValidatedJson<T>(pub T);

#[async_trait]
impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(json_rejection_to_app_error)?;

        value
            .validate()
            .map_err(|e| AppError::Validation(format_validation_errors(&e)))?;

        Ok(ValidatedJson(value))
    }
}

/// Extractor that deserializes query params and runs [`Validate`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ValidatedQuery<T>(pub T);

#[async_trait]
impl<S, T> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(query_rejection_to_app_error)?;

        value
            .validate()
            .map_err(|e| AppError::Validation(format_validation_errors(&e)))?;

        Ok(ValidatedQuery(value))
    }
}

fn json_rejection_to_app_error(err: JsonRejection) -> AppError {
    let message = err.body_text();
    match err {
        JsonRejection::JsonDataError(_) => AppError::Validation(message),
        JsonRejection::JsonSyntaxError(_)
        | JsonRejection::MissingJsonContentType(_)
        | JsonRejection::BytesRejection(_) => AppError::BadRequest(message),
        _ => AppError::BadRequest(message),
    }
}

fn query_rejection_to_app_error(err: QueryRejection) -> AppError {
    AppError::Validation(err.body_text())
}

fn format_validation_errors(errors: &validator::ValidationErrors) -> String {
    errors
        .field_errors()
        .iter()
        .flat_map(|(field, errs)| {
            errs.iter().map(move |err| {
                let msg = err
                    .message
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| err.code.to_string());
                format!("{field}: {msg}")
            })
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Reject empty / whitespace-only strings (handy outside derive attributes).
pub fn validate_not_empty(field: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        Err(AppError::Validation(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

/// Validate a Stellar address (G… / C… / etc.) via `stellar-strkey`.
pub fn validate_stellar_address(field: &str, value: &str) -> Result<(), AppError> {
    validate_not_empty(field, value)?;
    match stellar_strkey::Strkey::from_string(value) {
        Ok(_) => Ok(()),
        Err(_) => Err(AppError::Validation(format!(
            "{field} is not a valid Stellar address"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::post,
        Router,
    };
    use serde::Deserialize;
    use tower::ServiceExt;
    use validator::Validate;

    #[derive(Debug, Deserialize, Validate)]
    #[serde(deny_unknown_fields)]
    struct SampleRequest {
        #[validate(length(min = 1, message = "name must not be empty"))]
        name: String,
    }

    async fn echo(ValidatedJson(body): ValidatedJson<SampleRequest>) -> impl IntoResponse {
        body.name
    }

    fn test_router() -> Router {
        Router::new().route("/echo", post(echo))
    }

    #[tokio::test]
    async fn valid_json_passes_validation() {
        let app = test_router();
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"perigee"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_json_returns_bad_request_envelope() {
        let app = test_router();
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/json")
            .body(Body::from("{not-json"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "BAD_REQUEST");
        assert!(body["message"].as_str().unwrap().contains("Failed to parse"));
    }

    #[tokio::test]
    async fn unknown_field_returns_validation_envelope() {
        let app = test_router();
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"ok","extra":true}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn empty_field_returns_validation_envelope() {
        let app = test_router();
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":""}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "VALIDATION_ERROR");
        assert!(body["message"].as_str().unwrap().contains("name"));
    }

    #[test]
    fn stellar_address_helper_rejects_invalid() {
        assert!(validate_stellar_address("account", "").is_err());
        assert!(validate_stellar_address("account", "not-a-key").is_err());
    }
}
