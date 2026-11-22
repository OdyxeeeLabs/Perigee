# Runbook: DDoS / Volumetric Attack

**Trigger:** request rate or bandwidth materially above baseline, error rate
rising under load, or a provider quota alert.

Indications:

- p95 latency above 2 s with flat request volume (saturation, not necessarily
  attack);
- p95 latency above 2 s with request volume rising (load shedding — attack);
- WAF block rate above 5% of requests;
- CDN or provider quota at 50% of a monthly allocation.

**Default severity:** SEV-2. Escalate to SEV-1 if the API stops serving
legitimate customers entirely, or if funds movement is in progress and blocked.

## Diagnosis

### Establish whether it is an attack or organic load

```sql
SELECT
  date_trunc('minute', occurred_at) AS minute,
  COUNT(*) AS requests,
  COUNT(DISTINCT client_ip) AS distinct_clients,
  AVG(EXTRACT(EPOCH FROM duration_ms)) AS avg_ms,
  MAX(EXTRACT(EPOCH FROM duration_ms)) AS max_ms
FROM request_log
WHERE occurred_at > now() - interval '30 minutes'
GROUP BY 1
ORDER BY 1;
```

The discriminator is **requests per distinct client**. Organic growth raises
requests *and* clients together. An attack raises requests with clients flat or
falling.

### Characterise the source

```sql
SELECT
  client_ip,
  country,
  asn,
  user_agent,
  path,
  COUNT(*) AS hits
FROM request_log
WHERE occurred_at > now() - interval '10 minutes'
GROUP BY 1, 2, 3, 4, 5
HAVING COUNT(*) > 1000
ORDER BY hits DESC
LIMIT 50;
```

Look for: a narrow set of IPs or ASNs; requests concentrated on the most
expensive endpoints (`/api/contracts/invoke`, wasm downloads); a single ASN
accounting for a disproportionate share; a uniform user agent across many IPs.

### Check whether the expensive endpoints are the target

Endpoints that are cheap to request and expensive to serve are the ones that turn
a volumetric attack into an economic one. Check the top paths from the query
above, and note which of them touch the database.

## Mitigation

Work down this list. Each step buys time; none of them individually solves the
problem, and the point is to reach a configuration where legitimate traffic is
served while the attack is not.

1. **Turn on the rate limiter.** It is already implemented in
   `web/lib/api-middleware.ts`; confirm the limits are engaged in production
   rather than assuming they are.
   ```bash
   curl -sX POST "$API/admin/rate-limit" \
     -H "authorization: Bearer $ADMIN_TOKEN" \
     -d '{"max_per_minute":60}'
   ```
   *Cost:* legitimate bursts are throttled too. Watch successful-request rate,
   not just block rate — a limiter that blocks 60% of real users has moved the
   problem rather than solved it.

2. **Enable the challenge** on `/api/*` rather than hard-blocking. A proof-of-work
   or CAPTCHA turns a scripted flood into something a script cannot cheaply do,
   while leaving a human able to proceed.
   *Cost:* an extra round trip for every legitimate user.

3. **Add WAF rules** for the observed signature: ASN or geography block, rate
   limit per path, bot score threshold. Prefer per-path limits over a global
   one — a global limit is an invitation for one attacker to deny service to
   everyone.

4. **Scale out.** If the origin is the bottleneck and budget allows, add capacity.
   *Cost:* money. Only worth it if the attack rate is close to what capacity
   absorbs; scaling into a 50 Gbps flood just moves the invoice.

5. **Strip expensive endpoints at the edge.** If `/api/contracts/invoke` or the
   wasm routes are being targeted, block them at the CDN for the duration. These
   are the endpoints where the attack-to-cost ratio is worst.

6. **Ask the provider for mitigation.** Every major provider has a DDoS scrubbing
   service with an activation time measured in minutes to hours. Know the
   contact and the current runbook *before* the attack.

7. **Rotate the emergency contact if the attack targets the domain.** A
   registrar-level or DNS-level attack is out of scope for these runbooks; hand
   it to the provider's abuse team and raise a separate incident.

## Communication

| Audience | When | Content |
| --- | --- | --- |
| Incident commander | Immediately | Characterisation and which mitigations are engaged |
| Support | Within 30 min | Tell them requests may be rejected, and what a rejected request looks like |
| Status page | When customers are affected | Degradation, mitigation in progress, no ETA promise you cannot keep |
| Provider | If scrubbing is needed | Request mitigation, give the mitigation contact |

Support needs to know what a rejected request looks like. Without that they will
tell users the site is broken when it is deliberately shedding load, and generate
tickets for behaviour that is correct.

## Postmortem

- Which control would have stopped this, and why was it not already engaged?
- Was the attack distinguishable from organic load quickly enough? Time to
  correct characterisation is a real metric.
- Did the rate limiter protect or did it become the outage?
- What was the cost, and was any of it avoidable by a cheaper control?

## Do not

- Do not block by hand at scale during the incident; get it into configuration so
  it can be rolled back deliberately.
- Do not add a global rate limit as a first move. It is the same lever an attacker
  uses.
- Do not commit any change made during the incident without the usual review. The
  exception is documented in the postmortem.