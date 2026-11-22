# Data archival

`core/src/archival.rs` and migration `010_create_archival_tables.sql` implement tiered storage
so the primary database stays small.

| Tier | Age | Location | Access |
| --- | --- | --- | --- |
| Hot | last 90 days | main tables | normal queries |
| Warm | up to 12 months | monthly partitions on the same cluster | normal queries (partition pruned) |
| Cold | older | Parquet in S3/Glacier | Athena/Trino, or the async retrieval API |

## Partitioning

Large time-series tables are `PARTITION BY RANGE (created_at)` with one partition per month
(`vault_events_2026_10`). `ensure_partitions_sql` creates the current and upcoming months so
inserts never miss a partition; run it from the monthly job.

## Monthly archival job

`plan_monthly_run` lists the partitions that have aged past the warm window and are not yet in
the manifest. For each task:

1. Export the partition to Parquet, one object per tenant and vault, at
   `archive/<table>/tenant=<t>/vault=<v>/year=YYYY/month=MM/part-0000.parquet`.
2. Verify: the uploaded row count and checksum must equal the database's
   (`ArchiveManifest::record_verified` refuses otherwise).
3. Only then `DETACH PARTITION` and `DROP TABLE` the partition.

If verification fails nothing is recorded and the partition is kept.

## Querying cold data

`athena_table_ddl` produces an external table with partition projection over the archive prefix,
so Athena/Trino can prune by tenant, vault and month without catalog maintenance.

For clients, `RetrievalService` provides an asynchronous API:
`POST /archive/retrievals` returns an id; a worker moves it `queued -> running -> ready` and
attaches an expiring download URL (24 hours); `GET /archive/retrievals/{id}` reports the state.
Tenants can only see their own requests.

## Restoring an archived record

Support staff sometimes need to bring one archived record back into the active store, for
example during a customer or reconciliation workflow. `RestoreService::restore` performs a
controlled, auditable restore:

* **Permission.** Only an administrator (`RestoreActor::admin`) may restore. Every other
  principal is rejected with `ArchivalError::NotAuthorized` before any state changes.
* **Reason.** The request must carry a non-empty reason (`ArchivalError::MissingReason`).
* **Existence.** The archive key must be present in the `ArchiveManifest`
  (`ArchivalError::UnknownArchive`).
* **No overwrites of newer state.** If the active store already holds the record at the
  archived version it is a duplicate (`ArchivalError::AlreadyRestored`). If the active version
  is newer, the restore is refused (`ArchivalError::ActiveRecordNewer`) so a rollback can never
  discard a later write. Only strictly older active state is replaced.
* **Audit.** A successful restore appends a `RestoreAuditRecord` with the archive key, table,
  tenant, vault, period, actor, reason, restored version, and timestamp. Denied and rejected
  requests never appear as successful audit entries.

The restored record is written with `version = archived version + 1`, so the next optimistic
update detects the change.

## Retention and restore semantics

Hot and warm data stay queryable until the monthly job archives them. Once a partition is
archived and detached it remains in cold storage for the archive retention period; the
manifest and `/archive/retrievals` API are the source of truth for what can be restored.
Restoring does not delete or mutate the cold object, so a record can be restored again only
after its active copy is removed (or after a later archive supersedes it).

## GDPR

* Hot and warm data is erased with the soft-delete purge (`SOFT_DELETES.md`).
* Cold data: `ArchiveManifest::gdpr_rewrite_tasks(subject)` lists the archived objects that
  contain the subject. A worker rewrites each without that subject's rows and calls
  `mark_rewritten`, which updates the row count and checksum.
