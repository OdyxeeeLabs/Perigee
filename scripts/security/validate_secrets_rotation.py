#!/usr/bin/env python3
"""Validate `config/security/secrets-rotation.json`.

Run in CI and as a pre-commit check. Exits non-zero on any finding, so a secret
added without a rotation policy cannot land.

    python3 scripts/security/validate_secrets_rotation.py
    python3 scripts/security/validate_secrets_rotation.py --config path.json

Checks:

* every secret declares a policy that exists, and the policy's `max_age_days`
  matches the SLA in `docs/SECRETS_ROTATION.md` (90d passwords, 365d keys);
* every secret names an owner, a source and a rotation procedure;
* every secret that is consumed by a tracked file actually names a file that
  exists, so a renamed module cannot silently drop out of rotation;
* every secret sets `audit: true`;
* secret names do not look like they contain a literal value.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONFIG = ROOT / "config" / "security" / "secrets-rotation.json"

# The SLAs this file encodes. Changing either requires updating
# docs/SECRETS_ROTATION.md in the same change.
EXPECTED_MAX_AGE_DAYS = {"critical": 90, "standard": 90, "encryption_key": 365}

# Anything matching this in a secret *name* means a value was pasted into the
# config instead of a reference.
VALUE_SHAPED_TOKENS = ("=", "ghp_", "sk-", "xox", "AKIA", "-----BEGIN")


def validate(config_path: Path) -> list[str]:
    findings: list[str] = []

    try:
        config = json.loads(config_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return [f"{config_path.relative_to(ROOT)}: not found"]
    except json.JSONDecodeError as exc:
        return [f"{config_path.relative_to(ROOT)}: invalid JSON ({exc})"]

    policies = config.get("policies") or {}
    for name, expected_age in EXPECTED_MAX_AGE_DAYS.items():
        policy = policies.get(name)
        if policy is None:
            findings.append(f"policy {name!r} is missing")
            continue
        actual_age = policy.get("max_age_days")
        if actual_age != expected_age:
            findings.append(
                f"policy {name!r} max_age_days is {actual_age}, expected {expected_age}"
            )
        if not policy.get("require_dual_secret_window"):
            findings.append(
                f"policy {name!r} must set require_dual_secret_window so rotation is zero-downtime"
            )

    secrets = config.get("secrets") or []
    if not secrets:
        findings.append("no secrets are declared")
        return findings

    seen: set[str] = set()
    for entry in secrets:
        name = entry.get("name", "<unnamed>")
        label = f"secret {name!r}"

        if name in seen:
            findings.append(f"{label}: declared more than once")
        seen.add(name)

        if any(token in name for token in VALUE_SHAPED_TOKENS):
            findings.append(
                f"{label}: the name looks like it contains a literal value, not a reference"
            )

        policy_name = entry.get("policy")
        if policy_name not in policies:
            findings.append(f"{label}: unknown policy {policy_name!r}")

        for field in ("owner", "source", "rotation_procedure", "emergency_procedure"):
            if not entry.get(field):
                findings.append(f"{label}: missing {field}")

        if entry.get("audit") is not True:
            findings.append(f"{label}: audit must be true")

        consumers = entry.get("consumers") or []
        if not consumers:
            findings.append(f"{label}: no consumers declared")
        for consumer in consumers:
            if not (ROOT / consumer).exists():
                findings.append(
                    f"{label}: consumer {consumer!r} does not exist, so it cannot be rotated"
                )

    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    args = parser.parse_args()

    findings = validate(args.config)
    if findings:
        print(f"FAIL: {len(findings)} secret rotation finding(s)\n")
        for finding in findings:
            print(f"  - {finding}")
        return 1

    print(f"OK: {args.config.relative_to(ROOT)} validates")
    return 0


if __name__ == "__main__":
    sys.exit(main())