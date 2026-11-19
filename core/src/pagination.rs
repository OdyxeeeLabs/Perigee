//! # Pagination standard
//!
//! One pagination contract for every list endpoint (see `docs/PAGINATION.md`).
//!
//! * **Cursor based.** Clients pass an opaque `cursor`; the server never exposes
//!   offsets. Cursors are HMAC-signed, so a client cannot forge or edit one, and
//!   they are bound to the sort and filters they were issued for.
//! * **Response shape.** `{ "data": [...], "pagination": { "next_cursor",
//!   "has_more", "total_count"? } }`. `total_count` is only computed when the
//!   client asks for it with `include_total=true`, because counting is expensive.
//! * **Sorting** is an explicit `sort` parameter checked against a per-endpoint
//!   allowlist (`sort=created_at` ascending, `sort=-created_at` descending).
//! * **Filtering** uses structured `filter[name]=value` parameters, also against
//!   an allowlist.
//! * **Page size** defaults to 25 and is at most 100; larger values are rejected
//!   rather than silently clamped.
//!
//! Rows are ordered by `(sort field, id)` so ties never skip or repeat a row.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use thiserror::Error;

pub const DEFAULT_PAGE_SIZE: usize = 25;
pub const MAX_PAGE_SIZE: usize = 100;

/// Codec used by the list handlers. The secret comes from
/// `PAGINATION_CURSOR_SECRET`; when unset a random per-process secret is used, so
/// cursors are never forgeable but do not survive a restart.
pub fn cursor_codec() -> &'static CursorCodec {
    static CODEC: std::sync::OnceLock<CursorCodec> = std::sync::OnceLock::new();
    CODEC.get_or_init(|| match std::env::var("PAGINATION_CURSOR_SECRET") {
        Ok(secret) if !secret.is_empty() => CursorCodec::new(secret.as_bytes()),
        _ => {
            let mut secret = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut secret);
            CursorCodec::new(&secret)
        }
    })
}

impl From<PaginationError> for crate::errors::AppError {
    fn from(err: PaginationError) -> Self {
        crate::errors::AppError::BadRequest(format!("{}: {err}", err.code()))
    }
}

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Asc,
    Desc,
}

/// What a list endpoint allows. Declare one per endpoint.
#[derive(Debug, Clone, Copy)]
pub struct EndpointSpec {
    pub name: &'static str,
    pub sortable: &'static [&'static str],
    pub default_sort: &'static str,
    pub default_direction: Direction,
    pub filterable: &'static [&'static str],
}

/// Pagination behaviour of the existing list endpoints, for the docs and for
/// handlers migrating to cursors.
pub const VAULTS_LIST: EndpointSpec = EndpointSpec {
    name: "GET /vaults",
    sortable: &["created_at", "name"],
    default_sort: "created_at",
    default_direction: Direction::Desc,
    filterable: &["status"],
};

pub const MANAGERS_LIST: EndpointSpec = EndpointSpec {
    name: "GET /managers",
    sortable: &["created_at"],
    default_sort: "created_at",
    default_direction: Direction::Desc,
    filterable: &["status"],
};

pub const FEE_HISTORY: EndpointSpec = EndpointSpec {
    name: "GET /fees/history",
    sortable: &["ledger"],
    default_sort: "ledger",
    default_direction: Direction::Desc,
    filterable: &[],
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PaginationError {
    #[error("limit must be a whole number between 1 and {MAX_PAGE_SIZE}")]
    InvalidLimit,
    #[error("limit may not exceed {MAX_PAGE_SIZE}")]
    LimitTooLarge,
    #[error("cannot sort by `{field}`; allowed: {allowed}")]
    InvalidSort { field: String, allowed: String },
    #[error("cannot filter by `{0}`")]
    InvalidFilter(String),
    #[error("malformed or tampered cursor")]
    InvalidCursor,
    #[error("cursor was issued for a different sort or filter")]
    CursorMismatch,
}

impl PaginationError {
    /// Stable machine-readable code for the error body.
    pub fn code(&self) -> &'static str {
        match self {
            PaginationError::InvalidLimit => "invalid_limit",
            PaginationError::LimitTooLarge => "limit_too_large",
            PaginationError::InvalidSort { .. } => "invalid_sort",
            PaginationError::InvalidFilter(_) => "invalid_filter",
            PaginationError::InvalidCursor => "invalid_cursor",
            PaginationError::CursorMismatch => "cursor_mismatch",
        }
    }
}

/// Where the previous page ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub value: String,
    pub id: String,
}

#[derive(Serialize, Deserialize)]
struct CursorPayload {
    sort: String,
    dir: Direction,
    filters: String,
    position: Position,
}

/// Signs and verifies cursors.
#[derive(Clone)]
pub struct CursorCodec {
    secret: Vec<u8>,
}

impl CursorCodec {
    pub fn new(secret: &[u8]) -> Self {
        CursorCodec {
            secret: secret.to_vec(),
        }
    }

    fn mac(&self, data: &[u8]) -> Result<HmacSha256, PaginationError> {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).map_err(|_| PaginationError::InvalidCursor)?;
        mac.update(data);
        Ok(mac)
    }

    fn encode(&self, payload: &CursorPayload) -> Result<String, PaginationError> {
        let json = serde_json::to_vec(payload).map_err(|_| PaginationError::InvalidCursor)?;
        let body = URL_SAFE_NO_PAD.encode(&json);
        let tag = self.mac(body.as_bytes())?.finalize().into_bytes();
        Ok(format!("{body}.{}", URL_SAFE_NO_PAD.encode(tag)))
    }

    fn decode(&self, token: &str) -> Result<CursorPayload, PaginationError> {
        let (body, tag) = token.split_once('.').ok_or(PaginationError::InvalidCursor)?;
        let tag = URL_SAFE_NO_PAD
            .decode(tag)
            .map_err(|_| PaginationError::InvalidCursor)?;
        // `verify_slice` compares in constant time.
        self.mac(body.as_bytes())?
            .verify_slice(&tag)
            .map_err(|_| PaginationError::InvalidCursor)?;
        let json = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| PaginationError::InvalidCursor)?;
        serde_json::from_slice(&json).map_err(|_| PaginationError::InvalidCursor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    pub field: String,
    pub direction: Direction,
}

/// A validated list request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: usize,
    pub sort: Sort,
    pub filters: BTreeMap<String, String>,
    pub after: Option<Position>,
    pub include_total: bool,
}

fn filter_fingerprint(filters: &BTreeMap<String, String>) -> String {
    filters
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

impl PageRequest {
    /// Validate raw query parameters against `spec`.
    pub fn parse(
        spec: &EndpointSpec,
        codec: &CursorCodec,
        params: &[(String, String)],
    ) -> Result<PageRequest, PaginationError> {
        let mut limit = DEFAULT_PAGE_SIZE;
        let mut sort = Sort {
            field: spec.default_sort.to_string(),
            direction: spec.default_direction,
        };
        let mut filters = BTreeMap::new();
        let mut cursor = None;
        let mut include_total = false;

        for (name, value) in params {
            match name.as_str() {
                "limit" => {
                    let parsed: usize = value.parse().map_err(|_| PaginationError::InvalidLimit)?;
                    if parsed == 0 {
                        return Err(PaginationError::InvalidLimit);
                    }
                    if parsed > MAX_PAGE_SIZE {
                        return Err(PaginationError::LimitTooLarge);
                    }
                    limit = parsed;
                }
                "cursor" => cursor = Some(value.as_str()),
                "include_total" => include_total = value == "true",
                "sort" => {
                    let (field, direction) = match value.strip_prefix('-') {
                        Some(rest) => (rest, Direction::Desc),
                        None => (value.as_str(), Direction::Asc),
                    };
                    if !spec.sortable.contains(&field) {
                        return Err(PaginationError::InvalidSort {
                            field: field.to_string(),
                            allowed: spec.sortable.join(", "),
                        });
                    }
                    sort = Sort {
                        field: field.to_string(),
                        direction,
                    };
                }
                other => {
                    if let Some(key) = other
                        .strip_prefix("filter[")
                        .and_then(|rest| rest.strip_suffix(']'))
                    {
                        if !spec.filterable.contains(&key) {
                            return Err(PaginationError::InvalidFilter(key.to_string()));
                        }
                        filters.insert(key.to_string(), value.clone());
                    }
                }
            }
        }

        let after = match cursor {
            None => None,
            Some(token) => {
                let payload = codec.decode(token)?;
                if payload.sort != sort.field
                    || payload.dir != sort.direction
                    || payload.filters != filter_fingerprint(&filters)
                {
                    return Err(PaginationError::CursorMismatch);
                }
                Some(payload.position)
            }
        };

        Ok(PageRequest {
            limit,
            sort,
            filters,
            after,
            include_total,
        })
    }

    /// How many rows to fetch: one extra tells us whether another page exists.
    pub fn fetch_limit(&self) -> usize {
        self.limit + 1
    }

    /// `ORDER BY` fragment. The column comes from the endpoint allowlist, and is
    /// re-checked here so it can never carry SQL.
    pub fn order_by(&self) -> Option<String> {
        if !is_identifier(&self.sort.field) {
            return None;
        }
        let dir = match self.sort.direction {
            Direction::Asc => "ASC",
            Direction::Desc => "DESC",
        };
        Some(format!("ORDER BY {col} {dir}, id {dir}", col = self.sort.field))
    }

    /// Keyset predicate with two bind values (`value`, `id`), or `None` for the
    /// first page.
    pub fn keyset_predicate(&self) -> Option<(String, [String; 2])> {
        let after = self.after.as_ref()?;
        if !is_identifier(&self.sort.field) {
            return None;
        }
        let op = match self.sort.direction {
            Direction::Asc => ">",
            Direction::Desc => "<",
        };
        Some((
            format!("({}, id) {op} (?, ?)", self.sort.field),
            [after.value.clone(), after.id.clone()],
        ))
    }
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pagination {
    pub next_cursor: Option<String>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub pagination: Pagination,
}

/// Turn the rows fetched with [`PageRequest::fetch_limit`] into a response.
/// `position` returns the `(sort value, id)` of a row, which becomes the cursor.
pub fn build_page<T>(
    mut rows: Vec<T>,
    request: &PageRequest,
    codec: &CursorCodec,
    position: impl Fn(&T) -> (String, String),
    total_count: Option<u64>,
) -> Result<Page<T>, PaginationError> {
    let has_more = rows.len() > request.limit;
    rows.truncate(request.limit);

    let next_cursor = match (has_more, rows.last()) {
        (true, Some(last)) => {
            let (value, id) = position(last);
            Some(codec.encode(&CursorPayload {
                sort: request.sort.field.clone(),
                dir: request.sort.direction,
                filters: filter_fingerprint(&request.filters),
                position: Position { value, id },
            })?)
        }
        _ => None,
    };

    Ok(Page {
        data: rows,
        pagination: Pagination {
            next_cursor,
            has_more,
            total_count: if request.include_total { total_count } else { None },
        },
    })
}

/// Paginate an in-memory list. Values compare as strings, so numeric sort keys
/// must be zero-padded and timestamps ISO-8601.
pub fn paginate_slice<T: Clone>(
    items: &[T],
    request: &PageRequest,
    codec: &CursorCodec,
    position: impl Fn(&T) -> (String, String),
) -> Result<Page<T>, PaginationError> {
    let mut sorted: Vec<&T> = items.iter().collect();
    sorted.sort_by_cached_key(|item| position(item));
    if request.sort.direction == Direction::Desc {
        sorted.reverse();
    }

    let rows: Vec<T> = sorted
        .into_iter()
        .filter(|item| match &request.after {
            None => true,
            Some(after) => {
                let current = position(item);
                let boundary = (after.value.clone(), after.id.clone());
                match request.sort.direction {
                    Direction::Asc => current > boundary,
                    Direction::Desc => current < boundary,
                }
            }
        })
        .take(request.fetch_limit())
        .cloned()
        .collect();

    build_page(rows, request, codec, &position, Some(items.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: EndpointSpec = EndpointSpec {
        name: "GET /things",
        sortable: &["created_at", "name"],
        default_sort: "created_at",
        default_direction: Direction::Desc,
        filterable: &["status"],
    };

    fn codec() -> CursorCodec {
        CursorCodec::new(b"test-secret")
    }

    fn params(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
    struct Row {
        id: String,
        created_at: String,
    }

    fn position(row: &Row) -> (String, String) {
        (row.created_at.clone(), row.id.clone())
    }

    fn rows(n: usize) -> Vec<Row> {
        (0..n)
            .map(|i| Row {
                id: format!("id{i:03}"),
                created_at: format!("2026-01-{:02}", (i / 2) + 1), // ties: two rows per day
            })
            .collect()
    }

    #[test]
    fn defaults_apply_when_nothing_is_given() {
        let req = PageRequest::parse(&SPEC, &codec(), &[]).unwrap();
        assert_eq!(req.limit, DEFAULT_PAGE_SIZE);
        assert_eq!(req.sort, Sort { field: "created_at".into(), direction: Direction::Desc });
        assert!(req.after.is_none());
        assert!(!req.include_total);
    }

    #[test]
    fn limit_is_validated() {
        for (value, expected) in [
            ("0", PaginationError::InvalidLimit),
            ("-1", PaginationError::InvalidLimit),
            ("abc", PaginationError::InvalidLimit),
            ("101", PaginationError::LimitTooLarge),
        ] {
            let err = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", value)])).unwrap_err();
            assert_eq!(err, expected, "{value}");
        }
        assert_eq!(
            PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "100")])).unwrap().limit,
            100
        );
    }

    #[test]
    fn sort_is_checked_against_the_allowlist() {
        let ok = PageRequest::parse(&SPEC, &codec(), &params(&[("sort", "-name")])).unwrap();
        assert_eq!(ok.sort, Sort { field: "name".into(), direction: Direction::Desc });

        let err = PageRequest::parse(&SPEC, &codec(), &params(&[("sort", "password_hash")])).unwrap_err();
        assert_eq!(err.code(), "invalid_sort");
        let injection = PageRequest::parse(&SPEC, &codec(), &params(&[("sort", "name; DROP TABLE x")])).unwrap_err();
        assert_eq!(injection.code(), "invalid_sort");
    }

    #[test]
    fn filters_use_the_structured_form_and_an_allowlist() {
        let ok = PageRequest::parse(&SPEC, &codec(), &params(&[("filter[status]", "active")])).unwrap();
        assert_eq!(ok.filters.get("status").map(String::as_str), Some("active"));

        let err = PageRequest::parse(&SPEC, &codec(), &params(&[("filter[secret]", "x")])).unwrap_err();
        assert_eq!(err, PaginationError::InvalidFilter("secret".into()));
    }

    #[test]
    fn walking_every_page_returns_each_row_exactly_once_even_with_ties() {
        let items = rows(23);
        let codec = codec();
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut query = vec![("limit", "5")];
            if let Some(token) = &cursor {
                query.push(("cursor", token.as_str()));
            }
            let req = PageRequest::parse(&SPEC, &codec, &params(&query)).unwrap();
            let page = paginate_slice(&items, &req, &codec, position).unwrap();
            seen.extend(page.data.iter().map(|r| r.id.clone()));
            match page.pagination.next_cursor {
                Some(next) => {
                    assert!(page.pagination.has_more);
                    cursor = Some(next);
                }
                None => {
                    assert!(!page.pagination.has_more);
                    break;
                }
            }
        }

        assert_eq!(seen.len(), 23);
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 23, "no row repeated");
        // Default order is newest first, ties broken by id descending.
        assert_eq!(seen.first().map(String::as_str), Some("id022"));
        assert_eq!(seen.last().map(String::as_str), Some("id000"));
    }

    #[test]
    fn the_last_page_has_no_cursor() {
        let items = rows(3);
        let req = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "5")])).unwrap();
        let page = paginate_slice(&items, &req, &codec(), position).unwrap();
        assert_eq!(page.data.len(), 3);
        assert_eq!(page.pagination.next_cursor, None);
        assert!(!page.pagination.has_more);
    }

    #[test]
    fn total_count_is_only_present_when_requested() {
        let items = rows(10);
        let without = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "2")])).unwrap();
        let with = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "2"), ("include_total", "true")])).unwrap();

        let a = paginate_slice(&items, &without, &codec(), position).unwrap();
        let b = paginate_slice(&items, &with, &codec(), position).unwrap();

        assert_eq!(a.pagination.total_count, None);
        assert_eq!(b.pagination.total_count, Some(10));
        let json = serde_json::to_value(&a).unwrap();
        assert!(json["pagination"].get("total_count").is_none());
        assert!(json["pagination"]["next_cursor"].is_string());
    }

    #[test]
    fn tampered_or_foreign_cursors_are_rejected() {
        let items = rows(10);
        let req = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "2")])).unwrap();
        let token = paginate_slice(&items, &req, &codec(), position)
            .unwrap()
            .pagination
            .next_cursor
            .unwrap();

        let mut tampered = token.clone();
        tampered.insert(3, 'x');
        for bad in [tampered.as_str(), "garbage", "a.b", ""] {
            let err = PageRequest::parse(&SPEC, &codec(), &params(&[("cursor", bad)])).unwrap_err();
            assert_eq!(err, PaginationError::InvalidCursor, "{bad:?}");
        }

        let other = CursorCodec::new(b"another-secret");
        let err = PageRequest::parse(&SPEC, &other, &params(&[("cursor", &token)])).unwrap_err();
        assert_eq!(err, PaginationError::InvalidCursor);
    }

    #[test]
    fn a_cursor_cannot_be_replayed_with_a_different_sort_or_filter() {
        let items = rows(10);
        let req = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "2")])).unwrap();
        let token = paginate_slice(&items, &req, &codec(), position)
            .unwrap()
            .pagination
            .next_cursor
            .unwrap();

        let other_sort = PageRequest::parse(&SPEC, &codec(), &params(&[("cursor", &token), ("sort", "name")])).unwrap_err();
        let other_filter = PageRequest::parse(
            &SPEC,
            &codec(),
            &params(&[("cursor", &token), ("filter[status]", "active")]),
        )
        .unwrap_err();

        assert_eq!(other_sort, PaginationError::CursorMismatch);
        assert_eq!(other_filter, PaginationError::CursorMismatch);
    }

    #[test]
    fn sql_fragments_use_only_allowlisted_columns() {
        let items = rows(10);
        let req = PageRequest::parse(&SPEC, &codec(), &params(&[("limit", "2")])).unwrap();
        assert_eq!(req.order_by().as_deref(), Some("ORDER BY created_at DESC, id DESC"));
        assert!(req.keyset_predicate().is_none());

        let token = paginate_slice(&items, &req, &codec(), position).unwrap().pagination.next_cursor.unwrap();
        let next = PageRequest::parse(&SPEC, &codec(), &params(&[("cursor", &token)])).unwrap();
        let (sql, binds) = next.keyset_predicate().unwrap();
        assert_eq!(sql, "(created_at, id) < (?, ?)");
        assert_eq!(binds[1], "id008");

        let mut hostile = next.clone();
        hostile.sort.field = "created_at; DROP TABLE vaults".into();
        assert!(hostile.order_by().is_none());
        assert!(hostile.keyset_predicate().is_none());
    }

    #[test]
    fn existing_list_endpoints_declare_a_spec() {
        for spec in [VAULTS_LIST, MANAGERS_LIST, FEE_HISTORY] {
            assert!(spec.sortable.contains(&spec.default_sort), "{}", spec.name);
        }
    }
}
