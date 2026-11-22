# Incident Response Runbooks

Closes [#630](https://github.com/OdyxeeeLabs/Perigee/issues/630).

Runbooks for the incident classes Perigee can actually have. Each follows the
same structure so they can be followed under pressure:

| Section | What it is for |
| --- | --- |
| **Trigger** | When to open this runbook. Concrete enough to act on. |
| **Severity** | Default severity, and what moves it up or down. |
| **Diagnosis** | How to confirm it is what you think it is. |
| **Mitigation** | Ordered steps. Each says what it costs, because mitigation during an incident is a trade. |
| **Communication** | Who is told, when, and what is not said. |
| **Postmortem** | Template and the questions that must be answered. |

## Runbooks

| Incident | Runbook | Default severity |
| --- | --- | --- |
| Data breach (PII exposure) | [data-breach.md](data-breach.md) | SEV-1 |
| DDoS / volumetric attack | [ddos.md](ddos.md) | SEV-2 |
| Service degradation | [service-degradation.md](service-degradation.md) | SEV-2 |
| Security vulnerability in code | [vulnerability.md](vulnerability.md) | SEV-1 if exploitable, else SEV-2 |

## Severity scale

| Severity | Meaning | Response |
| --- | --- | --- |
| SEV-1 | Customer data at risk, or funds at risk, or full outage | Page immediately, 24/7 bridge, exec notified |
| SEV-2 | Partial outage or significant degradation, no data at risk | Page during business hours, bridge as needed |
| SEV-3 | Degraded, workaround exists, no customer impact | Ticket, triage next working day |
| SEV-4 | Cosmetic or internal only | Backlog |

Severity is set by **impact**, not by how alarming the symptom is. A 500 on
every request is SEV-1; a 500 on one endpoint with no customer impact is SEV-3.

## Escalation

```text
On-call engineer
  ├─ cannot contain within 30 min ──> Incident commander (engineering)
  │                                      ├─ data or funds at risk ──> Security lead + CISO
  │                                      └─ customer impact ─────> Head of support + comms
  └─ breach suspected ─────────────────> Security lead directly, in parallel
```

The last line matters: **a suspected breach escalates to the security lead in
parallel with technical containment, not after it.** Containment can destroy the
evidence that determines whether notification obligations were triggered, so
someone qualified to preserve it must be in the room before artefacts are rotated
or deleted.

## Communication principles

- **Say what you know, say what you don't.** "We detected unauthorised access to
  X between T1 and T2 and do not yet know if data was read" is a usable update.
  "We are investigating" is not.
- **Never speculate about root cause in a status update.** If the cause is not
  established, say so.
- **Regulatory clocks start at awareness, not at confirmation.** GDPR's 72 hours
  runs from when the controller became aware. Waiting for certainty can
  exhaust the window. When in doubt, notify and follow up.
- **One channel.** Status page updates go through the incident commander, not from
  individual responders, so two people do not publish contradictory accounts.

## Tabletop exercises

| Exercise | Frequency | Owner |
| --- | --- | --- |
| Data breach | Quarterly | Security lead |
| DDoS | Quarterly | Platform |
| Service degradation | Quarterly | Platform |
| Vulnerability disclosure | Twice yearly | Security lead |

Each exercise runs as a discussion against this runbook, timed, with a
discrepancy recorded for every step that turns out to be ambiguous or impossible.
An exercise that finds nothing was not run properly — these documents are written
before the pressure is on, and the ambiguity is the thing being looked for.

## Postmortems

Every SEV-1 and SEV-2 gets a written postmortem within five working days, using
[the template](postmortem-template.md). Blameless: the question is what made the
outcome likely, not who typed the command. A postmortem without a dated,
owned action item is a document, not a postmortem.