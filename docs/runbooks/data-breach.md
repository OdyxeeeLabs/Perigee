# Runbook: Data Breach

**Trigger:** any indication that personal data was accessed, modified, exfiltrated
or disclosed by means other than an authorised user acting within their role.

Indications that should open this runbook:

- an anomaly detection or SIEM alert on bulk read access to the PII tables;
- a report from a third party (researcher, user, vendor);
- a credential found committed, pasted or logged;
- an unexpected KMS `Decrypt` volume — see
  [encryption at rest](../ENCRYPTION_AT_REST.md);
- a misconfigured bucket, IAM role or CDN that made data public.

**Default severity:** SEV-1. Downgrade to SEV-2 only if the exposed data is
confirmed to be non-personal and publicly available anyway.

## Escalate immediately, in parallel

Contact the security lead **before** taking containment steps. Containment
commonly involves rotating credentials and deleting artefacts, both of which
destroy the evidence that establishes scope — and scope is what determines whether
GDPR Articles 33 and 34 are triggered. Preserve first, contain second.

## Diagnosis

Establish four things. Do not skip to containment before you can answer at
least the first two.

### 1. What data

```sql
-- Which tables, which columns, which rows touched
SELECT table_name, column_name, row_count_last_audit
FROM information_schema.columns
WHERE table_schema = 'perigee'
  AND column_name IN ('email', 'phone', 'full_name', 'wallet_address_label');
```

Perigee's PII is `email`, `phone`, `full_name`, `wallet_address_label` (from the
[encryption descriptor](../../config/security/encryption-at-rest.json)). On-chain
vault state is public by construction and is **not** a breach.

### 2. Who

```sql
-- Access in the window, with actor and authorisation
SELECT actor, action, resource, authorised, occurred_at
FROM audit_log
WHERE resource LIKE '%pii%'
  AND occurred_at BETWEEN $1 AND $2
ORDER BY occurred_at;
```

Check whether the access was authorised by a legitimate session. An actor who
acted within their role is an insider risk question, not necessarily a breach.

### 3. When

Take the window from the audit log, not from the alert timestamp. Alerts fire
late; the access is what matters. Overstate the window rather than understate it.

### 4. How much

```sql
SELECT COUNT(DISTINCT subject_id), MIN(occurred_at), MAX(occurred_at)
FROM audit_log
WHERE resource LIKE '%pii%' AND authorised = false;
```

## Mitigation

Ordered. Each step says what it costs, because containment during an incident is a
trade and the trade should be made deliberately.

1. **Revoke the credential.** If a key or token is suspected, revoke it now.
   *Cost:* whoever holds it is locked out. Acceptable — it may be the attacker.
2. **Do not delete logs.** Move them to a write-once store if they are at risk of
   being cleaned by retention policy. *Cost:* storage. Non-negotiable.
3. **Close the vector.** Widen the bucket policy, correct the IAM role, patch the
   endpoint. *Cost:* whatever the correction breaks. Prefer it to leaving the hole
   open.
4. **Rotate secrets** using the procedure in
   [secrets rotation](../SECRETS_ROTATION.md). If the breach may have exposed
   key material, rotation is urgent and the grace period is shortened — see the
   emergency procedure. *Cost:* an availability dip. Confidentiality wins.
5. **Assess notification obligation.** GDPR Article 33: notify the supervisory
   authority within **72 hours of awareness** unless the breach is unlikely to
   result in risk to individuals. Article 34 additionally requires notifying
   individuals where the risk is high. **The clock starts at awareness, not at
   confirmation.** If scope is not yet known at 72 hours, notify anyway and
   follow up — a late precise notification is better than an on-time vague one
   followed by nothing.
6. **Reset affected sessions** and require re-authentication.

## Communication

| Audience | When | Content |
| --- | --- | --- |
| Security lead + CISO | Immediately, before containment | Known facts only |
| Incident commander | Immediately | Trigger, severity, what is known |
| Supervisory authority | Within 72 h of awareness | Per GDPR Art. 33 template |
| Affected individuals | Within 72 h if risk is high | What data, what they should do |
| Status page | When customer impact exists | Facts and uncertainties, no speculation |
| All hands | At the postmortem | What we learned |

Never state a root cause in customer communication before it is established. "We
identified the mechanism on <date>" and "an unauthorised party used <technique>"
are both fine. "We were breached by <attacker>" when the attacker is a hypothesis
is not, because it becomes the headline of a story you cannot correct.

## Postmortem

Use [the template](postmortem-template.md) within five working days.

Questions specific to a breach that must be answered:

- What was the detection latency — time of the access to time of the alert?
  Anything over a day means the control that should have caught this did not.
- Would the [encryption at rest](../ENCRYPTION_AT_REST.md) controls have limited
  the impact, and if not, why not? A KMS-resident KEK with approval-gated
  `Decrypt` should have made the raw table unreadable; if it did not, the key
  policy was wrong.
- Did anyone have standing access to decrypted PII that they did not need?
- Was the audit log complete enough to answer "who" and "when"? If not, that gap
  is the most important finding in the document.

## Do not

- Do not delete, rotate or clean anything before the security lead has preserved
  it.
- Do not contact the attacker.
- Do not run an unapproved forensics script against production.
- Do not close the incident on the basis that "no one has complained". Absence of
  complaint is not evidence of absence of access.