//! Authenticated read/update of the caller's notification preferences (API-30).
//!
//! The user id is always the authenticated wallet address, so a caller can
//! only ever read or change their own preferences. The rules (categories,
//! defaults, mandatory notices) live in [`crate::notifications`]; see
//! `docs/NOTIFICATIONS.md`.

use crate::auth::AuthenticatedUser;
use crate::notifications::{NotificationService, PreferenceError, PreferencesUpdate};
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use axum::{
    extract::rejection::JsonRejection,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::sync::Arc;

fn problem(status: StatusCode, detail: &str) -> Response {
    let body = ProblemDetails::from_status(status.as_u16(), detail)
        .with_instance("/notifications/preferences")
        .to_json();
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

fn error_response(error: PreferenceError) -> Response {
    let status = match error {
        PreferenceError::UnsupportedCategory(_)
        | PreferenceError::MandatoryCategory(_)
        | PreferenceError::Empty => StatusCode::UNPROCESSABLE_ENTITY,
        PreferenceError::StatePoisoned => StatusCode::SERVICE_UNAVAILABLE,
    };
    problem(status, &error.to_string())
}

/// `GET /notifications/preferences`
pub async fn get_notification_preferences_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(service): Extension<Arc<NotificationService>>,
) -> Response {
    match service.preferences(&user.stellar_address) {
        Ok(view) => Json(view).into_response(),
        Err(error) => error_response(error),
    }
}

/// `PUT /notifications/preferences` – partial update, applied atomically.
pub async fn update_notification_preferences_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(service): Extension<Arc<NotificationService>>,
    body: Result<Json<PreferencesUpdate>, JsonRejection>,
) -> Response {
    let Json(update) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("invalid preferences update: {}", rejection.body_text()),
            )
        }
    };
    match service.update_preferences(&user.stellar_address, &update) {
        Ok(view) => Json(view).into_response(),
        Err(error) => error_response(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use crate::notifications::InMemoryQueue;
    use axum::{body::Body, http::Request, routing::get, Router};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn app(service: Arc<NotificationService>, user: &str) -> Router {
        Router::new()
            .route(
                "/notifications/preferences",
                get(get_notification_preferences_handler)
                    .put(update_notification_preferences_handler),
            )
            .layer(Extension(service))
            .layer(Extension(AuthenticatedUser::new(
                user.into(),
                Role::Manager,
                vec![],
            )))
    }

    async fn call(router: Router, method: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri("/notifications/preferences")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn category<'a>(view: &'a Value, name: &str) -> &'a Value {
        view["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["category"] == name)
            .unwrap()
    }

    #[tokio::test]
    async fn users_read_and_update_only_their_own_preferences() {
        let service = Arc::new(NotificationService::new(
            b"secret",
            Arc::new(InMemoryQueue::default()),
        ));

        let (status, view) = call(app(service.clone(), "GALICE"), "GET", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(category(&view, "vault_activity")["channels"]["sms"], true);
        assert_eq!(category(&view, "security")["mandatory"], true);

        let (status, view) = call(
            app(service.clone(), "GALICE"),
            "PUT",
            Some(json!({"categories": {"vault_activity": {"sms": false}}})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(category(&view, "vault_activity")["channels"]["sms"], false);

        let (_, bob) = call(app(service.clone(), "GBOB"), "GET", None).await;
        assert_eq!(category(&bob, "vault_activity")["channels"]["sms"], true);

        let (status, body) = call(
            app(service.clone(), "GALICE"),
            "PUT",
            Some(json!({"categories": {"account": {"email": false}}})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["detail"].as_str().unwrap().contains("mandatory"));

        let (status, _) = call(
            app(service.clone(), "GALICE"),
            "PUT",
            Some(json!({"categories": {"crypto_tips": {"email": true}}})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let (status, _) = call(
            app(service, "GALICE"),
            "PUT",
            Some(json!({"categories": {"reports": {"fax": true}}})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
