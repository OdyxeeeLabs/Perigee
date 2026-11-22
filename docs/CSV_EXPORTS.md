# Tenant-scoped CSV report exports (API-21)

White-label managers can download fee-reconciliation reports as CSV for
offline reconciliation. The export is read-only and scoped to the authenticated
manager's Stellar address; the `tenant_id` query parameter is deliberately not
accepted, so one manager can never request another manager's rows.

## Endpoint

```http
GET /reports/fee-reconciliation.csv?limit=50
Authorization: Bearer <jwt>
Accept: text/csv
```

| Parameter | Default | Allowed | Meaning |
| --- | --- | --- | --- |
| `limit` | `50` | `1..=200` | Number of most recent audit reports to consider. Values outside the range are clamped. |

Successful responses use:

```http
HTTP/1.1 200 OK
Content-Type: text/csv; charset=utf-8
Content-Disposition: attachment; filename="perigee-fee-reconciliation-reports.csv"
Cache-Control: no-store
X-Content-Type-Options: nosniff
```

Errors use the standard Perigee RFC 7807 error body. In particular, exports
that exceed the limits below return `400 Bad Request` with
`ErrorCode::InvalidInput`.

## Stable field order

The header line is emitted exactly once, with CRLF line endings, in this order:

| # | Column | Type | Notes |
| ---: | --- | --- | --- |
| 1 | `report_id` | text | Report UUID. |
| 2 | `tenant_id` | text | Always equals the authenticated manager's Stellar address. |
| 3 | `actor_id` | text | Identity that created the report. |
| 4 | `from_ledger` | integer | Inclusive ledger range start. |
| 5 | `to_ledger` | integer | Inclusive ledger range end. |
| 6 | `total_ledgers` | integer | Ledgers inspected. |
| 7 | `matched_records` | integer | Records present in both sources. |
| 8 | `status` | text | `clean` or `discrepancies_found`. |
| 9 | `discrepancy_count` | integer | Number of discrepancies in this report. |
| 10 | `created_at_utc` | UTC timestamp | RFC 3339, microseconds, `Z` suffix. |
| 11 | `ledger_sequence` | integer | Discrepancy ledger; blank for clean reports. |
| 12 | `discrepancy_kind` | text | `missing_fee_history`, `missing_ledger_event`, or `conflicting_record`. |
| 13 | `discrepancy_kind_code` | integer | Stable numeric code emitted by the fee engine. |
| 14 | `vault` | text | Untrusted vault identifier; formula-guarded. |
| 15 | `payer` | text | Untrusted payer identifier; formula-guarded. |
| 16 | `fee_history_gross_stroops` | integer | Exact `i128` decimal string. |
| 17 | `fee_history_discount_stroops` | integer | Exact `i128` decimal string. |
| 18 | `fee_history_net_stroops` | integer | Exact `i128` decimal string. |
| 19 | `fee_history_vault_share_stroops` | integer | Exact `i128` decimal string. |
| 20 | `fee_history_protocol_share_stroops` | integer | Exact `i128` decimal string. |
| 21 | `fee_history_timestamp_utc` | UTC timestamp | RFC 3339, microseconds, `Z` suffix. |
| 22 | `fee_history_kind` | integer | Fee kind code. |
| 23 | `ledger_event_gross_stroops` | integer | Exact `i128` decimal string. |
| 24 | `ledger_event_discount_stroops` | integer | Exact `i128` decimal string. |
| 25 | `ledger_event_net_stroops` | integer | Exact `i128` decimal string. |
| 26 | `ledger_event_vault_share_stroops` | integer | Exact `i128` decimal string. |
| 27 | `ledger_event_protocol_share_stroops` | integer | Exact `i128` decimal string. |
| 28 | `ledger_event_timestamp_utc` | UTC timestamp | RFC 3339, microseconds, `Z` suffix. |
| 29 | `ledger_event_kind` | integer | Fee kind code. |

This order is a compatibility contract. Additive fields, if ever needed, are
appended only, and their arrival is announced in the API changelog.

### Row shape

- Clean reports produce one row with blank discrepancy and record columns.
- Reports with discrepancies produce one row per fee-history/ledger-event
  record pair. Missing sides are blank.
- A tenant with no reports receives the header line only.

## Sample file

```csv
report_id,tenant_id,actor_id,from_ledger,to_ledger,total_ledgers,matched_records,status,discrepancy_count,created_at_utc,ledger_sequence,discrepancy_kind,discrepancy_kind_code,vault,payer,fee_history_gross_stroops,fee_history_discount_stroops,fee_history_net_stroops,fee_history_vault_share_stroops,fee_history_protocol_share_stroops,fee_history_timestamp_utc,fee_history_kind,ledger_event_gross_stroops,ledger_event_discount_stroops,ledger_event_net_stroops,ledger_event_vault_share_stroops,ledger_event_protocol_share_stroops,ledger_event_timestamp_utc,ledger_event_kind
rpt-001,GLOADTESTADMIN0000000000000000000000000000000000000000000000,operator-a,1000,1009,10,9,discrepancies_found,1,2026-01-02T03:04:05.000000Z,1007,conflicting_record,2,vault-7,payer-9,120,20,100,90,10,2023-11-14T22:13:20.000000Z,3,130,20,110,99,11,2023-11-14T22:13:20.000000Z,3
```

## Formatting guarantees

- **Quoting:** fields containing commas, double quotes, CR, or LF are wrapped
  in double quotes; embedded double quotes are doubled (RFC 4180).
- **Formula safety:** any text field whose first non-whitespace character is
  `=`, `+`, `-`, `@`, tab, or CR is prefixed with a literal apostrophe before
  CSV quoting. For example, `=SUM(1,2)` becomes `"'=SUM(1,2)"`.
- **Integers:** `i128` stroop amounts and all counters are rendered with
  `Display`, preserving exact decimal digits. No locale grouping, no floating
  point, no exponent notation.
- **Timestamps:** every timestamp is UTC RFC 3339 with exactly six fractional
  digits and a `Z` suffix, independent of server or browser locale.

## Size limits

| Limit | Value | Behavior |
| --- | ---: | --- |
| Reports considered per request | 200 | `limit` is clamped to `1..=200`. |
| CSV data rows per response | 10,000 | Request fails with `400` if exceeded. |
| Response body | 5 MiB | Request fails with `400` if exceeded. |

For larger reconciliation histories, page by time by requesting a smaller
`limit` and archiving each response; the audit store is append-only, so
previously exported reports are immutable.
