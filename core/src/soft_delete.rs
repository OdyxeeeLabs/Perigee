//! # Soft deletes
//!
//! Recoverable deletion for any table that carries `deleted_at` and
//! `deleted_by` columns.
//!
//! * **Default scope** hides deleted rows ([`Scope::Active`]); administrators can
//!   ask for [`Scope::WithDeleted`] or [`Scope::OnlyDeleted`]
//!   ([`scope_for`] enforces that regular users never get more than `Active`).
//! * **Cascades** are configured per relationship ([`Relationship`]): deleting a
//!   parent can be refused while live children exist (`Restrict`), delete the
//!   children with it (`Cascade`), or detach them (`Nullify`).
//! * **Restore** brings a row back, together with the children that were
//!   cascaded *by that delete* (they share its `deleted_at`).
//! * **Purge** hard-deletes rows that have been soft-deleted for longer than the
//!   retention window ([`RETENTION_DAYS`], 30) to satisfy GDPR erasure.
//! * **Audit**: every delete, restore and purge writes to `deletion_audit`,
//!   which is never purged, so the trail outlives the data.
//!
//! This module produces parameterised SQL ([`Statement`]) for a database layer
//! to run in a single transaction; it never builds SQL from untrusted values
//! (identifiers are validated, values are bound). See `docs/SOFT_DELETES.md`.

use thiserror::Error;

/// How long a soft-deleted row is kept before the purge job removes it.
pub const RETENTION_DAYS: u64 = 30;
const SECS_PER_DAY: u64 = 24 * 60 * 60;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SoftDeleteError {
    #[error("`{0}` is not a valid SQL identifier")]
    InvalidIdentifier(String),
    #[error("administrator access is required")]
    Forbidden,
}

fn ident(name: &str) -> Result<&str, SoftDeleteError> {
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if valid {
        Ok(name)
    } else {
        Err(SoftDeleteError::InvalidIdentifier(name.to_string()))
    }
}

/// A bound parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    Text(String),
    Int(i64),
}

/// SQL plus its positional (`?`) parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Param>,
    /// Restrict check: the statement is a `SELECT COUNT(*)` that must return 0,
    /// otherwise the whole delete must be refused.
    pub must_be_zero: bool,
}

impl Statement {
    fn new(sql: String, params: Vec<Param>) -> Self {
        Statement { sql, params, must_be_zero: false }
    }

    fn guard(sql: String, params: Vec<Param>) -> Self {
        Statement { sql, params, must_be_zero: true }
    }
}

// ---------------------------------------------------------------------------
// Scopes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The default: rows that are not deleted.
    Active,
    /// Admin only: everything.
    WithDeleted,
    /// Admin only: the trash.
    OnlyDeleted,
}

/// The scope a caller may use. Non-admins always get `Active`, whatever they ask for.
pub fn scope_for(requested: Scope, is_admin: bool) -> Scope {
    if is_admin {
        requested
    } else {
        Scope::Active
    }
}

/// `WHERE` fragment for `scope`; `alias` is an optional table alias.
pub fn scope_clause(alias: Option<&str>, scope: Scope) -> Result<Option<String>, SoftDeleteError> {
    let prefix = match alias {
        Some(a) => format!("{}.", ident(a)?),
        None => String::new(),
    };
    Ok(match scope {
        Scope::Active => Some(format!("{prefix}deleted_at IS NULL")),
        Scope::OnlyDeleted => Some(format!("{prefix}deleted_at IS NOT NULL")),
        Scope::WithDeleted => None,
    })
}

// ---------------------------------------------------------------------------
// Relationships and plans
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletePolicy {
    /// Refuse to delete the parent while live children exist.
    Restrict,
    /// Soft-delete the children too.
    Cascade,
    /// Keep the children and clear their foreign key.
    Nullify,
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub child_table: String,
    pub foreign_key: String,
    pub policy: DeletePolicy,
}

impl Relationship {
    pub fn new(child_table: &str, foreign_key: &str, policy: DeletePolicy) -> Self {
        Relationship { child_table: child_table.to_string(), foreign_key: foreign_key.to_string(), policy }
    }
}

fn text(value: &str) -> Param {
    Param::Text(value.to_string())
}

fn stamp(epoch_secs: u64) -> String {
    // RFC 3339 UTC, lexicographically sortable like the CURRENT_TIMESTAMP columns.
    chrono::DateTime::<chrono::Utc>::from_timestamp(epoch_secs as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Statements that soft-delete one row and apply the relationship policies. Run
/// them in order inside one transaction; abort if a `must_be_zero` check is non-zero.
pub fn plan_delete(
    table: &str,
    id: &str,
    deleted_by: &str,
    now: u64,
    relationships: &[Relationship],
) -> Result<Vec<Statement>, SoftDeleteError> {
    let table = ident(table)?;
    let at = stamp(now);
    let mut plan = Vec::new();

    for rel in relationships {
        let (child, fk) = (ident(&rel.child_table)?, ident(&rel.foreign_key)?);
        match rel.policy {
            DeletePolicy::Restrict => plan.push(Statement::guard(
                format!("SELECT COUNT(*) FROM {child} WHERE {fk} = ? AND deleted_at IS NULL"),
                vec![text(id)],
            )),
            DeletePolicy::Cascade => plan.push(Statement::new(
                format!("UPDATE {child} SET deleted_at = ?, deleted_by = ? WHERE {fk} = ? AND deleted_at IS NULL"),
                vec![text(&at), text(deleted_by), text(id)],
            )),
            DeletePolicy::Nullify => plan.push(Statement::new(
                format!("UPDATE {child} SET {fk} = NULL WHERE {fk} = ?"),
                vec![text(id)],
            )),
        }
    }

    plan.push(Statement::new(
        format!("UPDATE {table} SET deleted_at = ?, deleted_by = ? WHERE id = ? AND deleted_at IS NULL"),
        vec![text(&at), text(deleted_by), text(id)],
    ));
    plan.push(audit_statement(table, id, "DELETE", deleted_by, &at));
    Ok(plan)
}

/// Restore a row and the children that were cascaded together with it. Admin only.
pub fn plan_restore(
    table: &str,
    id: &str,
    restored_by: &str,
    is_admin: bool,
    now: u64,
    relationships: &[Relationship],
) -> Result<Vec<Statement>, SoftDeleteError> {
    if !is_admin {
        return Err(SoftDeleteError::Forbidden);
    }
    let table = ident(table)?;
    let mut plan = Vec::new();

    // Children first: they are matched on the parent's deleted_at, which the
    // parent update below clears.
    for rel in relationships.iter().filter(|r| r.policy == DeletePolicy::Cascade) {
        let (child, fk) = (ident(&rel.child_table)?, ident(&rel.foreign_key)?);
        plan.push(Statement::new(
            format!(
                "UPDATE {child} SET deleted_at = NULL, deleted_by = NULL WHERE {fk} = ? \
                 AND deleted_at = (SELECT deleted_at FROM {table} WHERE id = ?)"
            ),
            vec![text(id), text(id)],
        ));
    }
    plan.push(Statement::new(
        format!("UPDATE {table} SET deleted_at = NULL, deleted_by = NULL WHERE id = ? AND deleted_at IS NOT NULL"),
        vec![text(id)],
    ));
    plan.push(audit_statement(table, id, "RESTORE", restored_by, &stamp(now)));
    Ok(plan)
}

/// First instant that is old enough to purge.
pub fn purge_cutoff(now: u64) -> u64 {
    now.saturating_sub(RETENTION_DAYS * SECS_PER_DAY)
}

/// The purge job for one table: audit, then hard-delete rows soft-deleted
/// before the cutoff. Run for every soft-deleting table, then
/// [`purge_audit_trail`] stays untouched.
pub fn plan_purge(table: &str, now: u64) -> Result<Vec<Statement>, SoftDeleteError> {
    let table = ident(table)?;
    let cutoff = stamp(purge_cutoff(now));
    let at = stamp(now);
    Ok(vec![
        Statement::new(
            format!(
                "INSERT INTO deletion_audit (table_name, record_id, action, actor, occurred_at) \
                 SELECT '{table}', id, 'PURGE', 'purge-job', ? FROM {table} \
                 WHERE deleted_at IS NOT NULL AND deleted_at < ?"
            ),
            vec![text(&at), text(&cutoff)],
        ),
        Statement::new(
            format!("DELETE FROM {table} WHERE deleted_at IS NOT NULL AND deleted_at < ?"),
            vec![text(&cutoff)],
        ),
    ])
}

/// The audit table is deliberately never purged; this exists so the intent is
/// explicit and testable.
pub fn purge_audit_trail() -> Vec<Statement> {
    Vec::new()
}

fn audit_statement(table: &str, id: &str, action: &str, actor: &str, at: &str) -> Statement {
    Statement::new(
        "INSERT INTO deletion_audit (table_name, record_id, action, actor, occurred_at) VALUES (?, ?, ?, ?, ?)".to_string(),
        vec![text(table), text(id), text(action), text(actor), text(at)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-01-31 00:00:00 UTC
    const NOW: u64 = 1_769_817_600;

    fn rels() -> Vec<Relationship> {
        vec![
            Relationship::new("policies", "vault_id", DeletePolicy::Cascade),
            Relationship::new("fee_records", "vault_id", DeletePolicy::Nullify),
        ]
    }

    #[test]
    fn scopes_filter_deleted_rows_and_only_admins_can_widen_them() {
        assert_eq!(scope_clause(None, Scope::Active).unwrap().as_deref(), Some("deleted_at IS NULL"));
        assert_eq!(scope_clause(Some("v"), Scope::OnlyDeleted).unwrap().as_deref(), Some("v.deleted_at IS NOT NULL"));
        assert_eq!(scope_clause(None, Scope::WithDeleted).unwrap(), None);

        assert_eq!(scope_for(Scope::WithDeleted, false), Scope::Active);
        assert_eq!(scope_for(Scope::OnlyDeleted, false), Scope::Active);
        assert_eq!(scope_for(Scope::WithDeleted, true), Scope::WithDeleted);
    }

    #[test]
    fn identifiers_are_validated_so_sql_cannot_be_injected_through_them() {
        for bad in ["", "vaults; DROP TABLE x", "a b", "1abc", "a-b", "t\"x"] {
            assert!(matches!(plan_delete(bad, "1", "u", NOW, &[]), Err(SoftDeleteError::InvalidIdentifier(_))), "{bad}");
        }
        assert!(scope_clause(Some("v;--"), Scope::Active).is_err());
        let hostile = [Relationship::new("policies", "id; DROP TABLE x", DeletePolicy::Cascade)];
        assert!(plan_delete("vaults", "1", "u", NOW, &hostile).is_err());
    }

    #[test]
    fn delete_marks_the_row_applies_each_policy_and_audits() {
        let plan = plan_delete("vaults", "v1", "admin-7", NOW, &rels()).unwrap();
        let sql: Vec<&str> = plan.iter().map(|s| s.sql.as_str()).collect();

        assert_eq!(plan.len(), 4);
        assert!(sql[0].starts_with("UPDATE policies SET deleted_at = ?, deleted_by = ? WHERE vault_id = ? AND deleted_at IS NULL"));
        assert_eq!(sql[1], "UPDATE fee_records SET vault_id = NULL WHERE vault_id = ?");
        assert_eq!(sql[2], "UPDATE vaults SET deleted_at = ?, deleted_by = ? WHERE id = ? AND deleted_at IS NULL");
        assert!(sql[3].starts_with("INSERT INTO deletion_audit"));

        assert_eq!(plan[2].params, vec![text("2026-01-31 00:00:00"), text("admin-7"), text("v1")]);
        assert_eq!(plan[3].params[2], text("DELETE"));
        assert!(plan.iter().all(|s| !s.must_be_zero));
    }

    #[test]
    fn restrict_adds_a_guard_the_executor_must_see_return_zero() {
        let rels = [Relationship::new("positions", "vault_id", DeletePolicy::Restrict)];
        let plan = plan_delete("vaults", "v1", "u", NOW, &rels).unwrap();

        assert!(plan[0].must_be_zero);
        assert_eq!(plan[0].sql, "SELECT COUNT(*) FROM positions WHERE vault_id = ? AND deleted_at IS NULL");
        assert_eq!(plan[0].params, vec![text("v1")]);
    }

    #[test]
    fn restore_is_admin_only_and_brings_back_only_the_children_cascaded_with_the_parent() {
        assert_eq!(plan_restore("vaults", "v1", "u", false, NOW, &rels()), Err(SoftDeleteError::Forbidden));

        let plan = plan_restore("vaults", "v1", "admin-7", true, NOW, &rels()).unwrap();
        assert_eq!(plan.len(), 3, "one cascade child, the parent, the audit row (nullify is not undone)");
        assert!(plan[0].sql.contains("deleted_at = (SELECT deleted_at FROM vaults WHERE id = ?)"));
        assert!(plan[1].sql.starts_with("UPDATE vaults SET deleted_at = NULL, deleted_by = NULL WHERE id = ? AND deleted_at IS NOT NULL"));
        assert_eq!(plan[2].params[2], text("RESTORE"));
        // Children must be restored before the parent's deleted_at disappears.
        assert!(plan[0].sql.contains("policies") && plan[1].sql.contains("vaults"));
    }

    #[test]
    fn purge_covers_the_30_day_retention_window() {
        assert_eq!(RETENTION_DAYS, 30);
        assert_eq!(purge_cutoff(NOW), NOW - 30 * 86_400);

        let plan = plan_purge("vaults", NOW).unwrap();
        assert_eq!(plan.len(), 2);
        assert!(plan[0].sql.starts_with("INSERT INTO deletion_audit"), "audit is written before the rows go");
        assert!(plan[0].sql.contains("'PURGE'"));
        assert_eq!(plan[1].sql, "DELETE FROM vaults WHERE deleted_at IS NOT NULL AND deleted_at < ?");
        assert_eq!(plan[1].params, vec![text("2026-01-01 00:00:00")]);
    }

    #[test]
    fn the_audit_trail_is_never_purged() {
        assert!(purge_audit_trail().is_empty());
        for statement in plan_purge("vaults", NOW).unwrap() {
            assert!(!statement.sql.starts_with("DELETE FROM deletion_audit"));
        }
    }
}
