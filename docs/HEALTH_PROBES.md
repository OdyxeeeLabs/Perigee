# Health probes

Use the liveness endpoint to decide whether to restart a process, and the
readiness endpoint to decide whether to send it traffic:

| Endpoint | Use | Dependency checks |
| --- | --- | --- |
| `GET /health/live` | Kubernetes liveness probe; restart only when this fails. | None. It confirms that the process can respond. |
| `GET /health/ready` | Kubernetes readiness probe; route traffic only while this succeeds. | Required database, RPC provider, and agent-fleet state. |

Both endpoints return `200` when their check passes. Readiness returns `503`
when a required dependency is unavailable or its check exceeds the two-second
probe deadline. Its response is intentionally generic and does not include
dependency URLs, hostnames, credentials, or connection errors. Configure the
orchestrator's liveness probe against `/health/live` and its readiness probe
against `/health/ready`; do not use readiness failures as a restart signal.

The older `/health` and `/ready` paths remain available as aliases for
deployments that have not migrated yet.
