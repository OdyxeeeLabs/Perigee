# Runbook: Service Degradation

**Trigger:** partial outage, elevated error rate, elevated latency, or a stuck
queue.

Indications:

- error rate above 1% over 5 minutes;
- p95 latency above 2 s for 10 minutes;
- queue depth growing for 15 minutes;
- a health check failing on any instance;
- the vault dashboard showing deposits succeeding but withdrawals failing.

**Default severity:** SEV-2.

## Diagnosis

### Which layer

Work down; stop at the first layer that explains the symptom.

```bash
# 1. Edge — is it us or the network in front of us?
curl -s -o /dev/null -w '%{http_code} %{time_total}\n' https://perigee.example/healthz

# 2. API — is core/ healthy?
curl -s https://api.perigee.example/healthz | jq .

# 3. Dependencies — what does core/ say is wrong?
curl -s https://api.perigee.example/healthz/deep | jq .
```

The `/healthz/deep` endpoint reports database, Redis, RPC and indexer reachability
together, which turns "the API is slow" into "the API is slow because the database
is not accepting connections".

### Database

```sql
SELECT now(), state, count(*), max(now() - query_start) AS longest
FROM pg_stat_activity
GROUP BY state;

SELECT wait_event_type, wait_event, count(*)
FROM pg_stat_activity
WHERE state = 'active'
GROUP BY 1, 2
ORDER BY 3 DESC;
```

`wait_event_type = 'Lock'` means a blocking transaction. Find it:

```sql
SELECT pid, now() - xact_start AS age, query
FROM pg_stat_activity
WHERE state <> 'idle' AND xact_start IS NOT NULL
ORDER BY age DESC;
```

A long-running transaction holding locks is the single most common cause of
"the API degraded for no reason", because it looks like nothing is happening.

### Application

```sql
SELECT
  date_trunc('minute', occurred_at) AS minute,
  COUNT(*) FILTER (WHERE status >= 500) AS errors,
  COUNT(*) AS total,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY duration_ms) AS p95
FROM request_log
WHERE occurred_at > now() - interval '30 minutes'
GROUP BY 1
ORDER BY 1;
```

Is this a spike or a step change? A step change at a round timestamp is a deploy.
A gradual climb is usually capacity or a dependency degrading.

### Recent changes

```bash
git log --oneline --since="2 hours ago"
kubectl rollout history deployment/core
```

Check this before anything else. The most common trigger for degradation is a
deploy, and it is the cheapest to revert.

## Mitigation

Ordered by cost of being wrong.

1. **Roll back the last deploy** if one correlates. *Cost:* discarding whatever
   the deploy fixed. Cheapest and fastest; do it first if the timing fits.
2. **Scale out** if the bottleneck is CPU or connections.
   ```bash
   kubectl scale deployment/core --replicas=6
   ```
   *Cost:* money. Check connection limits first — more instances against a
   saturated pool makes things worse.
3. **Enable the circuit breaker.** The API should stop calling a degraded
   dependency rather than queue behind it. If it is not enabled, enable it
   manually; this is the control that turns a dependency failure into a
   degradation instead of an outage.
4. **Kill the blocking transaction** if a long-running transaction holds locks.
   ```sql
   SELECT pg_cancel_backend($PID);
   ```
   *Cost:* the work in flight is lost. Right trade when the alternative is a
   total outage.
5. **Disable the non-essential workload** — analytics, indexer backfill, the
   resource-usage chart polling. Reclaim capacity for the product path.
   *Cost:* data staleness and an empty dashboard. Acceptable during an incident.
6. **Raise limits** — connection pool, queue depth, worker count — **only after**
   confirming the bottleneck is a limit and not saturation. Raising a limit when
   the real problem is saturation moves the failure later and makes it worse.

## Communication

| Audience | When | Content |
| --- | --- | --- |
| Incident commander | Immediately | Which layer, what is engaged |
| Support | Within 15 min | Which features are degraded, what users will see |
| Status page | When users are affected | Scope and mitigation |
| Engineering | At resolution | Fixed, and any follow-up |

Support specifically needs: which features, what the user-visible symptom is, and
whether retrying is safe. "Deposits are retry-safe, withdrawals are not yet
confirmed" is the kind of sentence that prevents a support ticket storm.

## Recovery

Do not close the incident when metrics recover. Close it when:

1. Metrics have been healthy for **at least 30 minutes**, long enough to clear
   the busiest period.
2. Queues have drained, not merely stopped growing.
3. Any manually disabled feature is re-enabled, one at a time, watching the
   metrics after each.

Restoring disabled features one at a time is the step that gets skipped and then
produces a second incident an hour later.

## Postmortem

- Did the deploy pipeline's checks catch this, or did it reach production?
- Was the circuit breaker engaged automatically? If not, what had to be done by
  hand?
- How long from symptom to correct diagnosis? That is the metric to improve, not
  the total duration.
- What was the customer-visible impact in minutes, and did status-page updates
  match it?