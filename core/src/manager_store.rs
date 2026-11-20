use crate::auth::AuthenticatedUser;
use crate::auth::Role;
use crate::error_codes::ErrorCode;
use crate::errors::{ApiJson, AppError};
use crate::errors::AppError;
use crate::input_sanitization::{SanitizedJson, SanitizedPath, SanitizedQuery};
use crate::db;
use axum::{
    extract::{Extension, Path, State},
    extract::State,
    Json,
};
use chrono::Utc;
use std::sync::Arc;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum ManagerStoreError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Manager not found: {0}")]
    NotFound(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Stellar address already registered: {0}")]
    DuplicateAddress(String),
    #[error("Invalid manager state transition: {0}")]
    InvalidTransition(String),
    #[error("Unauthorized manager transition: {0}")]
    UnauthorizedTransition(String),
}

impl From<ManagerStoreError> for AppError {
    fn from(err: ManagerStoreError) -> Self {
        match err {
            ManagerStoreError::NotFound(msg) => {
                AppError::with_code(ErrorCode::ManagerNotFound, msg)
            }
            ManagerStoreError::InvalidData(msg) => {
                AppError::with_code(ErrorCode::InvalidInput, msg)
            }
            ManagerStoreError::DuplicateAddress(msg) => {
                AppError::with_code(ErrorCode::ManagerAlreadyExists, msg)
            }
            ManagerStoreError::InvalidTransition(msg) => {
                AppError::with_code(ErrorCode::StateMismatch, msg)
            }
            ManagerStoreError::UnauthorizedTransition(msg) => {
                AppError::Forbidden(msg)
            }
            ManagerStoreError::Database(e) => {
                AppError::with_code(ErrorCode::DatabaseError, e.to_string())
            }
        }
    }
}

/// Manager DTOs live in [`crate::db::models`]; the typed DB layer returns them
/// directly. They were duplicated here field-for-field, so every method that
/// forwarded a row from the DB returned a type the signature did not accept.
pub use crate::db::models::{
    ApproveManagerRequest, ManagerRecord, ManagerStatusResponse, RegisterManagerRequest,
};

pub struct ManagerStore {
    managers: db::schema::ManagersTable,
}

impl ManagerStore {
    pub fn new(managers: db::schema::ManagersTable) -> Self {
        Self { managers }
    }

    pub async fn register(&self, req: &RegisterManagerRequest) -> Result<ManagerRecord, ManagerStoreError> {
        if req.stellar_address.trim().is_empty() {
            return Err(ManagerStoreError::InvalidData(
                "stellar_address must not be empty".into(),
            ));
        }
        if req.name.trim().is_empty() {
            return Err(ManagerStoreError::InvalidData(
                "name must not be empty".into(),
            ));
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let stellar = req.stellar_address.trim();
        let name = req.name.trim();
        let email = req.email.trim();
        let kyc = req.kyc_document_ref.trim();

        self.managers
            .insert(&id, stellar, name, email, kyc, now)
            .await
            .map_err(|e| match e {
                sqlx::Error::Database(db_err) => {
                    if db_err.message().contains("UNIQUE") {
                        ManagerStoreError::DuplicateAddress(stellar.to_string())
                    } else {
                        ManagerStoreError::Database(sqlx::Error::Database(db_err))
                    }
                }
                other => ManagerStoreError::Database(other),
            })
    }

    pub async fn get(&self, id: &str) -> Result<ManagerRecord, ManagerStoreError> {
        self.managers
            .find_by_id(id)
            .await
            .map_err(ManagerStoreError::Database)?
            .ok_or_else(|| ManagerStoreError::NotFound(id.to_string()))
    }

    pub async fn find_by_stellar_address(&self, address: &str) -> Result<Option<ManagerRecord>, ManagerStoreError> {
        self.managers
            .find_by_stellar_address(address)
            .await
            .map_err(ManagerStoreError::Database)
    }

    pub async fn is_approved(&self, stellar_address: &str) -> Result<bool, ManagerStoreError> {
        let record = self.find_by_stellar_address(stellar_address).await?;
        match record {
            Some(r) => Ok(r.status == "approved"),
            None => Ok(false),
        }
    }

    pub async fn list(
        &self,
        status_filter: Option<&str>,
        pagination: &crate::db::models::PaginationParams,
    ) -> Result<crate::db::models::PagedResponse<ManagerRecord>, ManagerStoreError> {
        let (limit, offset) = pagination.to_limit_offset();
        let (data, total_count) = self
            .managers
            .list_paginated(status_filter, limit, offset)
            .await
            .map_err(ManagerStoreError::Database)?;

        let page = pagination.page.max(1);
        let page_size = pagination.page_size.clamp(1, crate::db::models::PaginationParams::MAX_PAGE_SIZE);
        let has_more = offset + limit < total_count;

        Ok(crate::db::models::PagedResponse {
            data,
            total_count,
            page,
            page_size,
            has_more,
        })
    }

    pub async fn approve(
        &self,
        id: &str,
        req: &ApproveManagerRequest,
    ) -> Result<ManagerRecord, ManagerStoreError> {
        self.transition(id, "approved", &Role::Admin, "system", req)
            .await
    }

    pub async fn reject(
        &self,
        id: &str,
        req: &ApproveManagerRequest,
    ) -> Result<ManagerRecord, ManagerStoreError> {
        self.transition(id, "rejected", &Role::Admin, "system", req)
            .await
    }

    pub async fn transition(
        &self,
        id: &str,
        target_status: &str,
        actor_role: &Role,
        actor: &str,
        req: &ApproveManagerRequest,
    ) -> Result<ManagerRecord, ManagerStoreError> {
        if *actor_role != Role::Admin {
            return Err(ManagerStoreError::UnauthorizedTransition(
                "Only administrators may approve or reject manager registrations".into(),
            ));
        }
        if target_status != "approved" && target_status != "rejected" {
            return Err(ManagerStoreError::InvalidTransition(format!(
                "Unknown target status '{target_status}'"
            )));
        }

        let reason = if !req.reason.trim().is_empty() {
            req.reason.trim()
        } else {
            req.notes.trim()
        };
        if reason.chars().count() > 500 {
            return Err(ManagerStoreError::InvalidData(
                "administrative reason must be at most 500 characters".into(),
            ));
        }

        let current = self.get(id).await?;
        if current.status != "pending" {
            return Err(ManagerStoreError::InvalidTransition(format!(
                "Cannot transition manager from '{}' to '{}'",
                current.status, target_status
            )));
        }

        let result = self.managers
            .update_status(id, "pending", target_status, reason, Utc::now())
            .await
            .map_err(ManagerStoreError::Database);

        match result {
            Ok(record) => {
                let action = if target_status == "approved" {
                    "manager.approved"
                } else {
                    "manager.rejected"
                };
                crate::audit_log::log_audit_event_with_reason(
                    id,
                    action,
                    actor,
                    (!reason.is_empty()).then_some(reason),
                );
                Ok(record)
            }
            Err(ManagerStoreError::Database(sqlx::Error::RowNotFound)) => {
                Err(ManagerStoreError::InvalidTransition(format!(
                    "Cannot transition manager from '{}' to '{}'",
                    current.status, target_status
                )))
            }
            Err(error) => Err(error),
        }
    }
}

fn require_admin(user: &AuthenticatedUser) -> Result<(), AppError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Administrator role is required".to_string(),
        ))
    }
}

#[utoipa::path(
    post,
    path = "/managers/register",
    request_body = RegisterManagerRequest,
    responses(
        (status = 200, description = "Manager registered (pending approval)", body = ManagerRecord),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Stellar address already registered")
    ),
    tag = "Managers"
)]
pub async fn register_manager_handler(
    State(state): State<Arc<crate::AppState>>,
    ApiJson(payload): ApiJson<RegisterManagerRequest>,
    SanitizedJson(payload): SanitizedJson<RegisterManagerRequest>,
) -> Result<Json<ManagerRecord>, AppError> {
    let manager = state.manager_store.register(&payload).await?;
    Ok(Json(manager))
}

#[utoipa::path(
    get,
    path = "/managers",
    params(
        ("status" = Option<String>, Query, description = "Filter by status: pending, approved, rejected"),
        ("page" = Option<u32>, Query, description = "Page number (1-indexed, default 1)"),
        ("page_size" = Option<u32>, Query, description = "Records per page (default 50, max 200)")
    ),
    responses(
        (status = 200, description = "Paginated list of manager records", body = PagedResponse<ManagerRecord>)
    ),
    tag = "Managers"
)]
pub async fn list_managers_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    SanitizedQuery(params): SanitizedQuery<std::collections::HashMap<String, String>>,
) -> Result<Json<crate::db::models::PagedResponse<ManagerRecord>>, AppError> {
    require_admin(&user)?;
    let status_filter = params.get("status").map(|s| s.as_str());

    let page: u32 = params
        .get("page")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let page_size: u32 = params
        .get("page_size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);

    let pagination = crate::db::models::PaginationParams { page, page_size };
    let result = state.manager_store.list(status_filter, &pagination).await?;
    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/managers/{id}",
    params(("id" = String, Path, description = "Manager ID")),
    responses(
        (status = 200, description = "Manager record", body = ManagerRecord),
        (status = 404, description = "Manager not found")
    ),
    tag = "Managers"
)]
pub async fn get_manager_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    SanitizedPath(id): SanitizedPath<String>,
) -> Result<Json<ManagerRecord>, AppError> {
    require_admin(&user)?;
    let manager = state.manager_store.get(&id).await?;
    Ok(Json(manager))
}

#[utoipa::path(
    post,
    path = "/managers/{id}/approve",
    params(("id" = String, Path, description = "Manager ID")),
    request_body = ApproveManagerRequest,
    responses(
        (status = 200, description = "Manager approved", body = ManagerRecord),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Administrator role required"),
        (status = 404, description = "Manager not found"),
        (status = 409, description = "Invalid state transition")
    ),
    tag = "Managers"
)]
pub async fn approve_manager_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<ApproveManagerRequest>,
    SanitizedPath(id): SanitizedPath<String>,
    SanitizedJson(payload): SanitizedJson<ApproveManagerRequest>,
) -> Result<Json<ManagerRecord>, AppError> {
    require_admin(&user)?;
    let manager = state
        .manager_store
        .transition(&id, "approved", &Role::Admin, &user.stellar_address, &payload)
        .await?;
    Ok(Json(manager))
}

#[utoipa::path(
    post,
    path = "/managers/{id}/reject",
    params(("id" = String, Path, description = "Manager ID")),
    request_body = ApproveManagerRequest,
    responses(
        (status = 200, description = "Manager rejected", body = ManagerRecord),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Administrator role required"),
        (status = 404, description = "Manager not found"),
        (status = 409, description = "Invalid state transition")
    ),
    tag = "Managers"
)]
pub async fn reject_manager_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<ApproveManagerRequest>,
    SanitizedPath(id): SanitizedPath<String>,
    SanitizedJson(payload): SanitizedJson<ApproveManagerRequest>,
) -> Result<Json<ManagerRecord>, AppError> {
    require_admin(&user)?;
    let manager = state
        .manager_store
        .transition(&id, "rejected", &Role::Admin, &user.stellar_address, &payload)
        .await?;
    Ok(Json(manager))
}

#[utoipa::path(
    get,
    path = "/managers/{id}/audit",
    params(("id" = String, Path, description = "Manager ID")),
    responses(
        (status = 200, description = "Tamper-evident administrative transition audit entries", body = Vec<crate::audit_log::AuditEvent>),
        (status = 403, description = "Administrator role required"),
        (status = 404, description = "Manager not found")
    ),
    tag = "Managers"
)]
pub async fn manager_audit_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    SanitizedPath(id): SanitizedPath<String>,
) -> Result<Json<Vec<crate::audit_log::AuditEvent>>, AppError> {
    require_admin(&user)?;
    state.manager_store.get(&id).await?;
    Ok(Json(crate::audit_log::global_entries_for_manager(&id)))
}

#[utoipa::path(
    get,
    path = "/managers/status/{stellar_address}",
    params(("stellar_address" = String, Path, description = "Stellar address")),
    responses(
        (status = 200, description = "Manager approval status", body = ManagerStatusResponse)
    ),
    tag = "Managers"
)]
pub async fn check_manager_status_handler(
    State(state): State<Arc<crate::AppState>>,
    SanitizedPath(stellar_address): SanitizedPath<String>,
) -> Result<Json<ManagerStatusResponse>, AppError> {
    let record = state
        .manager_store
        .find_by_stellar_address(&stellar_address)
        .await?;
    match record {
        Some(m) => {
            // Derive the message before moving `status` into the response.
            let message: String = match m.status.as_str() {
                "approved" => "Manager is approved and active".into(),
                "rejected" => "Manager registration was rejected".into(),
                _ => "Manager registration is pending approval".into(),
            };

            Ok(Json(ManagerStatusResponse {
                id: m.id,
                status: m.status,
                message,
            }))
        }
        None => Ok(Json(ManagerStatusResponse {
            id: String::new(),
            status: "unregistered".into(),
            message: "No manager registration found for this address".into(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_store() -> ManagerStore {
        let url = "sqlite::memory:".to_string();
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS managers (
                id TEXT PRIMARY KEY,
                stellar_address TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                email TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'approved', 'rejected')),
                kyc_document_ref TEXT NOT NULL DEFAULT '',
                notes TEXT NOT NULL DEFAULT '',
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        ManagerStore::new(
            db::schema::TypedSchema::new(std::sync::Arc::new(db::MonitoredPool::from_inner(pool)))
                .managers(),
        )
    }

    #[tokio::test]
    async fn register_and_get() {
        let store = test_store().await;
        let created = store
            .register(&RegisterManagerRequest {
                stellar_address: "GABC123".into(),
                name: "Alice".into(),
                email: "alice@example.com".into(),
                kyc_document_ref: "doc-123".into(),
            })
            .await
            .unwrap();

        assert_eq!(created.status, "pending");
        assert_eq!(created.name, "Alice");

        let fetched = store.get(&created.id).await.unwrap();
        assert_eq!(fetched.stellar_address, "GABC123");
    }

    #[tokio::test]
    async fn duplicate_address_rejected() {
        let store = test_store().await;
        store
            .register(&RegisterManagerRequest {
                stellar_address: "GABC123".into(),
                name: "Alice".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        let err = store
            .register(&RegisterManagerRequest {
                stellar_address: "GABC123".into(),
                name: "Bob".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap_err();

        match err {
            ManagerStoreError::DuplicateAddress(addr) => assert_eq!(addr, "GABC123"),
            other => panic!("expected DuplicateAddress, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn approve_pending_manager() {
        let store = test_store().await;
        let created = store
            .register(&RegisterManagerRequest {
                stellar_address: "GABC123".into(),
                name: "Alice".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        let approved = store
            .approve(&created.id, &ApproveManagerRequest { notes: "KYC verified".into(), reason: "".into() })
            .await
            .unwrap();
        assert_eq!(approved.status, "approved");
        assert_eq!(approved.notes, "KYC verified");

        assert!(store.is_approved("GABC123").await.unwrap());
    }

    #[tokio::test]
    async fn reject_pending_manager() {
        let store = test_store().await;
        let created = store
            .register(&RegisterManagerRequest {
                stellar_address: "GABC123".into(),
                name: "Alice".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        let rejected = store
            .reject(&created.id, &ApproveManagerRequest { notes: "Failed KYC".into(), reason: "".into() })
            .await
            .unwrap();
        assert_eq!(rejected.status, "rejected");

        assert!(!store.is_approved("GABC123").await.unwrap());
    }

    #[tokio::test]
    async fn rejects_invalid_manager_transitions() {
        let store = test_store().await;
        let created = store
            .register(&RegisterManagerRequest {
                stellar_address: "GTRANSITION".into(),
                name: "Transition".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        store
            .approve(&created.id, &ApproveManagerRequest {
                notes: "".into(),
                reason: "KYC complete".into(),
            })
            .await
            .unwrap();

        let err = store
            .reject(&created.id, &ApproveManagerRequest {
                notes: "".into(),
                reason: "Should not change approved status".into(),
            })
            .await
            .unwrap_err();

        assert!(matches!(err, ManagerStoreError::InvalidTransition(_)));
    }

    #[tokio::test]
    async fn rejects_non_admin_transitions_and_records_admin_audit() {
        let store = test_store().await;
        let created = store
            .register(&RegisterManagerRequest {
                stellar_address: "GAUDIT".into(),
                name: "Audit".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        let unauthorized = store
            .transition(
                &created.id,
                "approved",
                &Role::Manager,
                "GMANAGER",
                &ApproveManagerRequest {
                    notes: "".into(),
                    reason: "Not allowed".into(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(unauthorized, ManagerStoreError::UnauthorizedTransition(_)));

        store
            .transition(
                &created.id,
                "approved",
                &Role::Admin,
                "GADMIN",
                &ApproveManagerRequest {
                    notes: "".into(),
                    reason: "Identity documents verified".into(),
                },
            )
            .await
            .unwrap();

        let entries = crate::audit_log::global_entries_for_manager(&created.id);
        let entry = entries.last().expect("approval audit entry");
        assert_eq!(entry.action, "manager.approved");
        assert_eq!(entry.actor, "GADMIN");
        assert_eq!(entry.reason.as_deref(), Some("Identity documents verified"));
        assert!(crate::audit_log::verify_global_chain().is_ok());
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let store = test_store().await;
        store
            .register(&RegisterManagerRequest {
                stellar_address: "GA1".into(),
                name: "A".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();
        let m2 = store
            .register(&RegisterManagerRequest {
                stellar_address: "GA2".into(),
                name: "B".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();
        store
            .approve(&m2.id, &ApproveManagerRequest { notes: "".into(), reason: "".into() })
            .await
            .unwrap();

        let all = store.list(None, &crate::db::models::PaginationParams { page: 1, page_size: 50 }).await.unwrap();
        assert_eq!(all.total_count, 2);

        let pending = store.list(Some("pending"), &crate::db::models::PaginationParams { page: 1, page_size: 50 }).await.unwrap();
        assert_eq!(pending.total_count, 1);

        let approved = store.list(Some("approved"), &crate::db::models::PaginationParams { page: 1, page_size: 50 }).await.unwrap();
        assert_eq!(approved.total_count, 1);
    }

    #[tokio::test]
    async fn list_pagination_basic() {
        let store = test_store().await;

        // Insert 5 managers.
        for i in 0..5u32 {
            store
                .register(&RegisterManagerRequest {
                    stellar_address: format!("GA{i}"),
                    name: format!("Manager {i}"),
                    email: "".into(),
                    kyc_document_ref: "".into(),
                })
                .await
                .unwrap();
        }

        let page1 = store
            .list(None, &crate::db::models::PaginationParams { page: 1, page_size: 2 })
            .await
            .unwrap();
        assert_eq!(page1.data.len(), 2);
        assert_eq!(page1.total_count, 5);
        assert_eq!(page1.page, 1);
        assert_eq!(page1.page_size, 2);
        assert!(page1.has_more);

        let page2 = store
            .list(None, &crate::db::models::PaginationParams { page: 2, page_size: 2 })
            .await
            .unwrap();
        assert_eq!(page2.data.len(), 2);
        assert!(page2.has_more);

        let page3 = store
            .list(None, &crate::db::models::PaginationParams { page: 3, page_size: 2 })
            .await
            .unwrap();
        assert_eq!(page3.data.len(), 1);
        assert!(!page3.has_more);
    }

    #[tokio::test]
    async fn list_pagination_page_size_clamped_to_max() {
        let store = test_store().await;
        store
            .register(&RegisterManagerRequest {
                stellar_address: "GA1".into(),
                name: "A".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        // page_size of 9999 must be silently clamped to MAX (200).
        let result = store
            .list(None, &crate::db::models::PaginationParams { page: 1, page_size: 9999 })
            .await
            .unwrap();
        assert_eq!(result.page_size, 200);
        assert_eq!(result.total_count, 1);
    }

    #[tokio::test]
    async fn list_empty_page_beyond_end() {
        let store = test_store().await;
        store
            .register(&RegisterManagerRequest {
                stellar_address: "GA1".into(),
                name: "A".into(),
                email: "".into(),
                kyc_document_ref: "".into(),
            })
            .await
            .unwrap();

        let result = store
            .list(None, &crate::db::models::PaginationParams { page: 10, page_size: 50 })
            .await
            .unwrap();
        assert!(result.data.is_empty());
        assert_eq!(result.total_count, 1);
        assert!(!result.has_more);
    }
}
