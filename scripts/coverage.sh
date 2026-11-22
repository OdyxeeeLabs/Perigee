#!/usr/bin/env bash
#
# CONTRACT-41 (#535) — contract coverage report + threshold enforcement.
#
# Runs `cargo tarpaulin` over every Soroban contract crate that belongs to the
# Cargo workspace, writes XML/LCOV/HTML reports, prints a per-package summary
# and fails when the aggregate line-coverage percentage drops below
# `COVERAGE_THRESHOLD`.
#
# Usage:
#   bash scripts/coverage.sh
#   COVERAGE_THRESHOLD=90 bash scripts/coverage.sh
#   COVERAGE_PACKAGES="perigee-bootstrap Perigee-math" bash scripts/coverage.sh
#
# Environment variables:
#   COVERAGE_THRESHOLD      minimum line coverage % (default: 80)
#   COVERAGE_PACKAGES       space separated crate names to measure
#                           (default: auto-discover the workspace contracts)
#   COVERAGE_OUTPUT_DIR     report directory (default: coverage-report)
#   COVERAGE_PROFILE        cargo profile to build with (default: coverage,
#                           set to "" to fall back to tarpaulin's own default)
#   COVERAGE_TIMEOUT        per-test timeout in seconds (default: 300)
#   COVERAGE_EXCLUDE_FILES  optional tarpaulin --exclude-files patterns
#   COVERAGE_TARPAULIN      path/name of the tarpaulin binary (optional)
#
# Reports are written to:
#   $COVERAGE_OUTPUT_DIR/cobertura.xml  machine readable (used for the gate)
#   $COVERAGE_OUTPUT_DIR/lcov.info      editor / Codecov compatible
#   $COVERAGE_OUTPUT_DIR/index.html     human readable
#   $COVERAGE_OUTPUT_DIR/summary.md     Markdown summary, also appended to
#                                       $GITHUB_STEP_SUMMARY when set
#
# Exit codes: 0 = threshold met, 1 = threshold missed or setup error.

set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

THRESHOLD="${COVERAGE_THRESHOLD:-80}"
OUT_DIR="${COVERAGE_OUTPUT_DIR:-coverage-report}"
PROFILE="${COVERAGE_PROFILE-coverage}"
TIMEOUT="${COVERAGE_TIMEOUT:-300}"

if [[ -t 1 ]]; then
  BLUE=$'\033[34m'; GREEN=$'\033[32m'; RED=$'\033[31m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  BLUE=""; GREEN=""; RED=""; YELLOW=""; RESET=""
fi

log()  { printf '%s==>%s %s\n' "$BLUE" "$RESET" "$*"; }
ok()   { printf '%s%s%s\n' "$GREEN" "$*" "$RESET"; }
warn() { printf '%s[warn] %s%s\n' "$YELLOW" "$*" "$RESET" >&2; }
die()  { printf '%s[error] %s%s\n' "$RED" "$*" "$RESET" >&2; exit 1; }

# ── Validate the threshold ────────────────────────────────────────────────────
python3 - "$THRESHOLD" <<'PY' || die "COVERAGE_THRESHOLD must be a number between 0 and 100 (got '${THRESHOLD}')."
import sys
try:
    value = float(sys.argv[1])
except ValueError:
    sys.exit(1)
sys.exit(0 if 0.0 <= value <= 100.0 else 1)
PY

# ── Locate tarpaulin ─────────────────────────────────────────────────────────
if [[ -n "${COVERAGE_TARPAULIN:-}" ]]; then
  TARPAULIN=("$COVERAGE_TARPAULIN")
elif command -v cargo-tarpaulin >/dev/null 2>&1; then
  TARPAULIN=(cargo-tarpaulin)
elif command -v cargo >/dev/null 2>&1 && cargo tarpaulin --version >/dev/null 2>&1; then
  TARPAULIN=(cargo tarpaulin)
else
  die "cargo-tarpaulin is not installed. Install it with: cargo install cargo-tarpaulin --locked"
fi

# ── Select the contract crates to measure ────────────────────────────────────
if [[ -n "${COVERAGE_PACKAGES:-}" ]]; then
  log "Using COVERAGE_PACKAGES from the environment"
  PACKAGES=()
  # shellcheck disable=SC2206
  PACKAGES=(${COVERAGE_PACKAGES})
else
  log "Discovering contract crates in the Cargo workspace"
  PACKAGES=()
  while IFS= read -r pkg; do
    [[ -n "$pkg" ]] && PACKAGES+=("$pkg")
  done < <(cargo metadata --no-deps --format-version 1 | python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
members = set(metadata.get("workspace_members", []))
found = []
for package in metadata.get("packages", []):
    if package.get("id") not in members:
        continue
    manifest = str(package.get("manifest_path", "")).replace("\\", "/")
    if "/contracts/" in manifest:
        found.append(package["name"])
for name in sorted(found):
    print(name)
')
fi

[[ ${#PACKAGES[@]} -gt 0 ]] || die "No contract crates found. Pass COVERAGE_PACKAGES explicitly."

log "Measuring ${#PACKAGES[@]} contract crate(s): ${PACKAGES[*]}"

# Contract directories under contracts/ that are not workspace members are
# standalone Cargo projects and cannot be built from this workspace, so they
# are reported as skipped rather than silently ignored.
SKIPPED=()
while IFS= read -r manifest; do
  crate_dir="$(dirname -- "$manifest")"
  crate_name="$(sed -n 's/^name[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' "$manifest" | head -n 1)"
  [[ -z "$crate_name" ]] && continue
  keep=0
  for pkg in "${PACKAGES[@]}"; do
    if [[ "$pkg" == "$crate_name" ]]; then
      keep=1
      break
    fi
  done
  [[ $keep -eq 0 ]] && SKIPPED+=("$crate_dir")
done < <(find contracts -mindepth 2 -maxdepth 2 -name Cargo.toml 2>/dev/null | sort)

if [[ ${#SKIPPED[@]} -gt 0 ]]; then
  warn "Outside the Cargo workspace, skipped by this report: ${SKIPPED[*]}"
fi

# ── Run tarpaulin ────────────────────────────────────────────────────────────
mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR/cobertura.xml" "$OUT_DIR/lcov.info" "$OUT_DIR/summary.md"

TARPAULIN_ARGS=(--out Xml --out Lcov --out Html --output-dir "$OUT_DIR" --timeout "$TIMEOUT" --skip-clean)
for pkg in "${PACKAGES[@]}"; do
  TARPAULIN_ARGS+=(-p "$pkg")
done
[[ -n "$PROFILE" ]] && TARPAULIN_ARGS+=(--profile "$PROFILE")
# shellcheck disable=SC2206
[[ -n "${COVERAGE_EXCLUDE_FILES:-}" ]] && TARPAULIN_ARGS+=(--exclude-files ${COVERAGE_EXCLUDE_FILES})

log "Running: ${TARPAULIN[*]} ${TARPAULIN_ARGS[*]}"
"${TARPAULIN[@]}" "${TARPAULIN_ARGS[@]}"

COBERTURA="$OUT_DIR/cobertura.xml"
[[ -f "$COBERTURA" ]] || die "tarpaulin did not produce ${COBERTURA}"

# ── Evaluate the threshold ───────────────────────────────────────────────────
log "Evaluating coverage against the ${THRESHOLD}% threshold"
set +e
python3 - "$COBERTURA" "$THRESHOLD" "$OUT_DIR/summary.md" <<'PY'
import os
import sys
import xml.etree.ElementTree as ET

cobertura, threshold, summary_path = sys.argv[1], float(sys.argv[2]), sys.argv[3]
root = ET.parse(cobertura).getroot()


def package_stats(package):
    covered = valid = 0
    for line in package.iter("line"):
        valid += 1
        if int(line.get("hits", "0") or "0") > 0:
            covered += 1
    return covered, valid


total_covered = int(root.get("lines-covered", 0) or 0)
total_valid = int(root.get("lines-valid", 0) or 0)
if total_valid == 0:
    for line in root.iter("line"):
        total_valid += 1
        if int(line.get("hits", "0") or "0") > 0:
            total_covered += 1

percent = (100.0 * total_covered / total_valid) if total_valid else 0.0

rows = []
for package in root.iter("package"):
    name = package.get("name") or "(unnamed)"
    covered, valid = package_stats(package)
    pct = (100.0 * covered / valid) if valid else 0.0
    rows.append((name, pct, covered, valid))
rows.sort(key=lambda row: (row[1], row[0]))

lines = ["", "| Package | Line coverage | Lines hit |", "| --- | --- | --- |"]
for name, pct, covered, valid in rows:
    flag = "PASS" if pct >= threshold else "FAIL"
    lines.append(f"| `{name}` | {flag} {pct:.2f}% | {covered}/{valid} |")
lines.append(
    f"| **Total** | {'PASS' if percent >= threshold else 'FAIL'} "
    f"**{percent:.2f}%** | {total_covered}/{total_valid} |"
)

report = (
    f"## Contract coverage — {'PASS' if percent >= threshold else 'FAIL'}\n\n"
    f"Aggregate line coverage: **{percent:.2f}%** "
    f"(threshold: {threshold:.0f}%)\n" + "\n".join(lines) + "\n"
)

print(report)
with open(summary_path, "w", encoding="utf-8") as handle:
    handle.write(report)

step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
if step_summary:
    with open(step_summary, "a", encoding="utf-8") as handle:
        handle.write(report)

if total_valid == 0:
    sys.exit(2)
sys.exit(0 if percent >= threshold else 1)
PY
GATE=$?
set -e

case "$GATE" in
  0)
    ok "Coverage gate passed."
    ;;
  2)
    die "No executable lines were measured — check that the selected packages actually contain tests."
    ;;
  *)
    printf '%sCoverage gate failed: aggregate line coverage is below %s%%.%s\n' "$RED" "$THRESHOLD" "$RESET" >&2
    printf '  HTML : %s/index.html\n  XML  : %s/cobertura.xml\n  LCOV : %s/lcov.info\n' "$OUT_DIR" "$OUT_DIR" "$OUT_DIR" >&2
    exit 1
    ;;
esac

echo
ok "Reports written to ${OUT_DIR}"
printf '  HTML : %s/index.html\n  XML  : %s/cobertura.xml\n  LCOV : %s/lcov.info\n' "$OUT_DIR" "$OUT_DIR" "$OUT_DIR"
