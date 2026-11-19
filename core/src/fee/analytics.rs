//! Fee analytics integration (#669) — aggregates historical fee records.

use super::service::{FeeKind, FeeRecordView};

#[derive(Debug, Clone, Default)]
pub struct FeeAnalyticsSummary {
    pub count: u64,
    pub total_gross: i128,
    pub total_net: i128,
    pub total_discount: i128,
    pub management_count: u64,
    pub performance_count: u64,
}

/// Aggregate a slice of fee records for dashboards / indexers.
pub fn summarize(records: &[FeeRecordView]) -> FeeAnalyticsSummary {
    let mut s = FeeAnalyticsSummary::default();
    for r in records {
        s.count += 1;
        s.total_gross = s.total_gross.saturating_add(r.gross);
        s.total_net = s.total_net.saturating_add(r.net);
        s.total_discount = s.total_discount.saturating_add(r.discount);
        match r.kind {
            x if x == FeeKind::Management as u32 => s.management_count += 1,
            x if x == FeeKind::Performance as u32 => s.performance_count += 1,
            _ => {}
        }
    }
    s
}

/// Filter records by vault address (case-sensitive).
pub fn filter_by_vault<'a>(
    records: &'a [FeeRecordView],
    vault: &str,
) -> Vec<&'a FeeRecordView> {
    records.iter().filter(|r| r.vault == vault).collect()
}
