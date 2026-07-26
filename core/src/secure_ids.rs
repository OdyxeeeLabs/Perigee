use uuid::Uuid;

pub fn generate_reconciliation_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn generate_client_id() -> String {
    format!("cli_{}", Uuid::new_v4().to_string().replace('-', ""))
}

pub struct CursorPagination {
    pub cursor: Option<String>,
    pub limit: usize,
    pub has_more: bool,
}

impl CursorPagination {
    pub fn new(limit: usize) -> Self {
        Self {
            cursor: None,
            limit,
            has_more: false,
        }
    }

    pub fn next_page(last_id: &str) -> Self {
        Self {
            cursor: Some(last_id.to_string()),
            limit: 20,
            has_more: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_reconciliation_id() {
        let id = generate_reconciliation_id();
        assert!(!id.is_empty());
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(id.len(), 36);
    }

    #[test]
    fn test_generate_client_id() {
        let id = generate_client_id();
        assert!(id.starts_with("cli_"));
        assert!(!id.contains('-'));
    }

    #[test]
    fn test_cursor_pagination_new() {
        let p = CursorPagination::new(10);
        assert_eq!(p.limit, 10);
        assert!(p.cursor.is_none());
        assert!(!p.has_more);
    }

    #[test]
    fn test_cursor_pagination_next_page() {
        let p = CursorPagination::next_page("abc123");
        assert_eq!(p.cursor.as_deref(), Some("abc123"));
    }
}
