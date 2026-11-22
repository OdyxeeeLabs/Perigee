#!/usr/bin/env python3
"""Verify that the deployed data layer actually meets the encryption-at-rest
baseline described in `docs/ENCRYPTION_AT_REST.md`.

This is a configuration gate, not a cryptography implementation. It reads a
deployment descriptor and asserts the controls are declared, so a stack that
quietly loses TDE, loses a KEK, or starts storing PII in the clear fails CI
rather than being discovered during an audit.

    python3 scripts/security/verify_encryption_at_rest.py
    python3 scripts/security/verify_encryption_at_rest.py --descriptor path.json

The descriptor is expected at `config/security/encryption-at-rest.json`. Each
check has an explicit `why` so an auditor reading the output does not have to
open the documentation to learn why a failure matters.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DESCRIPTOR = ROOT / "config" / "security" / "encryption-at-rest.json"

# Cipher suites that are acceptable for envelope encryption. AES-256-GCM is
# required for data; the others are rejected outright rather than warned about,
# because "we use ChaCha20" is a decision that has to be deliberate.
APPROVED_CIPHERS = {"AES-256-GCM"}

# Key wrapping must never use a mode that is known to be unsafe for that job.
FORBIDDEN_WRAP_MODES = {"ECB", "NONE", "static", "deterministic"}

REQUIRED_FIELDS = (
    "algorithm",
    "kek_provider",
    "kek_protection",
    "dek_rotation_days",
    "deks_encrypted_at_rest",
    "pii_fields",
    "database_tde",
    "backups_encrypted",
    "key_access_logged",
    "key_access_requires_approval",
)


def validate(descriptor_path: Path) -> list[str]:
    findings: list[str] = []

    try:
        descriptor = json.loads(descriptor_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return [f"{descriptor_path.relative_to(ROOT)}: not found"]
    except json.JSONDecodeError as exc:
        return [f"{descriptor_path.relative_to(ROOT)}: invalid JSON ({exc})"]

    for field in REQUIRED_FIELDS:
        if field not in descriptor:
            findings.append(f"missing `{field}`: the control is undeclared, which reads as off")

    algorithm = descriptor.get("algorithm")
    if algorithm is not None and algorithm not in APPROVED_CIPHERS:
        findings.append(
            f"algorithm {algorithm!r} is not approved; expected one of {sorted(APPROVED_CIPHERS)}"
        )

    wrap_mode = descriptor.get("dek_wrap_mode")
    if wrap_mode in FORBIDDEN_WRAP_MODES:
        findings.append(f"dek_wrap_mode {wrap_mode!r} must not be used for key wrapping")

    kek_provider = descriptor.get("kek_provider")
    if kek_provider and kek_provider.lower() in {"env", "file", "plaintext", "none"}:
        findings.append(
            f"kek_provider {kek_provider!r} keeps the KEK outside a KMS/HSM, which is the thing this control exists to prevent"
        )

    kek_protection = descriptor.get("kek_protection")
    if kek_protection and "hsm" not in kek_protection.lower() and "kms" not in kek_protection.lower():
        findings.append(
            f"kek_protection {kek_protection!r} does not mention KMS or HSM protection"
        )

    rotation_days = descriptor.get("dek_rotation_days")
    if isinstance(rotation_days, int):
        if rotation_days > 365:
            findings.append(f"dek_rotation_days is {rotation_days}, longer than the 365-day ceiling")
        if rotation_days <= 0:
            findings.append("dek_rotation_days must be positive")

    pii_fields = descriptor.get("pii_fields")
    if isinstance(pii_fields, list) and not pii_fields:
        findings.append("pii_fields is empty; either declare the fields or record that there are none")

    for flag in ("deks_encrypted_at_rest", "database_tde", "backups_encrypted"):
        if descriptor.get(flag) is not True:
            findings.append(f"{flag} must be true")

    if descriptor.get("key_access_logged") is not True:
        findings.append("key_access_logged must be true")
    if descriptor.get("key_access_requires_approval") is not True:
        findings.append("key_access_requires_approval must be true")

    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--descriptor", type=Path, default=DEFAULT_DESCRIPTOR)
    args = parser.parse_args()

    findings = validate(args.descriptor)
    if findings:
        print(f"FAIL: {len(findings)} encryption-at-rest finding(s)\n")
        for finding in findings:
            print(f"  - {finding}")
        return 1

    print(f"OK: {args.descriptor.relative_to(ROOT)} meets the encryption-at-rest baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main())