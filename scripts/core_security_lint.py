from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
errors = []

for base in (ROOT / "core" / "src", ROOT / "contracts"):
    for path in base.rglob("*.rs"):
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT)
        for number, line in enumerate(text.splitlines(), 1):
            if re.search(r"BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY", line):
                errors.append(f"{relative}:{number}: embedded private key material")
            if base.name == "contracts" and re.search(r"\bunsafe\s*\{", line):
                errors.append(f"{relative}:{number}: unsafe block in contract code")
            if "danger_accept_invalid_certs(true)" in line.replace(" ", ""):
                errors.append(f"{relative}:{number}: invalid TLS certificates accepted")

main_path = ROOT / "core" / "src" / "main.rs"
main = main_path.read_text(encoding="utf-8")
if "database_url = %" in main or "redis_url = %" in main or "config = %config" in main:
    errors.append("core/src/main.rs: secret-bearing configuration logged")
if "JWT_PRIVATE_KEY or JWT_KEY_RING is required in production" not in main:
    errors.append("core/src/main.rs: production does not require persistent JWT key material")
if "CORS_ALLOWED_ORIGINS is required in production" not in main:
    errors.append("core/src/main.rs: production does not require an explicit CORS allowlist")

api_start = main.find("let api_routes = Router::new()")
api_end = main.find("let app = Router::new()", api_start)
if api_start < 0 or api_end < 0:
    errors.append("core/src/main.rs: unable to locate the public route block")
else:
    public_routes = main[api_start:api_end]
    sensitive_public_routes = (
        '"/auth/emergency-pause"',
        '.route("/managers",',
        '"/managers/:id"',
        '.route("/reconcile",',
    )
    for route in sensitive_public_routes:
        if route in public_routes:
            errors.append(f"core/src/main.rs: sensitive route is public: {route}")

auth_path = ROOT / "core" / "src" / "auth.rs"
auth = auth_path.read_text(encoding="utf-8")
for required in ("decode_header", "verification_key", "header.kid", "JWT_KEY_RING"):
    if required not in auth:
        errors.append(f"core/src/auth.rs: missing key-ring invariant '{required}'")

manager = (ROOT / "core" / "src" / "manager_store.rs").read_text(encoding="utf-8")
if "fn require_admin" not in manager:
    errors.append("core/src/manager_store.rs: missing administrator authorization guard")

reconciliation = (ROOT / "core" / "src" / "reconciliation.rs").read_text(encoding="utf-8")
if "fn require_operator" not in reconciliation:
    errors.append("core/src/reconciliation.rs: missing operator authorization guard")

if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)

print("core security lint passed")
