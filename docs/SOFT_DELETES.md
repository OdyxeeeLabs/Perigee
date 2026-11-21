# Soft deletes

`core/src/soft_delete.rs` and migration `009_soft_delete_framework.sql` add recoverable
deletion to any table with `deleted_at` and `deleted_by` columns (`vaults` and `managers`
have them).

## Behaviour

* **Default scope** excludes deleted rows (`deleted_at IS NULL`). Administrators can request
  `Scope::WithDeleted` or `Scope::OnlyDeleted`; `scope_for` gives everyone else `Active`
  regardless of what they ask for.
* **Delete** (`plan_delete`) marks the row and applies each `Relationship` policy:
  `Restrict` (refuse while live children exist: run the returned guard query and abort if
  the count is not zero), `Cascade` (soft-delete the children too) or `Nullify` (clear the
  children's foreign key).
* **Restore** (`plan_restore`, admin only) clears the markers on the row and on the children
  that were cascaded by that same delete (they share its `deleted_at`). Nullified children
  are not re-attached.
* **Purge** (`plan_purge`) hard-deletes rows soft-deleted more than 30 days ago
  (`RETENTION_DAYS`) for GDPR erasure. It writes a `PURGE` row to `deletion_audit` first.
* **Audit.** Delete, restore and purge are recorded in `deletion_audit` (table, record id,
  action, actor, time). It holds ids only, and is never purged.

## Running a plan

The planner returns parameterised statements. Run them in order inside one transaction.
Identifiers are validated, values are bound; nothing from a request is interpolated into SQL.

```rust
let plan = plan_delete("vaults", &id, &admin_id, now, &[
    Relationship::new("policies", "vault_id", DeletePolicy::Cascade),
    Relationship::new("positions", "vault_id", DeletePolicy::Restrict),
])?;
for statement in plan { /* execute; if statement.must_be_zero and the count != 0, roll back and return 409 */ }
```

Schedule `plan_purge` daily for every soft-deleting table.
