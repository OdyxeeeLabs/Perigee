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

## GDPR

* Hot and warm data is erased with the soft-delete purge (`SOFT_DELETES.md`).
* Cold data: `ArchiveManifest::gdpr_rewrite_tasks(subject)` lists the archived objects that
  contain the subject. A worker rewrites each without that subject's rows and calls
  `mark_rewritten`, which updates the row count and checksum.
