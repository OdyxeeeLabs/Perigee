# Postmortem Template

Every SEV-1 and SEV-2 incident gets one of these within five working days.
Blameless: the question is what made the outcome likely, not who typed the
command. "Engineer X ran the wrong command" is not a finding; "the command was
undocumented, unguarded and destructive" is.

A postmortem with no dated, owned action item is a document, not a postmortem.

---

## Incident summary

- **Title:**
- **Severity:** SEV-1 / SEV-2 / SEV-3
- **Date detected:**
- **Date mitigated:**
- **Customer-visible duration:**
- **Affected services:**
- **Data or funds impact:**

## Timeline

All times UTC. Include detection, not just occurrence — the gap between them is
usually the most important number in the document.

| Time | Event | Actor |
| --- | --- | --- |
| | | |
| | | |
| | | |

## What happened

Prose. What the system did, in order, and why each step produced the next.
No blame, no hedging.

## Why it happened

The contributing factors. There is almost always more than one, and "somebody
made a mistake" is never one of them on its own — it is always a mistake that the
system invited.

Distinguish:

- **Trigger** — the immediate cause.
- **Root cause** — why the system permitted the trigger.
- **Contributing factors** — what made detection or recovery harder.

## Detection

- How was it detected? Alert, user report, or noticed by chance.
- **Detection latency:** time from occurrence to detection.
- Did monitoring have visibility into the failure? If not, what was the blind spot?
- Would the existing alerts have caught it with better thresholds? Name them.

## Response

- What worked, and should be kept.
- What did not work, and should be removed.
- Where did responders have to improvise? Each improvisation is a documentation
  gap, and the improvisation is the evidence.

## Impact

- **Users:** how many, for how long, what did they experience.
- **Funds:** amount, and whether it is recoverable. State it plainly even if zero.
- **Data:** what was exposed, if anything. If none, say so explicitly.
- **Cost:** infrastructure spend during the incident, and any remediation spend.
- **Reputational:** status-page updates, support volume, public discussion.

## Actions

Every row needs an owner and a date. An unowned action is a wish.

| # | Action | Type | Owner | Due | Status |
| --- | --- | --- | --- | --- | --- |
| 1 | | Fix / Detect / Prevent / Process | | | Open |

### Action types

- **Fix** — removes the cause.
- **Detect** — makes the next occurrence visible sooner.
- **Prevent** — makes the next occurrence impossible or far less likely.
- **Process** — changes how the team works.

At least one **Detect** action is required. An incident that was found by a user
is the incident that should make detection the priority.

## Verification

How the team will know the actions actually worked. For a Detect action, that
means the new alert has been tested against a real occurrence, not just that the
rule exists.

---

## Incident-specific checklists

Append the relevant section for the incident class.

### Data breach — see [data-breach.md](data-breach.md)

- Detection latency from first unauthorised access to first alert.
- Did the encryption-at-rest controls limit the impact? If not, why not?
- Did anyone have standing access to decrypted PII they did not need?
- Was the audit log sufficient to answer "who" and "when"?
- Were notification deadlines met, measured from **awareness**?

### DDoS — see [ddos.md](ddos.md)

- Time to correctly characterise as an attack rather than organic load.
- Which control would have stopped it, and why was it not already engaged?
- Did the rate limiter protect, or did it become the outage?
- Cost attributable to the attack, and what share a cheaper control would avoid.

### Service degradation — see [service-degradation.md](service-degradation.md)

- Did a deploy trigger it? Did the pipeline's checks catch it?
- Was the circuit breaker engaged automatically or manually?
- Time from symptom to correct diagnosis.
- Did the status-page duration match the customer-visible duration?

### Vulnerability — see [vulnerability.md](vulnerability.md)

- Did a scanner fire? If so, why was the SLA missed?
- If no scanner fired, what rule would have caught it?
- Was the disclosure external, and what did the researcher see that we did not?
- Is any deployment configuration still affected after the code fix?