# API style guide

Conventions every public REST payload follows. Error bodies are covered in
[`OPENAPI.md`](./OPENAPI.md) (RFC 7807 problem details) and error codes in
[`ERROR_VARIANTS.md`](./ERROR_VARIANTS.md).

## Timestamps

All public timestamp fields are **UTC** and serialised as **RFC 3339** strings
with an explicit `Z` designator.

| Rule | Example |
| --- | --- |
| Whole seconds carry no fraction | `2026-11-19T08:30:15Z` |
| Stored sub-second precision is kept, in groups of 3 digits | `2026-11-19T08:30:15.250Z`, `2026-11-19T08:30:15.123456Z`, `2026-11-19T08:30:15.123456789Z` |
| Values are never rounded or truncated | `…15.999999999Z` stays in second `15` |
| Absent optional timestamps are `null` | `"revoked_at": null` |

Clients may send timestamps with any explicit offset; the server converts them
to UTC (`2026-11-19T10:30:15+02:00` is stored and returned as
`2026-11-19T08:30:15Z`). A timestamp **without** an offset
(`2026-11-19T08:30:15`) or a bare epoch number is rejected with `400`, because
the instant it names is ambiguous.

Numeric ledger fields such as `ledger_sequence` are **not** timestamps: they
stay JSON integers and are never rewritten as dates.

Field names end in `_at` (`created_at`, `last_used_at`, `checked_at`).

### Implementation

Use `core/src/timestamps.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct ExampleResponse {
    #[serde(with = "crate::timestamps::rfc3339_utc")]
    created_at: DateTime<Utc>,
    #[serde(with = "crate::timestamps::rfc3339_utc_option", default)]
    revoked_at: Option<DateTime<Utc>>,
    ledger_sequence: u32,
}
```

* Type new timestamp fields as `chrono::DateTime<chrono::Utc>` and create
  values with `chrono::Utc::now()`.
* `timestamps::format_rfc3339_utc` / `timestamps::parse_rfc3339_utc` cover
  values that are built or read outside serde.
* `timestamps::from_unix_seconds` converts a legacy `u64` epoch field while it
  is being migrated to `DateTime<Utc>`.
