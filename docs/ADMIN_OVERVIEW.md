# Admin overview (`GET /admin/overview`)

A bounded, read-only snapshot of queue, outbox and dependency health for
operators. Implemented in `core/src/admin_overview.rs`.

## Access

* Requires a valid bearer token **and** the administrator permission
  (`role: admin`, or an address listed in `PERIGEE_ADMIN_STELLAR_ADDRESSES`).
* Managers, operators and viewers receive `403` with an
  `application/problem+json` body. Nothing about the components is returned.
* Responses carry `Cache-Control: no-store`.
* The endpoint only reads; probes never mutate the observed stores.

## Response

```json
{
  "generated_at": "2026-11-19T08:30:15.120Z",
  "overall_status": "degraded",
  "components": [
    {
      "name": "dependency:database",
      "kind": "dependency",
      "status": "healthy",
      "counts": { "latency_ms": 2 },
      "oldest_item_at": null,
      "checked_at": "2026-11-19T08:30:15.118Z",
      "timeout_ms": 2000
    },
    {
      "name": "dependency:stellar_rpc",
      "kind": "dependency",
      "status": "timeout",
      "counts": {},
      "oldest_item_at": null,
      "checked_at": "2026-11-19T08:30:17.119Z",
      "timeout_ms": 2000,
      "detail": "no response within 2000 ms"
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `generated_at` | When the snapshot was assembled (UTC RFC 3339). |
| `overall_status` | `healthy` when every component is healthy, otherwise `degraded`. |
| `components[].name` | `queue:<name>`, `outbox`, or `dependency:<name>`. |
| `components[].kind` | `queue`, `outbox` or `dependency`. |
| `components[].status` | `healthy`, `degraded`, `unavailable` (the probe reported a failure) or `timeout` (no answer within `timeout_ms`). |
| `components[].counts` | Aggregate integers; only the keys below ever appear. |
| `components[].oldest_item_at` | Creation time of the oldest outstanding job or event, or `null`. |
| `components[].checked_at` | When this component's probe finished. |
| `components[].timeout_ms` | The probe budget, so a `timeout` is actionable. |
| `components[].detail` | Fixed text for `timeout` / `unavailable`. Raw probe errors go to the server log only. |

### Documented count keys

| Kind | Keys |
| --- | --- |
| `queue` | `waiting`, `delayed`, `active`, `completed`, `dead`, `throughput_per_min` |
| `outbox` | `pending`, `published`, `quarantined` |
| `dependency` | `latency_ms`, `healthy_instances`, `total_instances` |

Any other key a probe reports is dropped before the response is built. The
response never contains job or event payloads, record identifiers, addresses,
or connection strings.

## Bounds and timeouts

* Each probe runs concurrently with its own budget (default 2 s,
  `admin_overview::DEFAULT_PROBE_TIMEOUT`); one slow dependency cannot stall the
  request beyond that budget.
* At most 32 components (`MAX_COMPONENTS`) are registered.

## Registered components

`core/src/main.rs` registers `dependency:database`, `dependency:job_queue` and
`dependency:stellar_rpc`. `JobQueueProbe` and `OutboxProbe` report the
in-memory `job_queue::JobQueue` and `outbox::InMemoryOutbox` aggregates and
are registered with `AdminOverviewService::with_probe` wherever those stores
are constructed.
