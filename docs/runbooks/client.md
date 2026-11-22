# Runbook: Security Vulnerability

**Trigger:** a vulnerability is identified in Perigee code, a dependency, or the
deployment configuration — by us, by a dependency advisory, or by an external
researcher.

Indications:

- a scanner alert (see [vulnerability scanning](../VULNERABILITY_SCANNING.md) for
  the SLA that applies to each severity);
- a `cargo audit` or OSV advisory on a dependency with a reachable code path;
- an external disclosure.

**Default severity:** SEV-1 if exploitable against production with material
impact; SEV-2 otherwise.

The word that decides the severity is **reachable**. An advisory affecting a code
path we do not use is a patch ticket, not an incident. Establishing reachability is
the first task and it is the one most often skipped, which is how a 500-line
backlog of unreachable advisories accumulates and trains people to ignore the
feed.

## Triage

### 1. Is it reachable?

```bash
# Does the vulnerable code exist?
grep -rn "<vulnerable symbol>" --include=*.rs --include=*.ts core/ contracts/ web/ | head

# Is it reachable from a public entry point?
grep -rn "<vulnerable function>" core/src/main.rs | head
```

For a dependency, `cargo tree -i <crate>` shows who depends on it and whether it
is a build-time or runtime dependency.

### 2. What does it cost an attacker?

Answer explicitly, because the SLA depends on it:

| If the vulnerability is… | Severity |
| --- | --- |
| Remotely exploitable, unauthenticated, affecting funds or PII | SEV-1 |
| Remotely exploitable, authenticated, affecting funds or PII | SEV-1 |
| Remotely exploitable, no data or funds impact (DoS, XSS on our own origin) | SEV-2 |
| Locally exploitable only, or requires an admin key already | SEV-3 |
| Not reachable | Ticket, no incident |

### 3. Is it being exploited?

Do not wait for evidence of exploitation before mitigating a reachable
vulnerability. "We have not seen it exploited" is a statement about detection
coverage, which is rarely complete. For SEV-1, assume exploitation and look for
evidence in parallel.

```sql
SELECT occurred_at, actor, action, resource
FROM audit_log
WHERE action IN ('withdraw', 'transfer', 'admin_set')
  AND occurred_at > now() - interval '72 hours'
ORDER BY occurred_at DESC;
```

## Mitigation

1. **Patch.** For a dependency, bump and redeploy.
   ```bash
   cargo update -p <crate> --precise <fixed-version>
   ```
   For our own code, a branch and a PR through the normal review process — **the
   exception is documented in the postmortem**, not skipped silently.
2. **Add a compensating control if the patch cannot ship now.** A WAF rule, an
   input filter, or disabling the affected endpoint. A compensating control with a
   stated expiry date is far better than waiting for a perfect fix.
3. **Deploy to staging first**, then production. A security fix that breaks the
   app is its own incident.
4. **Rotate any secret the vulnerability could have exposed**, using the
   emergency procedure in [secrets rotation](../SECRETS_ROTATION.md).
5. **Close the loop.** Update the scanner configuration so the same class of
   finding is caught earlier next time. A vulnerability that reaches production
   and is then fixed, with no change to detection, will happen again in a variant.

## External disclosure

If a researcher has reported it:

1. **Do not panic-merge.** A rushed merge under pressure is how a security fix
   introduces a second vulnerability.
2. **Acknowledge within 24 hours.** Not with a fix — with confirmation that the
   report was received and is being worked.
3. **Confirm scope with them.** They usually have more of the picture than you do,
   and engaging them is both faster and fairer.
4. **Agree a disclosure date** with them, and honour it. Coordinated disclosure
   requires giving the reporter the courtesy of a fix landing before the detail
   does.
5. **Credit them** in the advisory unless they ask not to be named.
6. **Never threaten legal action** over a good-faith report. Do not ask for
   exclusivity either: a researcher who has reported elsewhere and not told you
   is a researcher who will not tell you next time.

## Communication

| Audience | When | Content |
| --- | --- | --- |
| Incident commander | Immediately | Severity, reachability, plan |
| Security lead | Immediately if externally reported | Coordinate the disclosure timeline |
| Support | Before disclosure | So they are not the first to hear |
| Status page | If user-impacting | Users may need to take action |
| Advisory | After the fix is deployed | Severity, affected versions, fixed version, credit |

Do not publish technical detail before the fix is deployed and the disclosure
window has passed.

## Postmortem

Use [the template](postmortem-template.md) within five working days.

Questions specific to a vulnerability:

- Why did this reach production? Was there a scanner, and did it fire?
- If a scanner fired, why did the finding not get fixed within its SLA?
- If no scanner fired, what rule would have caught it, and why is it not in the
  config?
- Was the disclosure external? If so, what did the researcher see that we did
  not?
- Is there any deployment configuration still affected?

The last question is the one that recurs. Code gets patched and a permissive
security group outlives it for months.