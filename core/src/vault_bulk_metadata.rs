//! Safe bulk vault metadata update (API-27).
//!
//! `POST /vaults/bulk-metadata` lets a manager relabel several of their own
//! vaults in one call. The operation is **atomic**: every item is validated
//! first, then all updates are applied in a single database transaction.
//! Either every item is applied, or none is and the response lists each
//! failing item by index.
//!
//! * The batch is bounded to [`MAX_BATCH_SIZE`] items.
//! * Duplicate vault ids in one batch are rejected.
//! * Only off-chain metadata can change: `name`, `labels` and
//!   `reporting_reference`. Policy and lifecycle fields (`config_json`,
//!   `status`, fees, assets, ...) are rejected, so this endpoint cannot change
//!   on-chain policy.
//! * Every item carries the vault's current `version` (optimistic locking, as
//!   for single-vault updates); each applied item bumps it by one.
//! * Tenant isolation: every vault must belong to the caller. A vault owned by
//!   another manager is reported exactly like one that does not exist.
//!
//! See `docs/VAULT_BULK_METADATA.md`.

use crate::auth::AuthenticatedUser;
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::SqlitePool;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

pub const MAX_BATCH_SIZE: usize = 50;
pub const MAX_LABELS: usize = 10;
const MAX_LABEL_LEN: usize = 32;
const MAX_NAME_LEN: usize = 128;
const MAX_REFERENCE_LEN: usize = 64;
const MAX_ID_LEN: usize = 64;

/// Fields an item may contain.
const ALLOWED_FIELDS: &[&str] = &["id", "version", "name", "labels", "reporting_reference"];
/// Fields that describe on-chain policy or lifecycle and are explicitly refused.
const POLICY_FIELDS: &[&str] = &[
    "config_json",
    "config",
    "policy",
    "status",
    "manager_id",
    "allowed_assets",
    "fee_bps",
    "performance_fee_bps",
    "management_fee_bps",
    "max_drawdown_bps",
    "strategy",
    "contract_id",
    "deleted_at",
];

/// One problem with one item of the batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemError {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub code: &'static str,
    pub message: String,
}

impl ItemError {
    fn new(
        index: usize,
        id: Option<&str>,
        field: Option<&str>,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            index,
            id: id.map(str::to_string),
            field: field.map(str::to_string),
            code,
            message: message.into(),
        }
    }
}

/// A validated item, ready to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataUpdate {
    pub index: usize,
    pub id: String,
    pub version: i64,
    pub name: Option<String>,
    pub labels: Option<Vec<String>>,
    /// `Some(None)` clears the reference.
    pub reporting_reference: Option<Option<String>>,
}

/// Post-update view of one vault's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VaultMetadataView {
    pub id: String,
    pub version: i64,
    pub name: String,
    pub labels: Vec<String>,
    pub reporting_reference: Option<String>,
    pub updated_at: String,
}

/// Successful response body.
#[derive(Debug, Clone, Serialize)]
pub struct BulkUpdateResponse {
    /// Always `true`: the batch was applied as one unit.
    pub atomic: bool,
    pub applied: usize,
    pub vaults: Vec<VaultMetadataView>,
}

/// Why a batch was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchRejection {
    /// Request-level problem (empty, too large, not a list).
    Request(String),
    /// Item-level problems; nothing was applied.
    Items(Vec<ItemError>),
    /// The store failed; the transaction was rolled back.
    Store(String),
}

fn validate_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL_LEN
        && label.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b':' | b'.')
        })
}

fn printable(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

/// Validate the raw request body without touching the store.
pub fn validate_batch(body: &Value) -> Result<Vec<MetadataUpdate>, BatchRejection> {
    let Some(object) = body.as_object() else {
        return Err(BatchRejection::Request(
            "body must be an object with an `updates` array".into(),
        ));
    };
    if let Some(extra) = object.keys().find(|k| k.as_str() != "updates") {
        return Err(BatchRejection::Request(format!(
            "unknown top-level field `{extra}`"
        )));
    }
    let Some(items) = object.get("updates").and_then(Value::as_array) else {
        return Err(BatchRejection::Request("`updates` must be an array".into()));
    };
    if items.is_empty() {
        return Err(BatchRejection::Request(
            "`updates` must contain at least one item".into(),
        ));
    }
    if items.len() > MAX_BATCH_SIZE {
        return Err(BatchRejection::Request(format!(
            "`updates` may contain at most {MAX_BATCH_SIZE} items, got {}",
            items.len()
        )));
    }

    let mut errors = Vec::new();
    let mut updates = Vec::with_capacity(items.len());
    let mut first_index_of: HashMap<String, usize> = HashMap::new();

    for (index, item) in items.iter().enumerate() {
        match validate_item(index, item) {
            Ok(update) => {
                if let Some(first) = first_index_of.get(&update.id) {
                    errors.push(ItemError::new(
                        index,
                        Some(&update.id),
                        Some("id"),
                        "duplicate_id",
                        format!(
                            "vault appears more than once in the batch (first at index {first})"
                        ),
                    ));
                } else {
                    first_index_of.insert(update.id.clone(), index);
                    updates.push(update);
                }
            }
            Err(mut item_errors) => errors.append(&mut item_errors),
        }
    }

    if errors.is_empty() {
        Ok(updates)
    } else {
        Err(BatchRejection::Items(errors))
    }
}

fn validate_item(index: usize, item: &Value) -> Result<MetadataUpdate, Vec<ItemError>> {
    let Some(fields) = item.as_object() else {
        return Err(vec![ItemError::new(
            index,
            None,
            None,
            "invalid_item",
            "each update must be an object",
        )]);
    };
    let id = fields.get("id").and_then(Value::as_str).map(str::trim);
    let mut errors = Vec::new();

    for key in fields.keys() {
        if POLICY_FIELDS.contains(&key.as_str()) {
            errors.push(ItemError::new(
                index,
                id,
                Some(key),
                "forbidden_field",
                format!("`{key}` is policy or lifecycle state and cannot be changed by a metadata update"),
            ));
        } else if !ALLOWED_FIELDS.contains(&key.as_str()) {
            errors.push(ItemError::new(
                index,
                id,
                Some(key),
                "unknown_field",
                format!("unknown field `{key}`"),
            ));
        }
    }

    let id = match id {
        Some(id) if !id.is_empty() && id.len() <= MAX_ID_LEN && printable(id) => {
            Some(id.to_string())
        }
        _ => {
            errors.push(ItemError::new(
                index,
                None,
                Some("id"),
                "invalid_id",
                "id must be a non-empty string",
            ));
            None
        }
    };

    let version = match fields.get("version").and_then(Value::as_i64) {
        Some(v) if v >= 1 => Some(v),
        _ => {
            errors.push(ItemError::new(
                index,
                id.as_deref(),
                Some("version"),
                "invalid_version",
                "version must be the vault's current version (a positive integer)",
            ));
            None
        }
    };

    let name = match fields.get("name") {
        None => None,
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME_LEN || !printable(trimmed) {
                errors.push(ItemError::new(
                    index,
                    id.as_deref(),
                    Some("name"),
                    "invalid_name",
                    format!("name must be 1-{MAX_NAME_LEN} printable characters"),
                ));
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => {
            errors.push(ItemError::new(
                index,
                id.as_deref(),
                Some("name"),
                "invalid_name",
                "name must be a string",
            ));
            None
        }
    };

    let labels = match fields.get("labels") {
        None => None,
        Some(Value::Array(values)) => {
            let mut set = BTreeSet::new();
            let mut valid = values.len() <= MAX_LABELS;
            for value in values {
                match value.as_str() {
                    Some(label) if validate_label(label) => {
                        set.insert(label.to_string());
                    }
                    _ => valid = false,
                }
            }
            if valid {
                Some(set.into_iter().collect())
            } else {
                errors.push(ItemError::new(
                    index,
                    id.as_deref(),
                    Some("labels"),
                    "invalid_labels",
                    format!(
                        "labels must be at most {MAX_LABELS} strings of 1-{MAX_LABEL_LEN} lowercase letters, digits, `-`, `_`, `:` or `.`"
                    ),
                ));
                None
            }
        }
        Some(_) => {
            errors.push(ItemError::new(
                index,
                id.as_deref(),
                Some("labels"),
                "invalid_labels",
                "labels must be an array",
            ));
            None
        }
    };

    let reporting_reference = match fields.get("reporting_reference") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.len() > MAX_REFERENCE_LEN || !printable(trimmed) {
                errors.push(ItemError::new(
                    index,
                    id.as_deref(),
                    Some("reporting_reference"),
                    "invalid_reporting_reference",
                    format!("reporting_reference must be 1-{MAX_REFERENCE_LEN} printable characters or null"),
                ));
                None
            } else {
                Some(Some(trimmed.to_string()))
            }
        }
        Some(_) => {
            errors.push(ItemError::new(
                index,
                id.as_deref(),
                Some("reporting_reference"),
                "invalid_reporting_reference",
                "reporting_reference must be a string or null",
            ));
            None
        }
    };

    if name.is_none() && labels.is_none() && reporting_reference.is_none() && errors.is_empty() {
        errors.push(ItemError::new(
            index,
            id.as_deref(),
            None,
            "empty_update",
            "provide at least one of `name`, `labels` or `reporting_reference`",
        ));
    }

    match (errors.is_empty(), id, version) {
        (true, Some(id), Some(version)) => Ok(MetadataUpdate {
            index,
            id,
            version,
            name,
            labels,
            reporting_reference,
        }),
        _ => Err(errors),
    }
}

/// Applies validated batches to the SQLite `vaults` / `vault_metadata` tables.
pub struct VaultBulkMetadataService {
    pool: Option<SqlitePool>,
}

impl VaultBulkMetadataService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool: Some(pool) }
    }

    /// A service whose store is not configured; every call answers `503`.
    pub fn unavailable() -> Self {
        Self { pool: None }
    }

    /// Apply every update for `manager_id` in one transaction, or none.
    pub async fn apply(
        &self,
        manager_id: &str,
        updates: &[MetadataUpdate],
    ) -> Result<Vec<VaultMetadataView>, BatchRejection> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| BatchRejection::Store("vault store is not configured".into()))?;
        let store_error = |e: sqlx::Error| BatchRejection::Store(e.to_string());
        let mut tx = pool.begin().await.map_err(store_error)?;

        // Phase 1: check ownership and versions of every item.
        let mut errors = Vec::new();
        let mut current = Vec::with_capacity(updates.len());
        for update in updates {
            let row = sqlx::query_as::<_, (String, String, i64)>(
                "SELECT manager_id, name, version FROM vaults WHERE id = ?1 AND deleted_at IS NULL",
            )
            .bind(&update.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            match row {
                Some((owner, name, version)) if owner == manager_id => {
                    if version != update.version {
                        errors.push(ItemError::new(
                            update.index,
                            Some(&update.id),
                            Some("version"),
                            "version_conflict",
                            format!("vault is at version {version}; reload and retry"),
                        ));
                    }
                    current.push(name);
                }
                // Another tenant's vault is indistinguishable from a missing one.
                _ => {
                    errors.push(ItemError::new(
                        update.index,
                        Some(&update.id),
                        Some("id"),
                        "not_found",
                        "vault not found",
                    ));
                    current.push(String::new());
                }
            }
        }
        if !errors.is_empty() {
            tx.rollback().await.map_err(store_error)?;
            return Err(BatchRejection::Items(errors));
        }

        // Phase 2: apply. Any surprise rolls back the whole batch.
        let now = Utc::now();
        let now_text = now.to_rfc3339_opts(SecondsFormat::AutoSi, true);
        let mut views = Vec::with_capacity(updates.len());
        for (update, current_name) in updates.iter().zip(current) {
            let name = update.name.clone().unwrap_or(current_name);
            let result = sqlx::query(
                "UPDATE vaults SET name = ?1, version = version + 1, updated_at = ?2 \
                 WHERE id = ?3 AND manager_id = ?4 AND version = ?5 AND deleted_at IS NULL",
            )
            .bind(&name)
            .bind(&now_text)
            .bind(&update.id)
            .bind(manager_id)
            .bind(update.version)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                tx.rollback().await.map_err(store_error)?;
                return Err(BatchRejection::Items(vec![ItemError::new(
                    update.index,
                    Some(&update.id),
                    Some("version"),
                    "version_conflict",
                    "vault changed while the batch was being applied; reload and retry",
                )]));
            }

            let existing = sqlx::query_as::<_, (String, Option<String>)>(
                "SELECT labels_json, reporting_reference FROM vault_metadata WHERE vault_id = ?1",
            )
            .bind(&update.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let (old_labels, old_reference) = existing.unwrap_or_else(|| ("[]".to_string(), None));
            let labels: Vec<String> = match &update.labels {
                Some(labels) => labels.clone(),
                None => serde_json::from_str(&old_labels).unwrap_or_default(),
            };
            let reference = match &update.reporting_reference {
                Some(value) => value.clone(),
                None => old_reference,
            };
            let labels_json =
                serde_json::to_string(&labels).map_err(|e| BatchRejection::Store(e.to_string()))?;
            sqlx::query(
                "INSERT INTO vault_metadata (vault_id, labels_json, reporting_reference, updated_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(vault_id) DO UPDATE SET labels_json = excluded.labels_json, \
                 reporting_reference = excluded.reporting_reference, updated_at = excluded.updated_at",
            )
            .bind(&update.id)
            .bind(&labels_json)
            .bind(&reference)
            .bind(&now_text)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;

            views.push(VaultMetadataView {
                id: update.id.clone(),
                version: update.version + 1,
                name,
                labels,
                reporting_reference: reference,
                updated_at: now_text.clone(),
            });
        }
        tx.commit().await.map_err(store_error)?;
        Ok(views)
    }
}

fn problem(status: StatusCode, detail: &str, extensions: Map<String, Value>) -> Response {
    let mut problem =
        ProblemDetails::from_status(status.as_u16(), detail).with_instance("/vaults/bulk-metadata");
    for (key, value) in extensions {
        problem = problem.with_extension(&key, value);
    }
    let mut response = (status, Json(problem.to_json())).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

fn rejection_response(rejection: BatchRejection) -> Response {
    let mut extensions = Map::new();
    extensions.insert("applied".into(), json!(0));
    match rejection {
        BatchRejection::Request(detail) => {
            problem(StatusCode::UNPROCESSABLE_ENTITY, &detail, extensions)
        }
        BatchRejection::Items(errors) => {
            let status = if errors.iter().any(|e| e.code == "version_conflict") {
                StatusCode::CONFLICT
            } else if errors.iter().all(|e| e.code == "not_found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            extensions.insert(
                "errors".into(),
                serde_json::to_value(&errors).unwrap_or(Value::Null),
            );
            problem(
                status,
                "No vaults were updated: the batch is atomic and at least one item was rejected",
                extensions,
            )
        }
        BatchRejection::Store(error) => {
            tracing::error!(error = %error, "bulk vault metadata update failed");
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "No vaults were updated: the vault store is unavailable",
                extensions,
            )
        }
    }
}

/// `POST /vaults/bulk-metadata`
pub async fn bulk_update_vault_metadata_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(service): Extension<Arc<VaultBulkMetadataService>>,
    Json(body): Json<Value>,
) -> Response {
    if !user.can_write_vaults() {
        return problem(
            StatusCode::FORBIDDEN,
            "Your role cannot update vault metadata",
            Map::new(),
        );
    }
    let updates = match validate_batch(&body) {
        Ok(updates) => updates,
        Err(rejection) => return rejection_response(rejection),
    };
    // Scoped tokens may only touch the vaults they were issued for.
    let out_of_scope: Vec<ItemError> = updates
        .iter()
        .filter(|u| !user.can_access_vault(&u.id))
        .map(|u| {
            ItemError::new(
                u.index,
                Some(&u.id),
                Some("id"),
                "not_found",
                "vault not found",
            )
        })
        .collect();
    if !out_of_scope.is_empty() {
        return rejection_response(BatchRejection::Items(out_of_scope));
    }

    match service.apply(&user.stellar_address, &updates).await {
        Ok(vaults) => Json(BulkUpdateResponse {
            atomic: true,
            applied: vaults.len(),
            vaults,
        })
        .into_response(),
        Err(rejection) => rejection_response(rejection),
    }
}

/// Parse a stored RFC 3339 `updated_at` (exposed for callers that need it).
pub fn parse_updated_at(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use axum::{body::Body, http::Request, routing::post, Router};
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;

    const ALICE: &str = "GALICE";
    const BOB: &str = "GBOB";

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrations::run_migrations(&pool).await.unwrap();
        for (id, owner) in [("v1", ALICE), ("v2", ALICE), ("v3", ALICE), ("b1", BOB)] {
            sqlx::query(
                "INSERT INTO vaults (id, manager_id, name, status, config_json, version) VALUES (?1, ?2, ?3, 'active', '{\"fee_bps\":100}', 1)",
            )
            .bind(id)
            .bind(owner)
            .bind(format!("Vault {id}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    async fn snapshot(pool: &SqlitePool) -> Vec<(String, String, i64, String, String)> {
        sqlx::query_as(
            "SELECT v.id, v.name, v.version, v.config_json, COALESCE(m.labels_json, '') \
             FROM vaults v LEFT JOIN vault_metadata m ON m.vault_id = v.id ORDER BY v.id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    fn codes(rejection: BatchRejection) -> Vec<(usize, &'static str)> {
        match rejection {
            BatchRejection::Items(errors) => {
                errors.into_iter().map(|e| (e.index, e.code)).collect()
            }
            other => panic!("expected item errors, got {other:?}"),
        }
    }

    #[test]
    fn batch_size_is_bounded() {
        let at_limit: Vec<Value> = (0..MAX_BATCH_SIZE)
            .map(|i| json!({"id": format!("v{i}"), "version": 1, "name": "x"}))
            .collect();
        assert!(validate_batch(&json!({ "updates": at_limit })).is_ok());

        let over: Vec<Value> = (0..=MAX_BATCH_SIZE)
            .map(|i| json!({"id": format!("v{i}"), "version": 1, "name": "x"}))
            .collect();
        assert!(matches!(
            validate_batch(&json!({ "updates": over })),
            Err(BatchRejection::Request(msg)) if msg.contains("at most 50")
        ));
        assert!(matches!(
            validate_batch(&json!({"updates": []})),
            Err(BatchRejection::Request(_))
        ));
        assert!(matches!(
            validate_batch(&json!([])),
            Err(BatchRejection::Request(_))
        ));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let body = json!({"updates": [
            {"id": "v1", "version": 1, "name": "a"},
            {"id": "v2", "version": 1, "name": "b"},
            {"id": "v1", "version": 1, "labels": ["x"]}
        ]});
        assert_eq!(
            codes(validate_batch(&body).unwrap_err()),
            vec![(2, "duplicate_id")]
        );
    }

    #[test]
    fn invalid_mixed_input_reports_every_bad_item() {
        let body = json!({"updates": [
            {"id": "v1", "version": 1, "name": "fine"},
            {"id": "v2", "version": 1, "config_json": "{\"fee_bps\":0}"},
            {"id": "v3", "version": 0, "name": "x"},
            {"id": "v4", "version": 1, "labels": ["Not Valid!"]},
            {"id": "v5", "version": 1},
            {"id": "", "version": 1, "name": "x"},
            {"id": "v6", "version": 1, "colour": "red"},
            "not-an-object"
        ]});
        let found = codes(validate_batch(&body).unwrap_err());
        assert_eq!(
            found,
            vec![
                (1, "forbidden_field"),
                (2, "invalid_version"),
                (3, "invalid_labels"),
                (4, "empty_update"),
                (5, "invalid_id"),
                (6, "unknown_field"),
                (7, "invalid_item"),
            ]
        );
    }

    #[test]
    fn on_chain_policy_fields_cannot_be_changed() {
        for field in [
            "config_json",
            "status",
            "policy",
            "allowed_assets",
            "fee_bps",
            "manager_id",
        ] {
            let body =
                json!({"updates": [{"id": "v1", "version": 1, "name": "x", field: "anything"}]});
            assert_eq!(
                codes(validate_batch(&body).unwrap_err()),
                vec![(0, "forbidden_field")],
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn a_valid_batch_is_applied_atomically() {
        let pool = pool().await;
        let service = VaultBulkMetadataService::new(pool.clone());
        let updates = validate_batch(&json!({"updates": [
            {"id": "v1", "version": 1, "name": "Treasury", "labels": ["core", "tier:1"]},
            {"id": "v2", "version": 1, "reporting_reference": "RPT-2026-11"}
        ]}))
        .unwrap();
        let views = service.apply(ALICE, &updates).await.unwrap();
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].version, 2);
        assert_eq!(views[0].labels, vec!["core", "tier:1"]);
        assert_eq!(views[1].name, "Vault v2");
        assert_eq!(views[1].reporting_reference.as_deref(), Some("RPT-2026-11"));
        assert!(parse_updated_at(&views[0].updated_at).is_some());

        // config_json (on-chain policy) is untouched.
        let after = snapshot(&pool).await;
        assert!(after.iter().all(|row| row.3 == "{\"fee_bps\":100}"));

        // Labels survive a later reference-only update.
        let next = validate_batch(
            &json!({"updates": [{"id": "v1", "version": 2, "reporting_reference": null}]}),
        )
        .unwrap();
        let views = service.apply(ALICE, &next).await.unwrap();
        assert_eq!(views[0].labels, vec!["core", "tier:1"]);
        assert_eq!(views[0].reporting_reference, None);
    }

    #[tokio::test]
    async fn one_bad_item_means_nothing_is_applied() {
        let pool = pool().await;
        let service = VaultBulkMetadataService::new(pool.clone());
        let before = snapshot(&pool).await;
        let updates = validate_batch(&json!({"updates": [
            {"id": "v1", "version": 1, "name": "A"},
            {"id": "v2", "version": 7, "name": "B"},
            {"id": "v3", "version": 1, "labels": ["c"]}
        ]}))
        .unwrap();
        let rejection = service.apply(ALICE, &updates).await.unwrap_err();
        assert_eq!(codes(rejection), vec![(1, "version_conflict")]);
        assert_eq!(snapshot(&pool).await, before, "atomic: no partial writes");
    }

    #[tokio::test]
    async fn other_tenants_vaults_are_isolated() {
        let pool = pool().await;
        let service = VaultBulkMetadataService::new(pool.clone());
        let before = snapshot(&pool).await;
        let updates = validate_batch(&json!({"updates": [
            {"id": "v1", "version": 1, "name": "mine"},
            {"id": "b1", "version": 1, "name": "hijacked"}
        ]}))
        .unwrap();
        let rejection = service.apply(ALICE, &updates).await.unwrap_err();
        match rejection {
            BatchRejection::Items(errors) => {
                assert_eq!(errors.len(), 1);
                assert_eq!((errors[0].index, errors[0].code), (1, "not_found"));
                assert_eq!(
                    errors[0].message, "vault not found",
                    "same answer as a missing vault"
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(snapshot(&pool).await, before);

        // Bob can update his own vault.
        let bob =
            validate_batch(&json!({"updates": [{"id": "b1", "version": 1, "name": "Bob's"}]}))
                .unwrap();
        assert!(service.apply(BOB, &bob).await.is_ok());
    }

    #[tokio::test]
    async fn soft_deleted_vaults_are_not_updated() {
        let pool = pool().await;
        sqlx::query("UPDATE vaults SET deleted_at = '2026-11-19T00:00:00Z' WHERE id = 'v3'")
            .execute(&pool)
            .await
            .unwrap();
        let service = VaultBulkMetadataService::new(pool);
        let updates =
            validate_batch(&json!({"updates": [{"id": "v3", "version": 1, "name": "x"}]})).unwrap();
        assert_eq!(
            codes(service.apply(ALICE, &updates).await.unwrap_err()),
            vec![(0, "not_found")]
        );
    }

    fn app(service: Arc<VaultBulkMetadataService>, user: AuthenticatedUser) -> Router {
        Router::new()
            .route(
                "/vaults/bulk-metadata",
                post(bulk_update_vault_metadata_handler),
            )
            .layer(Extension(service))
            .layer(Extension(user))
    }

    async fn post_json(router: Router, body: Value) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::post("/vaults/bulk-metadata")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn http_contract_documents_atomicity() {
        let pool = pool().await;
        let service = Arc::new(VaultBulkMetadataService::new(pool));
        let alice = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);

        let (status, body) = post_json(
            app(service.clone(), alice.clone()),
            json!({"updates": [{"id": "v1", "version": 1, "labels": ["a"]}, {"id": "v2", "version": 1, "name": "B"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["atomic"], true);
        assert_eq!(body["applied"], 2);

        let (status, body) = post_json(
            app(service.clone(), alice.clone()),
            json!({"updates": [{"id": "v3", "version": 1, "name": "ok"}, {"id": "v1", "version": 1, "name": "stale"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["applied"], 0);
        assert_eq!(body["errors"][0]["index"], 1);
        assert_eq!(body["errors"][0]["code"], "version_conflict");

        let (status, body) = post_json(
            app(service.clone(), alice),
            json!({"updates": [{"id": "v1", "version": 2, "config_json": "{}"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["errors"][0]["code"], "forbidden_field");

        let viewer = AuthenticatedUser::new(ALICE.into(), Role::Viewer, vec![]);
        let (status, _) = post_json(
            app(service.clone(), viewer),
            json!({"updates": [{"id": "v1", "version": 2, "name": "x"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // A token scoped to v1 cannot reach v2 even though Alice owns it.
        let scoped = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec!["v1".into()]);
        let (status, body) = post_json(
            app(service, scoped),
            json!({"updates": [{"id": "v2", "version": 2, "name": "x"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["applied"], 0);
    }

    #[tokio::test]
    async fn an_unconfigured_store_is_reported_as_unavailable() {
        let service = Arc::new(VaultBulkMetadataService::unavailable());
        let alice = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);
        let (status, body) = post_json(
            app(service, alice),
            json!({"updates": [{"id": "v1", "version": 1, "name": "x"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["applied"], 0);
    }
}
