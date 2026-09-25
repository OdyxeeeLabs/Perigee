//! White-label vault records with optimistic locking (API-37).
//!
//! Concurrent updates to the same vault must supply the current `version`.
//! The store bumps `version` only when `WHERE id = ? AND version = ?` matches;
//! otherwise the caller gets a conflict and must reload.

use crate::auth::AuthenticatedUser;
use crate::db;
use crate::errors::AppError;
use crate::input_sanitization::{SanitizedJson, SanitizedPath, SanitizedQuery};
use axum::{
    extract::{Extension, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum VaultStoreError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Vault not found: {0}")]
    NotFound(String),

    #[error("Version conflict for vault {vault_id}: expected {expected_version}")]
    Conflict {
        vault_id: String,
        expected_version: i64,
    },

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

impl From<crate::policy_expiry::PolicyExpiryError> for AppError {
    fn from(err: crate::policy_expiry::PolicyExpiryError) -> Self {
        use crate::policy_expiry::PolicyExpiryError;

        match err {
            PolicyExpiryError::PolicyExpired { .. } => AppError::PolicyExpired(err.to_string()),

            // A policy nobody can parse cannot authorise anything, but it is a
            // configuration fault rather than a lapsed authority, so it reads
            // as a bad request rather than a 403.
            PolicyExpiryError::MalformedPolicy(_) | PolicyExpiryError::InvalidExpiry(_) => {
                AppError::BadRequest(err.to_string())
            }
        }
    }
}

impl From<VaultStoreError> for AppError {
    fn from(err: VaultStoreError) -> Self {
        match err {
            VaultStoreError::NotFound(msg) => AppError::NotFound(msg),
            VaultStoreError::Conflict {
                vault_id,
                expected_version,
            } => AppError::Conflict(format!(
                "Vault '{vault_id}' was updated by another request (expected version {expected_version}); reload and retry"
            )),
            VaultStoreError::InvalidData(msg) => AppError::BadRequest(msg),
            VaultStoreError::Database(e) => AppError::Internal(e.to_string()),
        }
    }
}

/// Vault DTOs live in [`crate::db::models`]: the typed DB layer returns them
/// directly, so re-exporting is what keeps this store's signatures compatible
/// with what the DB hands back. They were previously duplicated here
/// field-for-field, which is what made `VaultStore::get` return one type while
/// the DB produced another.
pub use crate::db::models::{CreateVaultRequest, UpdateVaultRequest, VaultRecord};

pub struct VaultStore {
    vaults: db::schema::VaultsTable,
}

impl VaultStore {
    pub fn new(vaults: db::schema::VaultsTable) -> Self {
        Self { vaults }
    }

    pub async fn create(&self, req: &CreateVaultRequest) -> Result<VaultRecord, VaultStoreError> {
        if req.manager_id.trim().is_empty() {
            return Err(VaultStoreError::InvalidData(
                "manager_id must not be empty".into(),
            ));
        }
        if req.name.trim().is_empty() {
            return Err(VaultStoreError::InvalidData(
                "name must not be empty".into(),
            ));
        }

        // Idempotency: if a key is provided and non-empty, return the existing vault
        // for the same (manager_id, idempotency_key) pair.
        let idempotency_key = req
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if let Some(key) = idempotency_key {
            if let Some(vault) = self
                .vaults
                .find_by_idempotency_key(req.manager_id.trim(), key)
                .await
                .map_err(VaultStoreError::Database)?
            {
                return Ok(vault);
            }
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let manager_id = req.manager_id.trim();
        let name = req.name.trim();
        let status = req.status.trim();
        let config_json = req.config_json.trim();

        self.vaults
            .insert(
                &id,
                manager_id,
                name,
                status,
                config_json,
                idempotency_key,
                now,
            )
            .await
            .map_err(VaultStoreError::Database)
    }

    pub async fn list_by_manager(
        &self,
        manager_id: &str,
        pagination: &crate::db::models::PaginationParams,
    ) -> Result<crate::db::models::PagedResponse<VaultRecord>, VaultStoreError> {
        let (limit, offset) = pagination.to_limit_offset();
        let (data, total_count) = self
            .vaults
            .list_by_manager_paginated(manager_id, limit, offset)
            .await
            .map_err(VaultStoreError::Database)?;

        let page = pagination.page.max(1);
        let page_size = pagination
            .page_size
            .clamp(1, crate::db::models::PaginationParams::MAX_PAGE_SIZE);
        let has_more = offset + limit < total_count;

        Ok(crate::db::models::PagedResponse {
            data,
            total_count,
            page,
            page_size,
            has_more,
        })
    }

    pub async fn get(&self, id: &str) -> Result<VaultRecord, VaultStoreError> {
        self.vaults
            .find_by_id(id)
            .await
            .map_err(VaultStoreError::Database)?
            .ok_or_else(|| VaultStoreError::NotFound(id.to_string()))
    }

    /// Apply an update inside a transaction using optimistic locking.
    ///
    /// The typed schema's update method uses `WHERE id = ? AND version = ?`
    /// to guarantee only one concurrent writer succeeds for a given version
    /// snapshot.
    pub async fn update(
        &self,
        id: &str,
        req: &UpdateVaultRequest,
    ) -> Result<VaultRecord, VaultStoreError> {
        if req.version < 1 {
            return Err(VaultStoreError::InvalidData("version must be >= 1".into()));
        }

        let name = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let status = req
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let config_json = req.config_json.as_deref();
        let now = Utc::now();

        let result = self
            .vaults
            .update(id, req.version, name, status, config_json, now)
            .await;

        match result {
            Ok(record) => Ok(record),

            // The typed update matches on `id AND version`, so "no rows" means
            // either the vault is gone or the caller held a stale version.
            // Those are different answers — 404 versus 409 — and the caller
            // needs to know which, so ask the store which one it was.
            Err(sqlx::Error::RowNotFound) => {
                if self.vaults.find_by_id(id).await.ok().flatten().is_some() {
                    Err(VaultStoreError::Conflict {
                        vault_id: id.to_string(),
                        expected_version: req.version,
                    })
                } else {
                    Err(VaultStoreError::NotFound(id.to_string()))
                }
            }

            Err(sqlx::Error::Database(db_err)) => {
                if db_err.message().contains("UNIQUE") {
                    Err(VaultStoreError::InvalidData(
                        "A vault with this idempotency key already exists".into(),
                    ))
                } else {
                    Err(VaultStoreError::Database(sqlx::Error::Database(db_err)))
                }
            }

            Err(other) => Err(VaultStoreError::Database(other)),
        }
    }

    /// Fetch a vault regardless of deletion state. Used by the admin restore
    /// path and by the delete handler (which must check ownership of a vault
    /// that may already be soft-deleted).
    pub async fn get_including_deleted(&self, id: &str) -> Result<VaultRecord, VaultStoreError> {
        self.vaults
            .find_by_id_including_deleted(id)
            .await
            .map_err(VaultStoreError::Database)?
            .ok_or_else(|| VaultStoreError::NotFound(id.to_string()))
    }

    /// Soft-delete a vault: stamp `deleted_at` without removing the row
    /// (BE-044 / issue #281). Idempotent for an already-deleted vault
    /// (`NotFound` if it does not exist).
    pub async fn soft_delete(&self, id: &str) -> Result<VaultRecord, VaultStoreError> {
        let now = Utc::now();
        let affected = self
            .vaults
            .soft_delete(id, now)
            .await
            .map_err(VaultStoreError::Database)?;
        if affected == 0 {
            return Err(VaultStoreError::NotFound(id.to_string()));
        }
        self.get_including_deleted(id).await
    }

    /// Restore a soft-deleted vault by clearing `deleted_at` (BE-044 / #281).
    pub async fn restore(&self, id: &str) -> Result<VaultRecord, VaultStoreError> {
        let now = Utc::now();
        let affected = self
            .vaults
            .restore(id, now)
            .await
            .map_err(VaultStoreError::Database)?;
        if affected == 0 {
            return Err(VaultStoreError::NotFound(id.to_string()));
        }
        self.get_including_deleted(id).await
    }
}

/// Verify that `manager_id` (UUID) belongs to the authenticated Stellar address.
async fn verify_ownership(
    state: &crate::AppState,
    user: &AuthenticatedUser,
    manager_id: &str,
) -> Result<(), AppError> {
    if user.is_admin() {
        return Ok(());
    }

    if (user.role == crate::auth::Role::Operator || user.role == crate::auth::Role::Viewer)
        && !user.vault_scopes.is_empty()
    {
        return Ok(());
    }

    let manager = state
        .manager_store
        .find_by_stellar_address(&user.stellar_address)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| {
            crate::audit_log::log_security_event(
                crate::audit_log::SecurityEventType::VaultAccessDenied,
                Some(&user.stellar_address),
                None,
                None,
                Some("Manager not found for authenticated user"),
            );
            AppError::Unauthorized("Manager not found for authenticated user".into())
        })?;
    if manager.id != manager_id {
        crate::audit_log::log_security_event(
            crate::audit_log::SecurityEventType::VaultAccessDenied,
            Some(&user.stellar_address),
            None,
            None,
            Some("Agent not authorized for vault: manager mismatch"),
        );
        return Err(AppError::Unauthorized(
            "You can only access vaults belonging to your own manager account".into(),
        ));
    }
    Ok(())
}

fn list_vaults_default_page() -> u32 {
    1
}
fn list_vaults_default_page_size() -> u32 {
    50
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListVaultsQuery {
    pub manager_id: String,
    #[serde(default = "list_vaults_default_page")]
    pub page: u32,
    #[serde(default = "list_vaults_default_page_size")]
    pub page_size: u32,
}

// ── HTTP handlers ─────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/vaults",
    params(
        ("manager_id" = String, Query, description = "Manager ID to list vaults for"),
        ("page" = Option<u32>, Query, description = "Page number (1-indexed, default 1)"),
        ("page_size" = Option<u32>, Query, description = "Records per page (default 50, max 200)")
    ),
    responses(
        (status = 200, description = "Paginated list of vaults for the manager", body = PagedResponse<VaultRecord>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn list_vaults_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedQuery(query): SanitizedQuery<ListVaultsQuery>,
) -> Result<Json<crate::db::models::PagedResponse<VaultRecord>>, AppError> {
    user.authorize_vault_read("*")?;
    verify_ownership(&state, &user, &query.manager_id).await?;
    let pagination = crate::db::models::PaginationParams {
        page: query.page,
        page_size: query.page_size,
    };
    let mut result = state
        .vault_store
        .list_by_manager(&query.manager_id, &pagination)
        .await?;
    if !user.can_access_all_vaults() {
        result.data.retain(|v| user.can_access_vault(&v.id));
        result.total_count = result.data.len() as i64;
    }
    Ok(Json(result))
}

#[utoipa::path(
    post,
    path = "/vaults",
    request_body = CreateVaultRequest,
    responses(
        (status = 200, description = "Vault created", body = VaultRecord),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Insufficient privileges or vault-scoped restriction")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn create_vault_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedJson(payload): SanitizedJson<CreateVaultRequest>,
) -> Result<Json<VaultRecord>, AppError> {
    if !user.can_manage_vaults() {
        crate::audit_log::log_security_event(
            crate::audit_log::SecurityEventType::VaultAccessDenied,
            Some(&user.stellar_address),
            None,
            None,
            Some("Role not authorized to create vaults"),
        );
        return Err(AppError::Forbidden(
            format!("Role '{}' is not authorized to create vaults", user.role)
        ));
    }
    if !user.can_access_all_vaults() {
        return Err(AppError::Forbidden(
            "Vault-scoped token is not authorized to create new vaults".into(),
        ));
    }
    verify_ownership(&state, &user, &payload.manager_id).await?;
    let approved = state
        .manager_store
        .is_approved(&user.stellar_address)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !approved && !user.is_admin() {
        crate::audit_log::log_security_event(
            crate::audit_log::SecurityEventType::VaultAccessDenied,
            Some(&user.stellar_address),
            None,
            None,
            Some("Manager is not approved to create vaults"),
        );
        return Err(AppError::BadRequest(
            "Manager is not approved. Only approved managers may create vaults. Register via /managers/register and wait for approval.".into(),
        ));
    }
    let vault = state.vault_store.create(&payload).await?;
    crate::audit_log::log_audit_event(
        &payload.manager_id,
        "vault_provisioning",
        &payload.manager_id,
    );
    Ok(Json(vault))
}

#[utoipa::path(
    get,
    path = "/vaults/{id}",
    params(("id" = String, Path, description = "Vault ID")),
    responses(
        (status = 200, description = "Vault record", body = VaultRecord),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Vault scope not authorized"),
        (status = 404, description = "Vault not found")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn get_vault_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedPath(id): SanitizedPath<String>,
) -> Result<Json<VaultRecord>, AppError> {
    user.authorize_vault_read(&id)?;
    let vault = state.vault_store.get(&id).await?;
    verify_ownership(&state, &user, &vault.manager_id).await?;
    Ok(Json(vault))
}

#[utoipa::path(
    patch,
    path = "/vaults/{id}",
    params(("id" = String, Path, description = "Vault ID")),
    request_body = UpdateVaultRequest,
    responses(
        (status = 200, description = "Vault updated (version bumped)", body = VaultRecord),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Insufficient privileges or vault scope not authorized"),
        (status = 404, description = "Vault not found"),
        (status = 409, description = "Optimistic lock conflict — reload and retry")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn update_vault_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedPath(id): SanitizedPath<String>,
    SanitizedJson(payload): SanitizedJson<UpdateVaultRequest>,
) -> Result<Json<VaultRecord>, AppError> {
    user.authorize_vault_write(&id)?;
    let vault = state.vault_store.get(&id).await?;
    verify_ownership(&state, &user, &vault.manager_id).await?;

    // BE-023: an expired policy authorises nothing. Checked against the
    // *stored* policy, before the update is applied — otherwise a caller could
    // use an expired policy to extend its own expiry.
    crate::policy_expiry::ensure_policy_active(&vault.config_json, chrono::Utc::now())?;

    let vault = state.vault_store.update(&id, &payload).await?;
    if payload.config_json.is_some() {
        crate::audit_log::log_audit_event(&vault.manager_id, "fee_split_change", &vault.manager_id);
    } else {
        crate::audit_log::log_audit_event(&vault.manager_id, "vault_update", &vault.manager_id);
    }
    Ok(Json(vault))
}

/// Gate a handler on admin privileges.
///
/// Admin authority is determined by the `Admin` role in JWT claims or the
/// `PERIGEE_ADMIN_STELLAR_ADDRESSES` environment variable (comma-separated).
/// Requests without admin authority are rejected with `403 Forbidden`.
fn require_admin(user: &AuthenticatedUser) -> Result<(), AppError> {
    if user.is_admin() {
        Ok(())
    } else {
        crate::audit_log::log_security_event(
            crate::audit_log::SecurityEventType::UnauthorizedAccess,
            Some(&user.stellar_address),
            None,
            None,
            Some("Admin privileges required but caller is not admin"),
        );
        Err(AppError::Forbidden(
            "Admin privileges required to perform this action".into(),
        ))
    }
}

#[utoipa::path(
    delete,
    path = "/vaults/{id}",
    params(("id" = String, Path, description = "Vault ID")),
    responses(
        (status = 200, description = "Vault soft-deleted", body = VaultRecord),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Insufficient privileges or vault scope not authorized"),
        (status = 404, description = "Vault not found")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn soft_delete_vault_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedPath(id): SanitizedPath<String>,
) -> Result<Json<VaultRecord>, AppError> {
    user.authorize_vault_manage(&id)?;
    // Ownership is checked against the (possibly deleted) vault so the owner can
    // still delete their own vault.
    let vault = state.vault_store.get_including_deleted(&id).await?;
    verify_ownership(&state, &user, &vault.manager_id).await?;
    let vault = state.vault_store.soft_delete(&id).await?;
    crate::audit_log::log_audit_event(&vault.manager_id, "vault_soft_delete", &vault.manager_id);
    Ok(Json(vault))
}

#[utoipa::path(
    post,
    path = "/vaults/{id}/restore",
    params(("id" = String, Path, description = "Vault ID")),
    responses(
        (status = 200, description = "Vault restored", body = VaultRecord),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin privileges required"),
        (status = 404, description = "Vault not found")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn restore_vault_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedPath(id): SanitizedPath<String>,
) -> Result<Json<VaultRecord>, AppError> {
    require_admin(&user)?;
    let vault = state.vault_store.restore(&id).await?;
    crate::audit_log::log_audit_event(&vault.manager_id, "vault_restore", &vault.manager_id);
    Ok(Json(vault))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListDeletedVaultsQuery {
    /// Optional manager to scope the listing to.
    pub manager_id: Option<String>,
    #[serde(default = "list_vaults_default_page")]
    pub page: u32,
    #[serde(default = "list_vaults_default_page_size")]
    pub page_size: u32,
}

#[utoipa::path(
    get,
    path = "/admin/vaults/deleted",
    params(
        ("manager_id" = Option<String>, Query, description = "Optional manager ID to scope the listing"),
        ("page" = Option<u32>, Query, description = "Page number (1-indexed, default 1)"),
        ("page_size" = Option<u32>, Query, description = "Records per page (default 50, max 200)")
    ),
    responses(
        (status = 200, description = "Paginated list of soft-deleted vaults", body = PagedResponse<VaultRecord>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Admin privileges required")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Vaults"
)]
pub async fn list_deleted_vaults_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    SanitizedQuery(query): SanitizedQuery<ListDeletedVaultsQuery>,
) -> Result<Json<crate::db::models::PagedResponse<VaultRecord>>, AppError> {
    require_admin(&user)?;
    let pagination = crate::db::models::PaginationParams {
        page: query.page,
        page_size: query.page_size,
    };
    let (limit, offset) = pagination.to_limit_offset();

    let (data, total_count) = match &query.manager_id {
        Some(manager_id) => state
            .vault_store
            .vaults
            .list_deleted_by_manager(manager_id, limit, offset)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?,
        None => state
            .vault_store
            .vaults
            .list_all_deleted(limit, offset)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?,
    };

    let page = pagination.page.max(1);
    let page_size = pagination
        .page_size
        .clamp(1, crate::db::models::PaginationParams::MAX_PAGE_SIZE);
    let has_more = offset + limit < total_count;

    Ok(Json(crate::db::models::PagedResponse {
        data,
        total_count,
        page,
        page_size,
        has_more,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_store() -> VaultStore {
        let db_name = format!("vault_ol_{}", Uuid::new_v4());
        let url = format!("file:{db_name}?mode=memory&cache=shared");
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS vaults (
                id TEXT PRIMARY KEY,
                manager_id TEXT NOT NULL,
                name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                config_json TEXT NOT NULL DEFAULT '{}',
                version INTEGER NOT NULL DEFAULT 1,
                idempotency_key TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                deleted_at TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        VaultStore::new(db::schema::TypedSchema::new(std::sync::Arc::new(pool)).vaults())
    }

    #[tokio::test]
    async fn create_and_get_starts_at_version_one() {
        let store = test_store().await;
        let created = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Alpha".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        assert_eq!(created.version, 1);
        assert_eq!(created.name, "Alpha");

        let fetched = store.get(&created.id).await.unwrap();
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn successful_update_bumps_version() {
        let store = test_store().await;
        let created = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Alpha".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let updated = store
            .update(
                &created.id,
                &UpdateVaultRequest {
                    version: 1,
                    name: Some("Beta".into()),
                    status: None,
                    config_json: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.version, 2);
        assert_eq!(updated.name, "Beta");
    }

    #[tokio::test]
    async fn stale_version_is_rejected() {
        let store = test_store().await;
        let created = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Alpha".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        store
            .update(
                &created.id,
                &UpdateVaultRequest {
                    version: 1,
                    name: Some("First writer".into()),
                    status: None,
                    config_json: None,
                },
            )
            .await
            .unwrap();

        let err = store
            .update(
                &created.id,
                &UpdateVaultRequest {
                    version: 1, // stale
                    name: Some("Second writer".into()),
                    status: None,
                    config_json: None,
                },
            )
            .await
            .unwrap_err();

        match err {
            VaultStoreError::Conflict {
                expected_version, ..
            } => assert_eq!(expected_version, 1),
            other => panic!("expected Conflict, got {other:?}"),
        }

        let current = store.get(&created.id).await.unwrap();
        assert_eq!(current.version, 2);
        assert_eq!(current.name, "First writer");
    }

    #[tokio::test]
    async fn concurrent_updates_only_one_wins() {
        let store = Arc::new(test_store().await);
        let created = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Race".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let id = created.id.clone();
        let a = {
            let store = Arc::clone(&store);
            let id = id.clone();
            tokio::spawn(async move {
                store
                    .update(
                        &id,
                        &UpdateVaultRequest {
                            version: 1,
                            name: Some("Writer-A".into()),
                            status: None,
                            config_json: None,
                        },
                    )
                    .await
            })
        };
        let b = {
            let store = Arc::clone(&store);
            let id = id.clone();
            tokio::spawn(async move {
                store
                    .update(
                        &id,
                        &UpdateVaultRequest {
                            version: 1,
                            name: Some("Writer-B".into()),
                            status: None,
                            config_json: None,
                        },
                    )
                    .await
            })
        };

        let (ra, rb) = tokio::join!(a, b);
        let ra = ra.expect("task A join");
        let rb = rb.expect("task B join");

        let wins = [&ra, &rb].iter().filter(|r| r.is_ok()).count();
        let conflicts = [&ra, &rb]
            .iter()
            .filter(|r| matches!(r, Err(VaultStoreError::Conflict { .. })))
            .count();

        assert!(
            wins >= 1,
            "at least one writer must commit; results={ra:?} {rb:?}"
        );
        assert!(
            wins + conflicts == 2,
            "losers must be conflicts; results={ra:?} {rb:?}"
        );

        let final_vault = store.get(&id).await.unwrap();
        assert_eq!(final_vault.version, 1 + wins as i64);
        assert!(final_vault.name == "Writer-A" || final_vault.name == "Writer-B");
    }

    #[tokio::test]
    async fn idempotency_key_returns_same_vault_on_duplicate() {
        let store = test_store().await;
        let req = CreateVaultRequest {
            manager_id: "mgr-1".into(),
            name: "Alpha".into(),
            status: "active".into(),
            config_json: "{}".into(),
            idempotency_key: Some("key-abc".into()),
        };

        let first = store.create(&req).await.unwrap();
        let second = store.create(&req).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(first.name, second.name);

        // Only one vault should exist for this manager.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vaults WHERE manager_id = ?1")
            .bind("mgr-1")
            .fetch_one(store.vaults.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn different_idempotency_keys_create_separate_vaults() {
        let store = test_store().await;

        let a = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Alpha".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: Some("key-a".into()),
            })
            .await
            .unwrap();

        let b = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Beta".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: Some("key-b".into()),
            })
            .await
            .unwrap();

        assert_ne!(a.id, b.id);
        assert_eq!(a.name, "Alpha");
        assert_eq!(b.name, "Beta");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vaults WHERE manager_id = ?1")
            .bind("mgr-1")
            .fetch_one(store.vaults.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 2);
    }

    #[tokio::test]
    async fn create_without_idempotency_key_allows_duplicates() {
        let store = test_store().await;

        let a = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Alpha".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let b = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Alpha".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        assert_ne!(a.id, b.id);

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vaults WHERE manager_id = ?1")
            .bind("mgr-1")
            .fetch_one(store.vaults.pool())
            .await
            .unwrap();
        assert_eq!(count.0, 2);
    }

    // ── BE-023: policy expiry on vault operations ────────────────────────

    /// Build a policy JSON expiring at `expires_at`.
    fn policy_expiring(expires_at: &str) -> String {
        format!(r#"{{"policy":{{"expires_at":"{expires_at}"}}}}"#)
    }

    async fn vault_with_policy(store: &VaultStore, config_json: &str) -> VaultRecord {
        store
            .create(&CreateVaultRequest {
                manager_id: "mgr-1".into(),
                name: "Policy vault".into(),
                status: "active".into(),
                config_json: config_json.to_string(),
                idempotency_key: None,
            })
            .await
            .unwrap()
    }

    /// The acceptance criterion from BE-023: create a vault, let its policy
    /// expire, attempt an operation, expect rejection.
    #[tokio::test]
    async fn an_expired_policy_blocks_a_vault_operation() {
        let store = test_store().await;
        let vault = vault_with_policy(&store, &policy_expiring("2020-01-01T00:00:00Z")).await;

        let stored = store.get(&vault.id).await.unwrap();

        let err = crate::policy_expiry::ensure_policy_active(&stored.config_json, Utc::now())
            .unwrap_err();

        assert!(matches!(
            err,
            crate::policy_expiry::PolicyExpiryError::PolicyExpired { .. }
        ));

        // And the handler layer turns that into a 403, not a 400 or a 500.
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::PolicyExpired(_)));
    }

    #[tokio::test]
    async fn an_unexpired_policy_permits_a_vault_operation() {
        let store = test_store().await;
        let vault = vault_with_policy(&store, &policy_expiring("2099-01-01T00:00:00Z")).await;

        let stored = store.get(&vault.id).await.unwrap();

        assert!(
            crate::policy_expiry::ensure_policy_active(&stored.config_json, Utc::now()).is_ok()
        );

        // The operation itself still goes through.
        let updated = store
            .update(
                &vault.id,
                &UpdateVaultRequest {
                    version: stored.version,
                    name: Some("Renamed".into()),
                    status: None,
                    config_json: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "Renamed");
    }

    /// Existing vaults were created with `config_json = "{}"` and must keep
    /// working — expiry is opt-in.
    #[tokio::test]
    async fn a_vault_without_an_expiry_is_unaffected() {
        let store = test_store().await;
        let vault = vault_with_policy(&store, "{}").await;

        let stored = store.get(&vault.id).await.unwrap();

        assert!(
            crate::policy_expiry::ensure_policy_active(&stored.config_json, Utc::now()).is_ok()
        );
    }

    /// The guard reads the *stored* policy, so an expired vault cannot be
    /// used to extend its own expiry.
    #[tokio::test]
    async fn an_expired_policy_cannot_extend_itself() {
        let store = test_store().await;
        let vault = vault_with_policy(&store, &policy_expiring("2020-01-01T00:00:00Z")).await;

        let stored = store.get(&vault.id).await.unwrap();

        // What the handler checks, before applying any update.
        let guard = crate::policy_expiry::ensure_policy_active(&stored.config_json, Utc::now());

        assert!(
            guard.is_err(),
            "the stored policy is expired, so the update must be refused \
             regardless of what the request body would set it to"
        );
    }

    #[tokio::test]
    async fn a_malformed_policy_is_refused_as_a_bad_request() {
        let store = test_store().await;
        let vault = vault_with_policy(&store, "{not json").await;

        let stored = store.get(&vault.id).await.unwrap();
        let err = crate::policy_expiry::ensure_policy_active(&stored.config_json, Utc::now())
            .unwrap_err();

        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::BadRequest(_)));
    }

    // ── BE-027: pagination ────────────────────────────────────────────────────

    fn pagination(page: u32, page_size: u32) -> crate::db::models::PaginationParams {
        crate::db::models::PaginationParams { page, page_size }
    }

    #[tokio::test]
    async fn list_by_manager_paginated_basic() {
        let store = test_store().await;

        for i in 0..5u32 {
            store
                .create(&CreateVaultRequest {
                    manager_id: "mgr-pg".into(),
                    name: format!("Vault {i}"),
                    status: "active".into(),
                    config_json: "{}".into(),
                    idempotency_key: None,
                })
                .await
                .unwrap();
        }

        let page1 = store
            .list_by_manager("mgr-pg", &pagination(1, 2))
            .await
            .unwrap();
        assert_eq!(page1.data.len(), 2);
        assert_eq!(page1.total_count, 5);
        assert_eq!(page1.page, 1);
        assert_eq!(page1.page_size, 2);
        assert!(page1.has_more);

        let page2 = store
            .list_by_manager("mgr-pg", &pagination(2, 2))
            .await
            .unwrap();
        assert_eq!(page2.data.len(), 2);
        assert!(page2.has_more);

        let page3 = store
            .list_by_manager("mgr-pg", &pagination(3, 2))
            .await
            .unwrap();
        assert_eq!(page3.data.len(), 1);
        assert!(!page3.has_more);
    }

    #[tokio::test]
    async fn list_by_manager_paginated_empty_for_unknown_manager() {
        let store = test_store().await;
        let result = store
            .list_by_manager("unknown-mgr", &pagination(1, 50))
            .await
            .unwrap();
        assert!(result.data.is_empty());
        assert_eq!(result.total_count, 0);
        assert!(!result.has_more);
    }

    #[tokio::test]
    async fn list_by_manager_page_size_clamped_to_max() {
        let store = test_store().await;
        store
            .create(&CreateVaultRequest {
                manager_id: "mgr-clamp".into(),
                name: "Vault".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let result = store
            .list_by_manager("mgr-clamp", &pagination(1, 9999))
            .await
            .unwrap();
        assert_eq!(result.page_size, 200);
        assert_eq!(result.total_count, 1);
    }

    #[tokio::test]
    async fn list_by_manager_beyond_last_page_returns_empty() {
        let store = test_store().await;
        store
            .create(&CreateVaultRequest {
                manager_id: "mgr-eof".into(),
                name: "Vault".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        let result = store
            .list_by_manager("mgr-eof", &pagination(10, 50))
            .await
            .unwrap();
        assert!(result.data.is_empty());
        assert_eq!(result.total_count, 1);
        assert!(!result.has_more);
    }

    // ── BE-044 (#281): soft deletes ─────────────────────────────────────────

    #[tokio::test]
    async fn soft_delete_excludes_vault_from_default_listing() {
        let store = test_store().await;
        let created = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-soft".into(),
                name: "To Delete".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        store.soft_delete(&created.id).await.unwrap();

        // Default `get` no longer finds the vault.
        assert!(store.get(&created.id).await.is_err());

        // Default listing excludes it.
        let listed = store
            .list_by_manager("mgr-soft", &pagination(1, 50))
            .await
            .unwrap();
        assert!(listed.data.is_empty());
        assert_eq!(listed.total_count, 0);

        // It can still be located via the admin-inclusive path.
        let fetched = store.get_including_deleted(&created.id).await.unwrap();
        assert!(fetched.deleted_at.is_some());
    }

    #[tokio::test]
    async fn restore_returns_vault_to_default_queries() {
        let store = test_store().await;
        let created = store
            .create(&CreateVaultRequest {
                manager_id: "mgr-restore".into(),
                name: "Restore Me".into(),
                status: "active".into(),
                config_json: "{}".into(),
                idempotency_key: None,
            })
            .await
            .unwrap();

        store.soft_delete(&created.id).await.unwrap();
        assert!(store.get(&created.id).await.is_err());

        let restored = store.restore(&created.id).await.unwrap();
        assert!(restored.deleted_at.is_none());

        // Default `get` works again.
        let fetched = store.get(&created.id).await.unwrap();
        assert_eq!(fetched.id, created.id);

        // And it reappears in the default listing.
        let listed = store
            .list_by_manager("mgr-restore", &pagination(1, 50))
            .await
            .unwrap();
        assert_eq!(listed.total_count, 1);
    }

    #[tokio::test]
    async fn soft_delete_of_missing_vault_is_not_found() {
        let store = test_store().await;
        assert!(store.soft_delete("does-not-exist").await.is_err());
        assert!(store.restore("does-not-exist").await.is_err());
    }
}
