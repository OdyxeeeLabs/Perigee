//! Cross-tenant isolation tests for manager-owned vault records (API-05).
//!
//! Note: the async report-generation job work (API-22) lives in
//! `core/src/job_queue.rs` and is out of scope for this file; this file only
//! covers vault tenant isolation.
//!
//! Two managers own vaults with identical names; every operation for one
//! manager must be blind to the other's rows, in both directions.

use crate::db;
use crate::db::models::PaginationParams;
use crate::vault_store::{CreateVaultRequest, VaultStore};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

async fn store() -> VaultStore {
    let url = format!("file:iso_{}?mode=memory&cache=shared", Uuid::new_v4());
    let pool = SqlitePoolOptions::new().max_connections(5).connect(&url).await.unwrap();
    sqlx::query(
        "CREATE TABLE vaults (
            id TEXT PRIMARY KEY, manager_id TEXT NOT NULL, name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active', config_json TEXT NOT NULL DEFAULT '{}',
            version INTEGER NOT NULL DEFAULT 1, idempotency_key TEXT,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            deleted_at TIMESTAMP)",
    )
    .execute(&pool)
    .await
    .unwrap();
    VaultStore::new(
        db::schema::TypedSchema::new(std::sync::Arc::new(db::MonitoredPool::from_inner(pool))).vaults(),
    )
}

async fn create(store: &VaultStore, manager: &str) -> String {
    store
        .create(&CreateVaultRequest {
            manager_id: manager.into(),
            name: "Shared name".into(),
            status: "active".into(),
            config_json: "{}".into(),
            idempotency_key: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn managers_never_see_each_others_vaults() {
    let page = PaginationParams { page: 1, page_size: 50 };
    // (owner, other): run in both directions.
    for (owner, other) in [("mgr-a", "mgr-b"), ("mgr-b", "mgr-a")] {
        let store = store().await;
        let own_id = create(&store, owner).await;
        let other_id = create(&store, other).await;

        // List filter: only own rows, never the other tenant's.
        let listed = store.list_by_manager(owner, &page).await.unwrap();
        assert_eq!(listed.total_count, 1, "{owner} list count");
        assert!(listed.data.iter().all(|v| v.manager_id == owner && v.id != other_id));

        // Deleting the other tenant's vault leaves this tenant's list unchanged.
        store.soft_delete(&other_id).await.unwrap();
        let listed = store.list_by_manager(owner, &page).await.unwrap();
        assert_eq!(listed.data.len(), 1);
        assert_eq!(listed.data[0].id, own_id);

        // An unknown tenant sees nothing.
        let none = store.list_by_manager("mgr-unknown", &page).await.unwrap();
        assert_eq!(none.total_count, 0);
    }
}
