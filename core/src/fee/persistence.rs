//! Fee history persistence (#669) — SQL-backed store for historical queries.

use super::service::FeeRecordView;

/// In-memory stand-in used by unit tests and local dev.
#[derive(Default)]
pub struct FeeHistoryStore {
    rows: Vec<FeeRecordView>,
}

impl FeeHistoryStore {
    pub fn insert(&mut self, row: FeeRecordView) {
        self.rows.push(row);
    }

    pub fn by_seq(&self, seq: u64) -> Option<&FeeRecordView> {
        self.rows.iter().find(|r| r.seq == seq)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn all(&self) -> &[FeeRecordView] {
        &self.rows
    }
}
