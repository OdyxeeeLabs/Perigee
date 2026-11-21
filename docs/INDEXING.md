# Indexing strategy and audit

## Weekly audit

`.github/workflows/index-audit.yml` runs every Monday (and on demand) against a **read
replica** (`AUDIT_DATABASE_URL` secret; the job is skipped when it is unset):

1. `scripts/index_audit/collect.sql` reads `pg_stat_user_tables`, `pg_stat_user_indexes`,
   `pg_indexes` and `pg_stat_statements` into one JSON snapshot.
2. `index-audit` (`core/src/index_audit.rs`) analyses it and writes a Markdown report, attached
   to the run and shown in its summary.

Requirements: the `pg_stat_statements` extension on the replica; optionally `hypopg` for the
suggested checks.

## What it reports

* **Possibly missing indexes**: tables of at least 10,000 rows with at least 100 sequential
  scans, mostly scanned sequentially. The slow statements that read the table are used to infer
  filter columns, and the report gives a `hypopg` check and a `CREATE INDEX CONCURRENTLY`.
  Columns an existing index already leads with are not suggested again.
* **Unused indexes**: `idx_scan = 0`, at least 1 MiB, and neither unique nor primary.
  `idx_scan` counts since the last statistics reset, so confirm the replica has been up through a
  full business cycle (including month end) before dropping anything.
* **Redundant indexes**: the key columns are a leading prefix of another index on the same table
  with the same `WHERE` predicate, or an exact duplicate.

## Making changes safely

1. Apply the suggested change on a **replica** (or a restored copy) first and compare `EXPLAIN
   (ANALYZE)` for the affected queries before and after.
2. Use `CONCURRENTLY` for every `CREATE INDEX` and `DROP INDEX` in production so writes are not
   blocked; it cannot run inside a transaction block.
3. Ship the change as a migration, with the query it serves in a comment.
4. For a drop, keep the `CREATE INDEX` statement (from `pg_indexes.indexdef`) in the migration's
   down path so it can be rebuilt.

## Conventions

* Every foreign key gets an index on the referencing column.
* List endpoints use keyset pagination (`PAGINATION.md`), which wants a composite index on
  `(<sort column>, id)`.
* Soft-deleting tables index `deleted_at` (see `SOFT_DELETES.md`); consider a partial index
  `WHERE deleted_at IS NULL` for hot tables.
