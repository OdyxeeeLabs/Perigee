# Agent health summary for managers

Managers can check whether the agent assigned to each of their vaults is
working, without seeing operator-only fleet internals. Implemented in
`core/src/agent_health_summary.rs`.

| Method | Path | Returns |
| --- | --- | --- |
| `GET` | `/vaults/:id/agent-health` | Summary for one vault. |
| `GET` | `/agents/health` | `{"vaults": [...]}` for all of the caller's vaults (max 200, ordered by vault id). |

## Response

```json
{
  "vault_id": "vault-1",
  "agent_id": "agent-7",
  "status": "degraded",
  "last_heartbeat_at": "2026-11-19T08:25:00Z",
  "heartbeat_age_secs": 300,
  "fresh_within_secs": 120,
  "offline_after_secs": 600,
  "guidance": "The last heartbeat was 300 seconds ago (expected within 120 seconds). Actions may be delayed; check the agent's connectivity.",
  "checked_at": "2026-11-19T08:30:00Z"
}
```

### Status values

| Status | Meaning | `guidance` |
| --- | --- | --- |
| `active` | Heartbeat no older than `fresh_within_secs` and the fleet attests the agent is healthy. | `null` |
| `degraded` | Heartbeat is late (older than `fresh_within_secs`, not older than `offline_after_secs`), **or** fresh but the agent failed fleet health/identity attestation. | What to check |
| `offline` | No heartbeat for more than `offline_after_secs`. Automated actions are paused. | Restart or request reassignment |
| `unassigned` | No agent is assigned to the vault. `agent_id` is `null`. | Assign an agent |
| `unknown` | Health data is unavailable: no heartbeat has been recorded for the assigned agent. | Retry; contact support if it persists |

The set is stable: new internal states map onto one of these values.

## Freshness

* Defaults: fresh within **120 s**, offline after **600 s**. Both thresholds are
  returned on every summary so clients do not hard-code them.
* Thresholds are inclusive: a heartbeat exactly 120 s old is still fresh.
* `heartbeat_age_secs` is measured at `checked_at`. A heartbeat stamped in the
  future (clock skew) counts as age 0. An older heartbeat never replaces a
  newer one.

## Unavailable data

* No heartbeat recorded yet → `status: "unknown"`, `last_heartbeat_at` and
  `heartbeat_age_secs` are `null`. The endpoint still answers `200`.
* If the fleet cannot attest an agent (for example the agent is not in the
  directory), attestation is treated as unknown and the status follows
  heartbeat freshness alone.

## Privacy and tenant boundaries

* Summaries never include peer attestations, reputation scores, failover
  history, host details, or the diagnostic payload agents report. Those stay
  operator-only.
* A vault owned by another manager, or outside a scoped token's vaults, returns
  `404 vault not found`, the same as a missing vault. Administrators can read
  any vault's summary.
