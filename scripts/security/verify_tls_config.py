#!/usr/bin/env python3
"""Verify the deployed transport layer matches `config/security/tls-policy.yml`.

Configuration gate, not a TLS implementation. It reads the policy and asserts the
controls are declared, so a stack that has quietly dropped to TLS 1.2, turned off
mTLS, or stopped renewing certificates fails CI rather than being found by a
pentest.

    python3 scripts/security/verify_tls_config.py
    python3 scripts/security/verify_tls_config.py --policy path.yml

What this cannot do: it validates *intent*, not the wire. Only `testssl.sh` or an
`openssl s_client` probe tells you what the server actually offers, and
docs/TLS.md lists those commands. A config gate and a wire check answer different
questions, and both are needed.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_POLICY = ROOT / "config" / "security" / "tls-policy.yml"

# The version the policy is required to pin.
REQUIRED_MIN_VERSION = "1.3"

# Substrings that mark a suite as weak regardless of position.
FORBIDDEN_SUITE_MARKERS = ("CBC", "RC4", "DES", "NULL", "EXPORT", "anon", "MD5")

# Suites that offer no forward secrecy.
NO_FORWARD_SECRECY = {
    "TLS_RSA_WITH_AES_128_GCM_SHA256",
    "TLS_RSA_WITH_AES_256_GCM_SHA384",
}

# A MAC weaker than SHA-256.
WEAK_MAC_SUFFIXES = ("_SHA", "_MD5", "_SHA1")


def parse_yaml_subset(text: str) -> dict:
    """Parse the YAML used by the policy files."""
    try:
        import yaml
        return yaml.safe_load(text) or {}
    except ImportError:
        pass

    root: dict = {}
    # Stack of (indent, container). `container` is a dict or a list.
    stack: list[tuple[int, object]] = [(-1, root)]
    # Key awaiting a block sequence or nested mapping.
    pending: tuple[int, dict, str] | None = None

    for number, raw in enumerate(text.splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue

        indent = len(raw) - len(raw.lstrip())
        line = raw.strip()

        # A block sequence item belongs to the most recent key.
        if line.startswith("- "):
            if pending is None:
                raise ValueError(f"line {number}: list item with no parent key")
            key_indent, owner, key = pending
            if indent <= key_indent:
                raise ValueError(f"line {number}: list item not indented under its key")
            value = line[2:].strip().strip("'\"")
            existing = owner.get(key)
            if isinstance(existing, list):
                existing.append(value)
            else:
                owner[key] = [value]
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

        if value == "":
            child: object = {}
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


def validate(policy_path: Path) -> list[str]:
    findings: list[str] = []

    try:
        text = policy_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return [f"{policy_path.relative_to(ROOT)}: not found"]

    try:
        policy = parse_yaml_subset(text)
    except ValueError as exc:
        return [f"{policy_path.relative_to(ROOT)}: {exc}"]

    external = policy.get("external") or {}
    internal = policy.get("internal") or {}
    hsts = policy.get("hsts") or {}
    monitoring = policy.get("monitoring") or {}

    min_version = external.get("min_version")
    if min_version != REQUIRED_MIN_VERSION:
        findings.append(
            f"external.min_version is {min_version!r}, expected {REQUIRED_MIN_VERSION!r}"
        )

    max_version = external.get("max_version")
    if max_version is not None and max_version != REQUIRED_MIN_VERSION:
        findings.append(
            f"external.max_version is {max_version!r}; only {REQUIRED_MIN_VERSION} is approved"
        )

    suites = external.get("suites_1_3") or []
    if not suites:
        findings.append("external.suites_1_3 is empty")
    for suite in suites:
        if suite in NO_FORWARD_SECRECY:
            findings.append(f"suite {suite!r} offers no forward secrecy")
        for marker in FORBIDDEN_SUITE_MARKERS:
            if marker in suite:
                findings.append(f"suite {suite!r} contains {marker!r}")
        if any(suite.endswith(suffix) for suffix in WEAK_MAC_SUFFIXES):
            findings.append(f"suite {suite!r} uses a MAC weaker than SHA-256")

    for suite in external.get("suites_1_2_fallback") or []:
        if not suite.startswith("ECDHE"):
            findings.append(
                f"fallback suite {suite!r} is not forward secret; only ECDHE is approved"
            )

    if external.get("renewal_window_days") is None:
        findings.append("external.renewal_window_days is not set")

    if internal.get("mtls") is not True:
        findings.append("internal.mtls must be true")
    if internal.get("identity") != "spire":
        findings.append("internal.identity must be spire, so SVIDs are short-lived")

    if hsts.get("preload") is not True:
        findings.append("hsts.preload must be true")
    if str(hsts.get("max_age")) != "31536000":
        findings.append("hsts.max_age must be 31536000 for preload eligibility")

    if not (monitoring.get("expiry_alert_days") or []):
        findings.append("monitoring.expiry_alert_days is empty")
    if monitoring.get("synthetic_probe_interval_minutes") is None:
        findings.append(
            "monitoring.synthetic_probe_interval_minutes is not set; an alert from "
            "the certificate system cannot report that system being down"
        )

    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    args = parser.parse_args()

    findings = validate(args.policy)
    if findings:
        print(f"FAIL: {len(findings)} TLS policy finding(s)\n")
        for finding in findings:
            print(f"  - {finding}")
        return 1

    print(f"OK: {args.policy.relative_to(ROOT)} meets the transport security policy")
    return 0


if __name__ == "__main__":
    sys.exit(main())