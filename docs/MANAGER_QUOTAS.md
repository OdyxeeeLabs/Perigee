# Per-manager API quotas

Authenticated requests are counted against a quota keyed to the
**authenticated manager** and an **operation class**. This sits on top of the
per-IP public limiter (`core/src/rate_limiter.rs`), so managers behind a shared
network no longer throttle each other and a high-volume partner can be given a
larger allowance than a small integration. Implemented in
`core/src/manager_quota.rs`.

## Operation classes

The set is fixed; administrators cannot invent new classes.

| Class | Requests | Default |
| --- | --- | --- |
| `read` | `GET`, `HEAD`, `OPTIONS` | 600 per 60 s |
| `write` | other mutating requests | 120 per 60 s |
| `simulation` | `POST /analyze*`, `POST …/simulate*` | 30 per 60 s |

Defaults can be changed per deployment with
`PERIGEE_QUOTA_DEFAULT_READ`, `PERIGEE_QUOTA_DEFAULT_WRITE` and
`PERIGEE_QUOTA_DEFAULT_SIMULATION`, each formatted `<limit>/<window_secs>`
(for example `1200/60`). Invalid values are logged and ignored.

Administrators are exempt so they can always adjust quotas. Unauthenticated
requests are only subject to the public per-IP limiter.

## Response headers

Every authenticated, non-admin response carries:

| Header | Meaning |
| --- | --- |
| `X-Quota-Limit` | Requests allowed in the current window. |
| `X-Quota-Remaining` | Requests left in the current window. |
| `X-Quota-Reset` | Unix epoch seconds when the window ends. |
| `X-Quota-Class` | `read`, `write` or `simulation`. |
| `X-Quota-Id` | Opaque bucket identifier, `q_<16 hex>_<class>`. |

When the quota is exhausted the API answers `429 Too Many Requests` with an
`application/problem+json` body and `Retry-After: <seconds>`; retrying after
that many seconds succeeds. Clients should honour `Retry-After` rather than
retrying immediately.

`X-Quota-Id` is a truncated SHA-256 of the manager's public address. Bearer
tokens, API keys and other credentials are never used as quota keys and never
appear in quota headers, identifiers or logs.

## Administration

All endpoints require the administrator permission; other callers get `403`.

| Method | Path | Body | Effect |
| --- | --- | --- | --- |
| `GET` | `/admin/quotas/:manager` | – | Effective policy per class, with `overridden` flags. |
| `PUT` | `/admin/quotas/:manager` | `{"class":"write","limit":500,"window_secs":60}` | Install or replace one override. |
| `DELETE` | `/admin/quotas/:manager/:class` | – | Revert one class to its default. |

`limit` must be 1–1,000,000 and `window_secs` 1–86,400; anything else is
rejected with `422` and the previous policy stays in force. Changes are
recorded in the audit log.

### How changes take effect

Overrides are validated and swapped atomically and apply to the very next
request:

* **Usage is not forgiven.** Requests already counted in the current window
  still count, so lowering a limit below current usage denies further
  requests until the window ends, and raising it admits them immediately.
* **Window length follows the current policy.** Shortening the window can end
  the current window early; lengthening it extends the current window rather
  than granting a fresh one.

Counters are held in process memory, so in a multi-instance deployment each
instance enforces the quota independently.
