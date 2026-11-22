//! Tenant-scoped CSV exports for fee-reconciliation reports (API-21, #782).
//!
//! The header order in [`REPORT_CSV_HEADERS`] is a public contract: never
//! reorder, rename, or remove a column. Text fields are escaped per RFC 4180
//! and prefixed with `'` when they could be interpreted as a spreadsheet
//! formula. Integers are rendered with `Display`, so `i128` amounts keep every
//! digit and never become floating-point/scientific notation. Timestamps are
//! emitted as UTC RFC 3339 with microsecond precision.

use crate::fee::reconciliation::{
    FeeDiscrepancyKind, FeeReconciliationReport, FeeReconciliationStatus,
};
use crate::fee::service::FeeRecordView;
use chrono::{DateTime, SecondsFormat, Utc};
use std::fmt::Display;

/// Stable CSV header order. This array is the canonical field list.
pub const REPORT_CSV_HEADERS: [&str; 29] = [
    "report_id",
    "tenant_id",
    "actor_id",
    "from_ledger",
    "to_ledger",
    "total_ledgers",
    "matched_records",
    "status",
    "discrepancy_count",
    "created_at_utc",
    "ledger_sequence",
    "discrepancy_kind",
    "discrepancy_kind_code",
    "vault",
    "payer",
    "fee_history_gross_stroops",
    "fee_history_discount_stroops",
    "fee_history_net_stroops",
    "fee_history_vault_share_stroops",
    "fee_history_protocol_share_stroops",
    "fee_history_timestamp_utc",
    "fee_history_kind",
    "ledger_event_gross_stroops",
    "ledger_event_discount_stroops",
    "ledger_event_net_stroops",
    "ledger_event_vault_share_stroops",
    "ledger_event_protocol_share_stroops",
    "ledger_event_timestamp_utc",
    "ledger_event_kind",
];

/// Maximum number of reports a single export request may consider.
pub const MAX_EXPORT_REPORTS: usize = 200;
/// Maximum number of CSV data rows in a response.
pub const MAX_EXPORT_ROWS: usize = 10_000;
/// Maximum response body size (5 MiB).
pub const MAX_EXPORT_BYTES: usize = 5 * 1024 * 1024;

/// Rendered CSV plus the number of data rows written (excluding the header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportCsv {
    pub body: String,
    pub rows: usize,
}

fn text_field(value: &str) -> String {
    // Guard both literal leading control characters and formula characters
    // hidden behind leading whitespace.
    let first = value.chars().next();
    let first_non_ws = value
        .trim_start_matches(|c: char| c.is_whitespace())
        .chars()
        .next();
    let needs_guard = matches!(first, Some('=' | '+' | '-' | '@' | '\t' | '\r'))
        || matches!(first_non_ws, Some('=' | '+' | '-' | '@'));
    let guarded = if needs_guard {
        format!("'{value}")
    } else {
        value.to_string()
    };

    if guarded.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn numeric_field<T: Display>(value: T) -> String {
    value.to_string()
}

fn utc_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn unix_timestamp_utc(seconds: u64) -> String {
    let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(utc_timestamp)
        .unwrap_or_default()
}

fn status_str(status: FeeReconciliationStatus) -> &'static str {
    match status {
        FeeReconciliationStatus::Clean => "clean",
        FeeReconciliationStatus::DiscrepanciesFound => "discrepancies_found",
    }
}

fn kind_str(kind: FeeDiscrepancyKind) -> &'static str {
    match kind {
        FeeDiscrepancyKind::MissingFeeHistory => "missing_fee_history",
        FeeDiscrepancyKind::MissingLedgerEvent => "missing_ledger_event",
        FeeDiscrepancyKind::ConflictingRecord => "conflicting_record",
    }
}

fn push_record_fields(record: Option<&FeeRecordView>, row: &mut Vec<String>) {
    match record {
        Some(record) => {
            row.push(numeric_field(record.gross));
            row.push(numeric_field(record.discount));
            row.push(numeric_field(record.net));
            row.push(numeric_field(record.vault_share));
            row.push(numeric_field(record.protocol_share));
            row.push(unix_timestamp_utc(record.timestamp));
            row.push(numeric_field(record.kind));
        }
        None => {
            for _ in 0..7 {
                row.push(String::new());
            }
        }
    }
}

fn push_report_row(report: &FeeReconciliationReport, row: &mut Vec<String>) {
    row.push(text_field(&report.id));
    row.push(text_field(&report.tenant_id));
    row.push(text_field(&report.actor_id));
    row.push(numeric_field(report.from_ledger));
    row.push(numeric_field(report.to_ledger));
    row.push(numeric_field(report.total_ledgers));
    row.push(numeric_field(report.matched_records));
    row.push(text_field(status_str(report.status)));
    row.push(numeric_field(report.discrepancies.len()));
    row.push(utc_timestamp(report.created_at));
}

fn push_discrepancy_row(
    report: &FeeReconciliationReport,
    discrepancy_index: Option<usize>,
    record_index: usize,
    row: &mut Vec<String>,
) {
    push_report_row(report, row);

    match discrepancy_index.and_then(|index| report.discrepancies.get(index)) {
        Some(discrepancy) => {
            row.push(numeric_field(discrepancy.ledger_sequence));
            row.push(text_field(kind_str(discrepancy.discrepancy)));
            row.push(numeric_field(discrepancy.kind));
            row.push(text_field(&discrepancy.vault));
            row.push(text_field(&discrepancy.payer));
            push_record_fields(discrepancy.fee_history.get(record_index), row);
            push_record_fields(discrepancy.ledger_events.get(record_index), row);
        }
        None => {
            for _ in 0..5 {
                row.push(String::new());
            }
            push_record_fields(None, row);
            push_record_fields(None, row);
        }
    }
}

/// Render CSV rows for exactly one tenant. Reports for other tenants are
/// silently skipped — the endpoint must never leak cross-tenant data.
pub fn reports_to_csv(tenant_id: &str, reports: &[FeeReconciliationReport]) -> ReportCsv {
    let mut lines = vec![REPORT_CSV_HEADERS.join(",")];

    for report in reports
        .iter()
        .filter(|report| report.tenant_id == tenant_id)
    {
        if report.discrepancies.is_empty() {
            let mut row = Vec::with_capacity(REPORT_CSV_HEADERS.len());
            push_discrepancy_row(report, None, 0, &mut row);
            lines.push(row.join(","));
            continue;
        }

        for (discrepancy_index, discrepancy) in report.discrepancies.iter().enumerate() {
            let record_pairs = discrepancy
                .fee_history
                .len()
                .max(discrepancy.ledger_events.len())
                .max(1);
            for record_index in 0..record_pairs {
                let mut row = Vec::with_capacity(REPORT_CSV_HEADERS.len());
                push_discrepancy_row(report, Some(discrepancy_index), record_index, &mut row);
                lines.push(row.join(","));
            }
        }
    }

    let rows = lines.len() - 1;
    ReportCsv {
        body: format!("{}\r\n", lines.join("\r\n")),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn record(seq: u64, net: i128, vault: &str, payer: &str) -> FeeRecordView {
        FeeRecordView {
            seq,
            vault: vault.to_string(),
            payer: payer.to_string(),
            gross: net + 20,
            discount: 20,
            net,
            vault_share: net - 10,
            protocol_share: 10,
            timestamp: 1_700_000_000,
            kind: 3,
        }
    }

    fn report(tenant_id: &str, id: &str, vault: &str, payer: &str) -> FeeReconciliationReport {
        FeeReconciliationReport {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            actor_id: "operator-a".to_string(),
            from_ledger: 10,
            to_ledger: 20,
            total_ledgers: 11,
            matched_records: 10,
            status: FeeReconciliationStatus::DiscrepanciesFound,
            discrepancies: vec![crate::fee::reconciliation::FeeReconciliationDiscrepancy {
                ledger_sequence: 17,
                vault: vault.to_string(),
                payer: payer.to_string(),
                kind: 2,
                discrepancy: FeeDiscrepancyKind::ConflictingRecord,
                fee_history: vec![record(17, 9_007_199_254_740_993, vault, payer)],
                ledger_events: vec![record(17, 9_007_199_254_740_992, vault, payer)],
            }],
            created_at: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
        }
    }

    #[test]
    fn headers_are_stable() {
        let expected = "report_id,tenant_id,actor_id,from_ledger,to_ledger,total_ledgers,matched_records,status,discrepancy_count,created_at_utc,ledger_sequence,discrepancy_kind,discrepancy_kind_code,vault,payer,fee_history_gross_stroops,fee_history_discount_stroops,fee_history_net_stroops,fee_history_vault_share_stroops,fee_history_protocol_share_stroops,fee_history_timestamp_utc,fee_history_kind,ledger_event_gross_stroops,ledger_event_discount_stroops,ledger_event_net_stroops,ledger_event_vault_share_stroops,ledger_event_protocol_share_stroops,ledger_event_timestamp_utc,ledger_event_kind";
        assert_eq!(REPORT_CSV_HEADERS.join(","), expected);
    }

    #[test]
    fn text_fields_are_escaped_and_formula_safe() {
        let csv = reports_to_csv(
            "tenant-a",
            &[report(
                "tenant-a",
                "report-1",
                "=SUM(1,2)",
                "payer \"quoted\", with comma\nand newline",
            )],
        );
        let lines: Vec<&str> = csv.body.trim_end().split("\r\n").collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\"'=SUM(1,2)\""));
        assert!(lines[1].contains("\"payer \"\"quoted\"\", with comma\nand newline\""));
    }

    #[test]
    fn text_fields_guard_leading_control_and_whitespace_formulas() {
        let csv = reports_to_csv(
            "tenant-a",
            &[report("tenant-a", "report-1", "\t=SUM(1)", "  -cmd")],
        );
        let line = csv.body.trim_end().split("\r\n").nth(1).unwrap();

        assert!(line.contains("'\t=SUM(1)"));
        assert!(line.contains("'  -cmd"));
    }

    #[test]
    fn integer_amounts_keep_full_i128_precision() {
        let csv = reports_to_csv("tenant-a", &[report("tenant-a", "report-1", "v", "p")]);
        assert!(csv.body.contains("9007199254740993"));
        assert!(csv.body.contains("9007199254740992"));
        assert!(!csv.body.contains("9.007199"));
    }

    #[test]
    fn empty_results_render_headers_only() {
        let csv = reports_to_csv("tenant-a", &[]);
        assert_eq!(csv.rows, 0);
        assert_eq!(csv.body, format!("{}\r\n", REPORT_CSV_HEADERS.join(",")));
    }

    #[test]
    fn tenant_filter_prevents_manager_isolation_leaks() {
        let own = report("tenant-a", "own-report", "vault-a", "payer-a");
        let other = report("tenant-b", "other-report", "vault-b", "payer-b");
        let csv = reports_to_csv("tenant-a", &[other, own]);

        assert_eq!(csv.rows, 1);
        assert!(csv.body.contains("own-report"));
        assert!(!csv.body.contains("other-report"));
        assert!(!csv.body.contains("vault-b"));
        assert!(!csv.body.contains("tenant-b"));
    }

    #[test]
    fn timestamps_are_utc_rfc3339() {
        let csv = reports_to_csv("tenant-a", &[report("tenant-a", "report-1", "v", "p")]);
        assert!(csv.body.contains("2026-01-02T03:04:05.000000Z"));
        assert!(csv.body.contains("2023-11-14T22:13:20.000000Z"));
    }
}
