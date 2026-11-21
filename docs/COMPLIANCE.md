# Compliance Reporting

Closes [#629](https://github.com/OdyxeeeLabs/Perigee/issues/629).

Perigee's control set is declared once in
[`config/compliance/frameworks.yml`](../config/compliance/frameworks.yml) and
mapped to the concrete evidence that satisfies each requirement. The auditor-ready
package is assembled by
[`scripts/compliance/collect_evidence.py`](../scripts/compliance/collect_evidence.py),
which fails if a mapped control has no evidence behind it — the gap that turns a
quarter of evidence-gathering into a scramble three days before an audit.

```bash
python3 scripts/compliance/collect_evidence.py --evidence-dir evidence/2026-Q1
```

## Frameworks

| Framework | Scope | What Perigee has to show |
| --- | --- | --- |
| SOC 2 Type II | Security, Availability, Confidentiality | Controls operating **over a period**, not once. A control with one screenshot is not Type II evidence. |
| GDPR | Personal data | Lawful basis, data-subject rights, retention, breach notification at 72 h. |
| PCI DSS | Cardholder data | Scope reduction: Perigee does **not** touch cardholder data, so most of PCI does not apply. |

### User-facing privacy notice

The web notice links to configurable privacy and telemetry documentation and is
enabled by default. Deployments can override `NEXT_PUBLIC_PRIVACY_NOTICE_ENABLED`,
`NEXT_PUBLIC_PRIVACY_POLICY_URL`, `NEXT_PUBLIC_TELEMETRY_DOCS_URL`, and
`NEXT_PUBLIC_PRIVACY_LEGAL_NOTE`. The notice and its default copy do not assert
certification or legal compliance; deployment owners supply any legal wording.

### Scope statement (PCI)

Perigee moves no cardholder data. Deposits are made through Stellar Classic
Assets or on-chain tokens; no card number, PAN or CVV is processed, stored or
transited. That statement reduces PCI scope to SAQ A rather than SAQ D, which is
the single highest-leverage thing this document does. It must be re-confirmed
whenever a fiat on-ramp is added — and that confirmation is itself a control.

## Evidence collection

Each control names where its evidence comes from. The collector resolves those
globs and reports three states per control:

| State | Meaning |
| --- | --- |
| `present` | At least one artefact matched, and the required observation window is covered. |
| `missing` | No artefact matched. Fails the run. |
| `partial` | Artefacts matched but the observation window is shorter than the framework requires. Reported, does not fail. |

The `partial` state is the interesting one. A missing artefact is obvious; an
audit that covers eleven weeks of logs when the framework needs twelve is how a
Type II opinion turns into a qualified one with no warning.

### Observation windows

| Framework | Window |
| --- | --- |
| SOC 2 Type II | 3 months minimum, 6–12 typical |
| GDPR | Continuous, plus 12 months of breach records |
| PCI DSS | 12 months of ASV scans |

## Running it

Collection is scheduled weekly and before each audit:

```bash
# Weekly
python3 scripts/compliance/collect_evidence.py --output evidence/$(date +%Y-W%V)

# Before an audit
python3 scripts/compliance/collect_evidence.py --evidence-dir evidence/2026-Q1 --strict
```

`--strict` turns `partial` into a failure, which is what you want when you are
actually assembling a package rather than just checking.

Output is a directory containing a manifest (`manifest.json`) mapping every
control to the artefacts that satisfy it, plus the artefacts themselves. The
manifest is the part an auditor reads; the artefacts are the part they spot-check.

## Continuous compliance dashboard

The collector emits machine-readable status, so the dashboard is a read of
`manifest.json` rather than a separate pipeline. Per control: state, artefact
count, window start and end, and days until the window lapses.

The metric worth watching is **days until window lapses**, not the pass rate. A
control that has been `present` for eleven weeks out of twelve is healthy; the
pass rate is identical at eleven weeks and at thirteen, and only one of those is
a problem.

## Controls with no evidence yet

`collect_evidence.py` exits non-zero while any control is `missing`, so the gap is
visible in CI rather than discovered in an audit. Current gaps are recorded in
`frameworks.yml` with an owner and a target, not left as an empty list.

## Adding a control

1. Add it to `config/compliance/frameworks.yml` with its requirement text, the
   evidence glob, and the observation window.
2. Create the evidence producer if there is not one already.
3. Run the collector. A control whose glob matches nothing fails immediately,
   which is the intended behaviour — a control with no evidence is not a control.