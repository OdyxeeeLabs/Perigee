use std::collections::HashSet;

use sha2::{Digest, Sha256};
use sqlx::{Executor, Sqlite, SqlitePool, Transaction};

#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

impl Migration {
    pub fn checksum(&self) -> String {
        let normalized = self.sql.replace("\r\n", "\n").replace('\r', "\n");
        hex::encode(Sha256::digest(normalized.as_bytes()))
    }
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "create_jobs_table",
        sql: include_str!("../../migrations/001_create_jobs_table.sql"),
    },
    Migration {
        version: 2,
        name: "create_fee_tables",
        sql: include_str!("../../migrations/002_create_fee_tables.sql"),
    },
    Migration {
        version: 3,
        name: "create_reconciliation_tables",
        sql: include_str!("../../migrations/003_create_reconciliation_tables.sql"),
    },
    Migration {
        version: 4,
        name: "create_vaults_table",
        sql: include_str!("../../migrations/004_create_vaults_table.sql"),
    },
    Migration {
        version: 5,
        name: "add_vault_idempotency_key",
        sql: include_str!("../../migrations/005_add_vault_idempotency_key.sql"),
    },
    Migration {
        version: 6,
        name: "create_managers_table",
        sql: include_str!("../../migrations/006_create_managers_table.sql"),
    },
    Migration {
        version: 7,
        name: "add_vault_soft_delete",
        sql: include_str!("../../migrations/007_add_vault_soft_delete.sql"),
    },
];

pub fn validate_migrations(migrations: &[Migration]) -> Result<(), String> {
    if migrations.is_empty() {
        return Err("at least one migration is required".to_string());
    }

    let mut previous_version = None;
    let mut names = HashSet::new();
    for migration in migrations {
        if migration.version <= 0 {
            return Err(format!("migration {} has an invalid version", migration.name));
        }
        if migration.sql.trim().is_empty() {
            return Err(format!("migration {} has an empty SQL body", migration.name));
        }
        let normalized_sql = migration.sql.to_ascii_lowercase();
        if normalized_sql
            .lines()
            .any(|line| line.trim_start().starts_with("drop "))
        {
            return Err(format!(
                "migration {} contains a destructive schema change",
                migration.name
            ));
        }
        if let Some(previous) = previous_version {
            if migration.version <= previous {
                return Err(format!(
                    "migration versions must be strictly increasing: {} follows {}",
                    migration.version, previous
                ));
            }
        }
        if !names.insert(migration.name) {
            return Err(format!("duplicate migration name: {}", migration.name));
        }
        previous_version = Some(migration.version);
    }
    Ok(())
}

pub struct MigrationRunner {
    migrations: Vec<Migration>,
}

impl MigrationRunner {
    pub fn new() -> Self {
        Self {
            migrations: MIGRATIONS.to_vec(),
        }
    }

    pub fn with_migrations(migrations: Vec<Migration>) -> Self {
        Self { migrations }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_migrations(&self.migrations)
    }

    pub async fn run(&self, pool: &SqlitePool) -> Result<(), sqlx::Error> {
        self.validate().map_err(protocol_error)?;
        tracing::info!(count = self.migrations.len(), "Applying database migrations");
        let mut transaction = pool.begin().await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _perigee_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, checksum TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)",
        )
        .execute(&mut *transaction)
        .await?;
        adopt_legacy_history(&mut *transaction, &self.migrations).await?;
        let known_versions: HashSet<i64> = self.migrations.iter().map(|m| m.version).collect();
        let applied_versions = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM _perigee_migrations ORDER BY version",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if let Some(version) = applied_versions
            .into_iter()
            .find(|version| !known_versions.contains(version))
        {
            return Err(protocol_error(format!(
                "database contains unknown migration version {version}"
            )));
        }
        for migration in &self.migrations {
            let existing = sqlx::query_as::<_, (String, String)>(
                "SELECT name, checksum FROM _perigee_migrations WHERE version = ?1",
            )
            .bind(migration.version)
            .fetch_optional(&mut *transaction)
            .await?;

            if let Some((name, checksum)) = existing {
                if name != migration.name || checksum != migration.checksum() {
                    return Err(protocol_error(format!(
                        "migration {} was modified after it was applied",
                        migration.version
                    )));
                }
                match migration.name {
                    "add_vault_idempotency_key" => {
                        (&mut *transaction)
                            .execute(
                                "CREATE UNIQUE INDEX IF NOT EXISTS idx_vaults_idempotency ON vaults(manager_id, idempotency_key) WHERE idempotency_key IS NOT NULL",
                            )
                            .await?;
                    }
                    "add_vault_soft_delete" => {
                        (&mut *transaction)
                            .execute(
                                "CREATE INDEX IF NOT EXISTS idx_vaults_deleted_at ON vaults(deleted_at)",
                            )
                            .await?;
                    }
                    _ => {}
                }
                continue;
            }

            let already_applied = match migration.name {
                "add_vault_idempotency_key" => {
                    column_exists(&mut *transaction, "vaults", "idempotency_key").await?
                }
                "add_vault_soft_delete" => {
                    column_exists(&mut *transaction, "vaults", "deleted_at").await?
                }
                _ => false,
            };
            if already_applied {
                match migration.name {
                    "add_vault_idempotency_key" => {
                        (&mut *transaction)
                            .execute(
                                "CREATE UNIQUE INDEX IF NOT EXISTS idx_vaults_idempotency ON vaults(manager_id, idempotency_key) WHERE idempotency_key IS NOT NULL",
                            )
                            .await?;
                    }
                    "add_vault_soft_delete" => {
                        (&mut *transaction)
                            .execute(
                                "CREATE INDEX IF NOT EXISTS idx_vaults_deleted_at ON vaults(deleted_at)",
                            )
                            .await?;
                    }
                    _ => {}
                }
            } else {
                (&mut *transaction).execute(migration.sql).await?;
            }
            sqlx::query(
                "INSERT INTO _perigee_migrations (version, name, checksum) VALUES (?1, ?2, ?3)",
            )
            .bind(migration.version)
            .bind(migration.name)
            .bind(migration.checksum())
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        tracing::info!("Database migrations applied");
        Ok(())
    }
}

impl Default for MigrationRunner {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    MigrationRunner::new().run(pool).await
}

async fn adopt_legacy_history(
    transaction: &mut Transaction<'_, Sqlite>,
    migrations: &[Migration],
) -> Result<(), sqlx::Error> {
    let legacy_table_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut **transaction)
    .await?;
    if legacy_table_count == 0 {
        return Ok(());
    }

    let custom_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM _perigee_migrations",
    )
    .fetch_one(&mut **transaction)
    .await?;
    let legacy_versions = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE success = TRUE ORDER BY version",
    )
    .fetch_all(&mut **transaction)
    .await?;
    if legacy_versions.is_empty() {
        return Ok(());
    }
    if custom_count > 0 {
        let max_version = migrations.iter().map(|migration| migration.version).max().unwrap_or(0);
        if legacy_versions.iter().any(|version| *version < 1 || *version > max_version) {
            return Err(protocol_error(
                "legacy database contains an unknown migration version",
            ));
        }
        return Ok(());
    }

    let managers_exists = table_exists(&mut *transaction, "managers").await?;
    let deleted_at_exists = column_exists(&mut *transaction, "vaults", "deleted_at").await?;
    let mut adopted_versions = HashSet::new();
    for version in legacy_versions {
        match version {
            1..=5 => {}
            6 if managers_exists && deleted_at_exists => {
                adopted_versions.insert(6);
                adopted_versions.insert(7);
            }
            6 if managers_exists => {
                adopted_versions.insert(6);
            }
            6 if deleted_at_exists => {
                adopted_versions.insert(7);
            }
            6 => {
                return Err(protocol_error(
                    "legacy migration version 6 is ambiguous; reconcile managers and vault soft-delete history",
                ));
            }
            7 => {
                adopted_versions.insert(7);
            }
            _ => {
                return Err(protocol_error(format!(
                    "legacy database contains unknown migration version {version}"
                )));
            }
        }
    }

    for migration in migrations {
        if adopted_versions.contains(&migration.version) {
            sqlx::query(
                "INSERT OR IGNORE INTO _perigee_migrations (version, name, checksum) VALUES (?1, ?2, ?3)",
            )
            .bind(migration.version)
            .bind(migration.name)
            .bind(migration.checksum())
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}

async fn table_exists(
    transaction: &mut Transaction<'_, Sqlite>,
    table: &str,
) -> Result<bool, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(count > 0)
}

async fn column_exists(
    transaction: &mut Transaction<'_, Sqlite>,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
    )
    .bind(table)
    .bind(column)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(count > 0)
}

fn protocol_error(message: impl Into<String>) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn migration_catalog_is_ordered_and_unique() {
        assert!(validate_migrations(MIGRATIONS).is_ok());
        assert!(validate_migrations(&[
            MIGRATIONS[0],
            Migration {
                version: 1,
                name: "duplicate",
                sql: "SELECT 1",
            },
        ])
        .is_err());
        assert!(validate_migrations(&[Migration {
            version: 1,
            name: "destructive",
            sql: "DROP TABLE vaults",
        }])
        .is_err());
    }

    #[tokio::test]
    async fn each_migration_applies_after_the_previous_prefix() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for prefix_len in 1..MIGRATIONS.len() {
            MigrationRunner::with_migrations(MIGRATIONS[..prefix_len].to_vec())
                .run(&pool)
                .await
                .unwrap();
        }
        run_migrations(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn every_migration_applies_forward_and_is_idempotent() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();

        let tables = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for table in [
            "jobs",
            "ledger_fee_samples",
            "transaction_fee_records",
            "reconciliation_reports",
            "reconciliation_discrepancies",
            "vaults",
            "managers",
        ] {
            assert!(tables.iter().any(|name| name == table), "missing {table}");
        }

        let versions = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM _perigee_migrations ORDER BY version",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let expected_versions: Vec<i64> = (1..=7).collect();
        assert_eq!(versions, expected_versions);

        let vault_columns = sqlx::query_scalar::<_, String>(
            "SELECT name FROM pragma_table_info('vaults')",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(vault_columns.iter().any(|column| column == "deleted_at"));
    }
}
