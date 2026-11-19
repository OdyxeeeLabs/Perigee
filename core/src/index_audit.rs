//! # Database index audit
//!
//! Turns PostgreSQL statistics into an index report with ready-to-review SQL.
//! The statistics come from `scripts/index_audit/collect.sql` (it reads
//! `pg_stat_user_tables`, `pg_stat_user_indexes`, `pg_indexes` and, when the
//! extension is installed, `pg_stat_statements`) and are deserialised into
//! [`Snapshot`]. [`audit`] then finds:
//!
//! * **Missing indexes** – large tables read mostly by sequential scan, with the
//!   slowest statements that touch them and the columns those statements filter
//!   on. A `hypopg` check is generated for each suggestion so the planner can
//!   confirm the index would be used before anything is built.
//! * **Unused indexes** – never scanned since the statistics were reset, and
//!   neither unique nor primary (those enforce constraints).
//! * **Redundant indexes** – the key columns are a leading prefix of another
//!   index on the same table with the same predicate, or an exact duplicate.
//!
//! Nothing here runs SQL. [`render_markdown`] produces the weekly report; apply
//! its `CREATE`/`DROP` statements on a replica first and in production with
//! `CONCURRENTLY`. See `docs/INDEXING.md`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct TableStats {
    pub table: String,
    pub seq_scan: u64,
    pub idx_scan: u64,
    pub n_live_tup: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexStats {
    pub table: String,
    pub index: String,
    pub idx_scan: u64,
    pub size_bytes: u64,
    /// Output of `pg_get_indexdef`.
    pub definition: String,
    pub is_unique: bool,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatementStats {
    pub query: String,
    pub calls: u64,
    pub mean_ms: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Snapshot {
    pub tables: Vec<TableStats>,
    pub indexes: Vec<IndexStats>,
    #[serde(default)]
    pub statements: Vec<StatementStats>,
}

#[derive(Debug, Clone)]
pub struct Thresholds {
    /// Ignore tables smaller than this when looking for missing indexes.
    pub min_rows: u64,
    /// A table is "scanned sequentially" when at least this share of its scans are.
    pub seq_scan_ratio: f64,
    pub min_seq_scans: u64,
    /// Do not bother reporting unused indexes smaller than this.
    pub min_unused_bytes: u64,
    /// Statements slower than this (mean) are candidates for index suggestions.
    pub slow_ms: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            min_rows: 10_000,
            seq_scan_ratio: 0.5,
            min_seq_scans: 100,
            min_unused_bytes: 1024 * 1024,
            slow_ms: 50.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MissingIndex {
    pub table: String,
    pub seq_scan: u64,
    pub idx_scan: u64,
    pub rows: u64,
    /// Columns worth indexing, from the slow statements' `WHERE` clauses.
    pub columns: Vec<String>,
    pub example_query: Option<String>,
    pub create_sql: Option<String>,
    pub hypopg_check: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnusedIndex {
    pub table: String,
    pub index: String,
    pub size_bytes: u64,
    pub drop_sql: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedundantIndex {
    pub table: String,
    pub index: String,
    pub covered_by: String,
    pub size_bytes: u64,
    pub drop_sql: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Report {
    pub missing: Vec<MissingIndex>,
    pub unused: Vec<UnusedIndex>,
    pub redundant: Vec<RedundantIndex>,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.unused.is_empty() && self.redundant.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Key columns and `WHERE` predicate from a `CREATE INDEX` definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIndex {
    pub columns: Vec<String>,
    pub predicate: Option<String>,
    pub unique: bool,
}

/// Parse `CREATE [UNIQUE] INDEX name ON public.t USING btree (a, b DESC) WHERE x`.
pub fn parse_index_definition(definition: &str) -> Option<ParsedIndex> {
    let lower = definition.to_lowercase();
    let unique = lower.starts_with("create unique");
    let using = lower.find(" using ")?;
    let open = lower[using..].find('(')? + using;

    let mut depth = 0usize;
    let mut close = None;
    for (offset, c) in definition[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;

    let columns: Vec<String> = definition[open + 1..close]
        .split(',')
        .map(|c| c.split_whitespace().next().unwrap_or("").trim_matches('"').to_string())
        .filter(|c| !c.is_empty())
        .collect();

    let rest = &lower[close + 1..];
    let predicate = rest.find(" where ").map(|i| rest[i + 7..].trim().to_string());
    Some(ParsedIndex { columns, predicate, unique })
}

/// Columns a statement filters on: identifiers directly left of a comparison
/// operator, `IN`, `LIKE` or `BETWEEN` in its `WHERE` clause.
pub fn where_columns(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    let Some(start) = lower.find(" where ") else { return Vec::new() };
    let mut end = lower.len();
    for stop in [" group by ", " order by ", " limit ", " having ", " returning "] {
        if let Some(i) = lower[start..].find(stop) {
            end = end.min(start + i);
        }
    }
    let clause = &lower[start + 7..end];

    let mut columns: Vec<String> = Vec::new();
    let tokens: Vec<&str> = clause.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        let operator = matches!(*token, "=" | "<" | ">" | "<=" | ">=" | "<>" | "!=" | "in" | "like" | "ilike" | "between" | "is");
        if operator && i > 0 {
            let candidate = tokens[i - 1].trim_matches(|c| c == '(' || c == ')' || c == '"');
            let name = candidate.rsplit('.').next().unwrap_or(candidate);
            let valid = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
                && !matches!(name, "and" | "or" | "not" | "null");
            if valid && !columns.contains(&name.to_string()) {
                columns.push(name.to_string());
            }
        }
    }
    columns
}

fn mentions_table(query: &str, table: &str) -> bool {
    let lower = query.to_lowercase();
    [" from ", " join ", " update ", " into "].iter().any(|kw| {
        lower.match_indices(kw).any(|(i, _)| {
            let after = lower[i + kw.len()..].trim_start();
            let word: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '"')
                .collect();
            word.trim_matches('"').rsplit('.').next() == Some(table)
        })
    })
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Analyse a statistics snapshot.
pub fn audit(snapshot: &Snapshot, thresholds: &Thresholds) -> Report {
    let mut report = Report::default();

    // Missing indexes.
    for table in &snapshot.tables {
        let total = table.seq_scan + table.idx_scan;
        if table.n_live_tup < thresholds.min_rows
            || table.seq_scan < thresholds.min_seq_scans
            || total == 0
            || (table.seq_scan as f64 / total as f64) < thresholds.seq_scan_ratio
        {
            continue;
        }

        let mut slow: Vec<&StatementStats> = snapshot
            .statements
            .iter()
            .filter(|s| s.mean_ms >= thresholds.slow_ms && mentions_table(&s.query, &table.table))
            .collect();
        slow.sort_by(|a, b| (b.mean_ms * b.calls as f64).total_cmp(&(a.mean_ms * a.calls as f64)));

        let mut columns: Vec<String> = Vec::new();
        for statement in &slow {
            for column in where_columns(&statement.query) {
                if !columns.contains(&column) {
                    columns.push(column);
                }
            }
        }
        // Columns an existing index already leads with need no new index.
        let existing: Vec<ParsedIndex> = snapshot
            .indexes
            .iter()
            .filter(|i| i.table == table.table)
            .filter_map(|i| parse_index_definition(&i.definition))
            .collect();
        columns.retain(|c| !existing.iter().any(|e| e.predicate.is_none() && e.columns.first() == Some(c)));
        columns.truncate(3);

        let (create_sql, hypopg_check) = if columns.is_empty() {
            (None, None)
        } else {
            let cols = columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
            let name = format!("idx_{}_{}", table.table, columns.join("_"));
            let create = format!("CREATE INDEX CONCURRENTLY IF NOT EXISTS {} ON {} ({cols});", quote(&name), quote(&table.table));
            let check = format!(
                "SELECT * FROM hypopg_create_index('CREATE INDEX ON {} ({cols})'); EXPLAIN (GENERIC_PLAN) {};",
                quote(&table.table),
                slow.first().map(|s| s.query.as_str()).unwrap_or("<query>")
            );
            (Some(create), Some(check))
        };

        report.missing.push(MissingIndex {
            table: table.table.clone(),
            seq_scan: table.seq_scan,
            idx_scan: table.idx_scan,
            rows: table.n_live_tup,
            columns,
            example_query: slow.first().map(|s| s.query.clone()),
            create_sql,
            hypopg_check,
        });
    }

    // Unused indexes.
    for index in &snapshot.indexes {
        if index.idx_scan == 0 && !index.is_unique && !index.is_primary && index.size_bytes >= thresholds.min_unused_bytes {
            report.unused.push(UnusedIndex {
                table: index.table.clone(),
                index: index.index.clone(),
                size_bytes: index.size_bytes,
                drop_sql: format!("DROP INDEX CONCURRENTLY IF EXISTS {};", quote(&index.index)),
            });
        }
    }

    // Redundant indexes.
    let parsed: Vec<(&IndexStats, ParsedIndex)> = snapshot
        .indexes
        .iter()
        .filter_map(|i| parse_index_definition(&i.definition).map(|p| (i, p)))
        .collect();
    for (index, p) in &parsed {
        if index.is_primary || index.is_unique {
            continue; // they enforce a constraint
        }
        let cover = parsed.iter().find(|(other, q)| {
            other.index != index.index
                && other.table == index.table
                && q.predicate == p.predicate
                && q.columns.len() >= p.columns.len()
                && q.columns[..p.columns.len()] == p.columns[..]
                // For an exact duplicate, keep the one that sorts first.
                && (q.columns.len() > p.columns.len() || other.index < index.index)
        });
        if let Some((other, _)) = cover {
            report.redundant.push(RedundantIndex {
                table: index.table.clone(),
                index: index.index.clone(),
                covered_by: other.index.clone(),
                size_bytes: index.size_bytes,
                drop_sql: format!("DROP INDEX CONCURRENTLY IF EXISTS {};", quote(&index.index)),
            });
        }
    }

    // An index that is both unused and redundant is reported once, as redundant.
    report.unused.retain(|u| !report.redundant.iter().any(|r| r.index == u.index));
    report
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The weekly report.
pub fn render_markdown(report: &Report, generated_on: &str) -> String {
    let mut out = format!("# Index audit - {generated_on}\n\n");
    if report.is_clean() {
        out.push_str("No missing, unused or redundant indexes found.\n");
        return out;
    }
    out.push_str("Test every change on a replica first (see docs/INDEXING.md).\n\n");

    out.push_str(&format!("## Possibly missing indexes ({})\n\n", report.missing.len()));
    for m in &report.missing {
        out.push_str(&format!("### `{}`\n\n{} sequential scans vs {} index scans on ~{} rows.\n\n", m.table, m.seq_scan, m.idx_scan, m.rows));
        match (&m.create_sql, &m.hypopg_check) {
            (Some(create), Some(check)) => {
                out.push_str(&format!("Check with hypopg first:\n\n```sql\n{check}\n```\n\nThen:\n\n```sql\n{create}\n```\n\n"));
            }
            _ => out.push_str("No filtered columns could be inferred; review the slow statements for this table.\n\n"),
        }
    }

    out.push_str(&format!("## Unused indexes ({})\n\n", report.unused.len()));
    for u in &report.unused {
        out.push_str(&format!("* `{}` on `{}` ({}) - never scanned\n\n  ```sql\n  {}\n  ```\n", u.index, u.table, human(u.size_bytes), u.drop_sql));
    }

    out.push_str(&format!("\n## Redundant indexes ({})\n\n", report.redundant.len()));
    for r in &report.redundant {
        out.push_str(&format!("* `{}` on `{}` ({}) is covered by `{}`\n\n  ```sql\n  {}\n  ```\n", r.index, r.table, human(r.size_bytes), r.covered_by, r.drop_sql));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(name: &str, seq: u64, idx: u64, rows: u64) -> TableStats {
        TableStats { table: name.into(), seq_scan: seq, idx_scan: idx, n_live_tup: rows }
    }

    fn index(name: &str, table: &str, scans: u64, definition: &str) -> IndexStats {
        IndexStats {
            table: table.into(),
            index: name.into(),
            idx_scan: scans,
            size_bytes: 8 * 1024 * 1024,
            definition: definition.into(),
            is_unique: definition.contains("UNIQUE"),
            is_primary: name.ends_with("_pkey"),
        }
    }

    fn statement(query: &str, calls: u64, mean_ms: f64) -> StatementStats {
        StatementStats { query: query.into(), calls, mean_ms }
    }

    #[test]
    fn index_definitions_are_parsed() {
        let p = parse_index_definition("CREATE UNIQUE INDEX idx ON public.vaults USING btree (manager_id, \"created_at\" DESC) WHERE (deleted_at IS NULL)").unwrap();
        assert_eq!(p.columns, vec!["manager_id", "created_at"]);
        assert!(p.unique);
        assert_eq!(p.predicate.as_deref(), Some("(deleted_at is null)"));

        let plain = parse_index_definition("CREATE INDEX i ON public.t USING btree (lower((email)::text))").unwrap();
        assert_eq!(plain.columns.len(), 1);
        assert_eq!(plain.predicate, None);
        assert!(parse_index_definition("not an index").is_none());
    }

    #[test]
    fn filter_columns_come_from_the_where_clause() {
        assert_eq!(
            where_columns("SELECT * FROM vaults v WHERE v.manager_id = $1 AND status IN ($2, $3) AND created_at > $4 ORDER BY created_at LIMIT 10"),
            vec!["manager_id", "status", "created_at"]
        );
        assert_eq!(where_columns("SELECT * FROM vaults"), Vec::<String>::new());
        assert_eq!(where_columns("SELECT 1 FROM t WHERE a = 1 GROUP BY b HAVING count(*) > 2"), vec!["a"]);
    }

    #[test]
    fn a_large_sequentially_scanned_table_gets_a_suggestion_with_a_hypopg_check() {
        let snapshot = Snapshot {
            tables: vec![table("vaults", 5_000, 100, 200_000), table("tiny", 9_000, 0, 50), table("healthy", 10, 9_000, 500_000)],
            indexes: vec![index("vaults_pkey", "vaults", 40, "CREATE UNIQUE INDEX vaults_pkey ON public.vaults USING btree (id)")],
            statements: vec![
                statement("SELECT * FROM vaults WHERE manager_id = $1 AND status = $2", 10_000, 120.0),
                statement("SELECT * FROM vaults WHERE id = $1", 50_000, 0.1),
                statement("SELECT * FROM healthy WHERE a = $1", 10, 900.0),
            ],
        };

        let report = audit(&snapshot, &Thresholds::default());

        assert_eq!(report.missing.len(), 1, "small and well-indexed tables are skipped");
        let missing = &report.missing[0];
        assert_eq!(missing.table, "vaults");
        assert_eq!(missing.columns, vec!["manager_id", "status"]);
        assert_eq!(
            missing.create_sql.as_deref(),
            Some("CREATE INDEX CONCURRENTLY IF NOT EXISTS \"idx_vaults_manager_id_status\" ON \"vaults\" (\"manager_id\", \"status\");")
        );
        let check = missing.hypopg_check.as_deref().unwrap();
        assert!(check.contains("hypopg_create_index") && check.contains("EXPLAIN (GENERIC_PLAN) SELECT * FROM vaults WHERE manager_id"));
    }

    #[test]
    fn columns_an_existing_index_already_leads_with_are_not_suggested_again() {
        let snapshot = Snapshot {
            tables: vec![table("vaults", 5_000, 100, 200_000)],
            indexes: vec![index("idx_manager", "vaults", 5, "CREATE INDEX idx_manager ON public.vaults USING btree (manager_id)")],
            statements: vec![statement("SELECT * FROM vaults WHERE manager_id = $1 AND status = $2", 100, 200.0)],
        };
        let report = audit(&snapshot, &Thresholds::default());
        assert_eq!(report.missing[0].columns, vec!["status"]);
    }

    #[test]
    fn a_seq_scan_heavy_table_without_slow_statements_is_flagged_without_sql() {
        let snapshot = Snapshot { tables: vec![table("events", 5_000, 0, 1_000_000)], ..Default::default() };
        let report = audit(&snapshot, &Thresholds::default());
        assert_eq!(report.missing[0].create_sql, None);
        assert!(render_markdown(&report, "2026-10-19").contains("No filtered columns could be inferred"));
    }

    #[test]
    fn unused_indexes_skip_constraints_and_tiny_indexes() {
        let mut tiny = index("idx_tiny", "t", 0, "CREATE INDEX idx_tiny ON public.t USING btree (x)");
        tiny.size_bytes = 8 * 1024;
        let snapshot = Snapshot {
            tables: vec![],
            indexes: vec![
                index("idx_never", "t", 0, "CREATE INDEX idx_never ON public.t USING btree (a)"),
                index("idx_used", "t", 12, "CREATE INDEX idx_used ON public.t USING btree (b)"),
                index("t_uniq", "t", 0, "CREATE UNIQUE INDEX t_uniq ON public.t USING btree (c)"),
                index("t_pkey", "t", 0, "CREATE UNIQUE INDEX t_pkey ON public.t USING btree (id)"),
                tiny,
            ],
            statements: vec![],
        };
        let report = audit(&snapshot, &Thresholds::default());
        assert_eq!(report.unused.len(), 1);
        assert_eq!(report.unused[0].index, "idx_never");
        assert_eq!(report.unused[0].drop_sql, "DROP INDEX CONCURRENTLY IF EXISTS \"idx_never\";");
    }

    #[test]
    fn prefix_and_duplicate_indexes_are_redundant_but_constraints_and_partial_indexes_are_not() {
        let snapshot = Snapshot {
            tables: vec![],
            indexes: vec![
                index("idx_a", "t", 9, "CREATE INDEX idx_a ON public.t USING btree (a)"),
                index("idx_ab", "t", 9, "CREATE INDEX idx_ab ON public.t USING btree (a, b)"),
                index("idx_dup1", "t", 9, "CREATE INDEX idx_dup1 ON public.t USING btree (z)"),
                index("idx_dup2", "t", 9, "CREATE INDEX idx_dup2 ON public.t USING btree (z)"),
                index("idx_partial", "t", 9, "CREATE INDEX idx_partial ON public.t USING btree (a) WHERE (deleted_at IS NULL)"),
                index("idx_other_table", "u", 9, "CREATE INDEX idx_other_table ON public.u USING btree (a)"),
                index("t_a_unique", "t", 9, "CREATE UNIQUE INDEX t_a_unique ON public.t USING btree (a)"),
            ],
            statements: vec![],
        };
        let report = audit(&snapshot, &Thresholds::default());
        let redundant: Vec<(&str, &str)> = report.redundant.iter().map(|r| (r.index.as_str(), r.covered_by.as_str())).collect();

        assert!(redundant.contains(&("idx_a", "idx_ab")));
        assert!(redundant.contains(&("idx_dup2", "idx_dup1")), "one of two duplicates stays");
        assert!(!redundant.iter().any(|(i, _)| *i == "idx_dup1"));
        assert!(!redundant.iter().any(|(i, _)| *i == "idx_partial"), "different predicate");
        assert!(!redundant.iter().any(|(i, _)| *i == "idx_other_table"), "different table");
        assert!(!redundant.iter().any(|(i, _)| *i == "t_a_unique"), "enforces a constraint");
    }

    #[test]
    fn an_unused_index_that_is_also_redundant_is_reported_once() {
        let snapshot = Snapshot {
            tables: vec![],
            indexes: vec![
                index("idx_a", "t", 0, "CREATE INDEX idx_a ON public.t USING btree (a)"),
                index("idx_ab", "t", 7, "CREATE INDEX idx_ab ON public.t USING btree (a, b)"),
            ],
            statements: vec![],
        };
        let report = audit(&snapshot, &Thresholds::default());
        assert_eq!((report.unused.len(), report.redundant.len()), (0, 1));
    }

    #[test]
    fn the_markdown_report_lists_every_finding_with_its_sql() {
        assert!(render_markdown(&Report::default(), "2026-10-19").contains("No missing, unused or redundant indexes"));

        let snapshot = Snapshot {
            tables: vec![table("vaults", 5_000, 100, 200_000)],
            indexes: vec![
                index("idx_never", "vaults", 0, "CREATE INDEX idx_never ON public.vaults USING btree (a)"),
                index("idx_x", "vaults", 3, "CREATE INDEX idx_x ON public.vaults USING btree (x)"),
                index("idx_xy", "vaults", 3, "CREATE INDEX idx_xy ON public.vaults USING btree (x, y)"),
            ],
            statements: vec![statement("SELECT * FROM vaults WHERE manager_id = $1", 100, 200.0)],
        };
        let markdown = render_markdown(&audit(&snapshot, &Thresholds::default()), "2026-10-19");

        assert!(markdown.starts_with("# Index audit - 2026-10-19"));
        for expected in ["Possibly missing indexes (1)", "CREATE INDEX CONCURRENTLY", "Unused indexes (1)", "8.0 MiB", "Redundant indexes (1)", "covered by `idx_xy`", "replica first"] {
            assert!(markdown.contains(expected), "missing {expected:?} in\n{markdown}");
        }
    }

    #[test]
    fn snapshots_deserialize_from_the_collection_script_output() {
        let snapshot: Snapshot = serde_json::from_str(
            r#"{"tables":[{"table":"t","seq_scan":1,"idx_scan":2,"n_live_tup":3}],
                "indexes":[{"table":"t","index":"i","idx_scan":0,"size_bytes":10,"definition":"CREATE INDEX i ON public.t USING btree (a)","is_unique":false,"is_primary":false}]}"#,
        )
        .unwrap();
        assert_eq!((snapshot.tables.len(), snapshot.indexes.len(), snapshot.statements.len()), (1, 1, 0));
    }
}
