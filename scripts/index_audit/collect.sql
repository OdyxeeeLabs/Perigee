-- Collects the statistics the index audit needs, as one JSON document on stdout.
--
--   psql "$AUDIT_DATABASE_URL" -At -f scripts/index_audit/collect.sql > snapshot.json
--
-- Run it against a read replica. Requires the pg_stat_statements extension
-- (`CREATE EXTENSION pg_stat_statements;`, with it in shared_preload_libraries).
SELECT json_build_object(
  'tables', (
    SELECT COALESCE(json_agg(json_build_object(
      'table', relname,
      'seq_scan', COALESCE(seq_scan, 0),
      'idx_scan', COALESCE(idx_scan, 0),
      'n_live_tup', n_live_tup)), '[]'::json)
    FROM pg_stat_user_tables
  ),
  'indexes', (
    SELECT COALESCE(json_agg(json_build_object(
      'table', s.relname,
      'index', s.indexrelname,
      'idx_scan', s.idx_scan,
      'size_bytes', pg_relation_size(s.indexrelid),
      'definition', i.indexdef,
      'is_unique', x.indisunique,
      'is_primary', x.indisprimary)), '[]'::json)
    FROM pg_stat_user_indexes s
    JOIN pg_indexes i ON i.schemaname = s.schemaname AND i.indexname = s.indexrelname
    JOIN pg_index x ON x.indexrelid = s.indexrelid
  ),
  'statements', (
    SELECT COALESCE(json_agg(json_build_object(
      'query', query, 'calls', calls, 'mean_ms', mean_exec_time)), '[]'::json)
    FROM (
      SELECT query, calls, mean_exec_time
      FROM pg_stat_statements
      ORDER BY total_exec_time DESC
      LIMIT 200
    ) top
  )
);
