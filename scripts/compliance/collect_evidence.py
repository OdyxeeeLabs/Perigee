#!/usr/bin/env python3
"""Assemble an auditor-ready compliance evidence package.

Reads `config/compliance/frameworks.yml`, resolves each control's evidence glob
against an evidence directory, and reports three states per control:

    present   at least one artefact matched and the observation window is covered
    missing   no artefact matched — always a failure
    partial   artefacts matched but the observation window is too short —
              reported, and a failure only under --strict

The `partial` state is the one that matters. A missing artefact is obvious; an
audit covering eleven weeks of logs when the framework needs twelve is how a Type
II opinion becomes a qualified one, and nothing about that is visible on a pass
rate.

    python3 scripts/compliance/collect_evidence.py --evidence-dir evidence/2026-Q1
    python3 scripts/compliance/collect_evidence.py --strict
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import date, datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_FRAMEWORKS = ROOT / "config" / "compliance" / "frameworks.yml"
DEFAULT_EVIDENCE = ROOT / "evidence"


def parse_yaml_subset(text: str) -> dict:
    """Parse the strict YAML subset the framework files use.

    Nested mappings, block sequences and inline flow sequences — and nothing
    else. Raises rather than guessing on anything it does not understand, so a
    malformed control file fails loudly instead of quietly producing an empty
    control set.
    """
    root: dict = {}
    stack: list[tuple[int, object]] = [(-1, root)]
    pending: tuple[int, dict, str] | None = None

    for number, raw in enumerate(text.splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue

        indent = len(raw) - len(raw.lstrip())
        line = raw.strip()

        if line.startswith("- "):
            if pending is None:
                raise ValueError(f"line {number}: list item with no parent key")
            key_indent, owner, key = pending
            if indent <= key_indent:
                raise ValueError(f"line {number}: list item not indented under its key")
            existing = owner.get(key)
            if isinstance(existing, list):
                existing.append(line[2:].strip().strip("'\""))
            else:
                owner[key] = [line[2:].strip().strip("'\"")]
            continue

        while stack and indent <= stack[-1][0]:
            stack.pop()
        if not stack:
            raise ValueError(f"line {number}: inconsistent indentation")
        parent = stack[-1][1]
        if not isinstance(parent, dict):
            raise ValueError(f"line {number}: expected a mapping")

        if ":" not in line:
            raise ValueError(f"line {number}: expected `key: value`")
        key, _, value = line.partition(":")
        key = key.strip()
        value = value.strip()
        pending = None

        if value in (">-", ">", "|"):
            # Folded block scalar: skip it, none of the control data uses it for
            # anything the collector reads.
            child: object = ""
            parent[key] = child
            continue

        if value == "":
            child = {}
            parent[key] = child
            stack.append((indent, child))
            pending = (indent, parent, key)
        elif value.startswith("[") and value.endswith("]"):
            parent[key] = [
                item.strip().strip("'\"")
                for item in value[1:-1].split(",")
                if item.strip()
            ]
        else:
            parent[key] = value.strip("'\"")

    return root


def resolve_glob(evidence_dir: Path, pattern: str) -> list[Path]:
    """Resolve one evidence glob relative to the evidence directory.

    Patterns are rooted, e.g. `evidence/logs/access/**`. The `evidence/` prefix
    is stripped so the same mapping works whether the caller points at the repo
    root or at a packaged evidence directory.
    """
    trimmed = pattern
    if trimmed.startswith("evidence/"):
        trimmed = trimmed[len("evidence/") :]
    trimmed = trimmed.lstrip("/")

    # `a/**/b` should also match `a/b`, which pathlib's glob does not do.
    candidate = trimmed.replace("/**/", "/")
    matches = set(evidence_dir.glob(trimmed))
    matches |= set(evidence_dir.glob(candidate))
    return sorted(p for p in matches if p.is_file())


def observation_window_days(evidence_dir: Path, artefacts: list[Path]) -> int:
    """Days between the oldest and newest artefact modification times."""
    if len(artefacts) < 2:
        return 0
    stamps = [
        datetime.fromtimestamp(p.stat().st_mtime, tz=timezone.utc) for p in artefacts
    ]
    return (max(stamps) - min(stamps)).days


def collect(
    frameworks_path: Path, evidence_dir: Path, strict: bool
) -> tuple[dict, list[str]]:
    findings: list[str] = []
    try:
        text = frameworks_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return {}, [f"{frameworks_path.relative_to(ROOT)}: not found"]

    try:
        config = parse_yaml_subset(text)
    except ValueError as exc:
        return {}, [f"{frameworks_path.relative_to(ROOT)}: {exc}"]

    declared_gaps = {
        entry.get("control")
        for entry in (config.get("known_gaps") or [])
        if isinstance(entry, dict)
    }

    manifest: dict = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "evidence_dir": str(evidence_dir),
        "strict": strict,
        "frameworks": {},
    }

    total_controls = 0
    missing_controls = 0
    partial_controls = 0

    for framework_key, framework in (config.get("frameworks") or {}).items():
        if not isinstance(framework, dict):
            continue

        required_days = int(framework.get("observation_window_days") or 0)
        criteria_out = {}
        for control_id, control in (framework.get("criteria") or {}).items():
            if not isinstance(control, dict):
                continue
            total_controls += 1

            pattern = control.get("evidence") or ""
            artefacts = resolve_glob(evidence_dir, pattern) if pattern else []
            window = observation_window_days(evidence_dir, artefacts)

            if not artefacts:
                state = "missing"
            elif required_days and window < required_days:
                state = "partial"
            else:
                state = "present"

            if state == "missing":
                missing_controls += 1
                if control_id not in declared_gaps:
                    findings.append(
                        f"{framework_key}.{control_id} ({control.get('title')}): "
                        f"no artefact matches {pattern!r} and it is not a declared gap"
                    )
            if state == "partial":
                partial_controls += 1

            criteria_out[control_id] = {
                "title": control.get("title"),
                "pattern": pattern,
                "state": state,
                "artefacts": [str(p.relative_to(evidence_dir)) for p in artefacts],
                "artefact_count": len(artefacts),
                "observation_window_days": window,
                "required_window_days": required_days,
                "declared_gap": control_id in declared_gaps,
            }

        manifest["frameworks"][framework_key] = {
            "name": framework.get("name"),
            "observation_window_days": required_days,
            "criteria": criteria_out,
        }

    summary = manifest["summary"] = {
        "total_controls": total_controls,
        "present": total_controls - missing_controls - partial_controls,
        "partial": partial_controls,
        "missing": missing_controls,
    }

    if strict and partial_controls:
        findings.append(
            f"--strict: {partial_controls} control(s) have evidence but an observation "
            "window shorter than the framework requires"
        )

    return manifest, findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frameworks", type=Path, default=DEFAULT_FRAMEWORKS)
    parser.add_argument("--evidence-dir", type=Path, default=DEFAULT_EVIDENCE)
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument(
        "--strict",
        action="store_true",
        help="treat an incomplete observation window as a failure, not a warning",
    )
    args = parser.parse_args()

    manifest, findings = collect(args.frameworks, args.evidence_dir, args.strict)

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        print(f"wrote {args.output}")

    summary = manifest.get("summary") or {}
    print(
        f"controls: {summary.get('present', 0)} present, "
        f"{summary.get('partial', 0)} partial, {summary.get('missing', 0)} missing"
    )

    if findings:
        print(f"\nFAIL: {len(findings)} finding(s)\n")
        for finding in findings:
            print(f"  - {finding}")
        return 1

    print("OK: every declared control has evidence within its observation window")
    return 0


if __name__ == "__main__":
    sys.exit(main())