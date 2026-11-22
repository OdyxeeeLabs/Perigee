#!/usr/bin/env python3
"""Run the contract resource-budget report and compare it with a baseline.

The report comes from `cargo test -p perigee-resource-budget -- --nocapture`,
which prints one line per measured operation:

    RESOURCE|<contract>|<operation>|cpu=<instructions>|mem=<bytes>

Those numbers come from the Soroban host's deterministic cost model, never from
a clock, so the comparison is not subject to timing noise.

Usage:
    python3 scripts/resource_budget_guard.py [--baseline PATH] [--tolerance PCT]
                                             [--update] [--input FILE]

Exit codes:
    0  every operation is within tolerance (new operations are only reported)
    1  one or more operations exceed the baseline by more than the tolerance,
       or the report could not be produced
"""
import argparse
import json
import os
import subprocess
import sys

DEFAULT_BASELINE = "contracts/resource_budget/baseline.json"
DEFAULT_TOLERANCE = 5.0  # percent; counts are deterministic, so this is generous
CMD = ["cargo", "test", "-p", "perigee-resource-budget", "--", "--nocapture", "--test-threads=1"]


def parse(text):
    """Return {"contract::operation": {"cpu": int, "mem": int}}."""
    results = {}
    for line in text.splitlines():
        if not line.startswith("RESOURCE|"):
            continue
        parts = line.strip().split("|")
        if len(parts) != 5:
            continue
        _, contract, operation, cpu, mem = parts
        try:
            results[f"{contract}::{operation}"] = {
                "cpu": int(cpu.removeprefix("cpu=")),
                "mem": int(mem.removeprefix("mem=")),
            }
        except ValueError:
            print(f"warn: could not parse line: {line}", file=sys.stderr)
    return results


def load_baseline(path):
    if not os.path.exists(path):
        return {}
    with open(path) as f:
        return json.load(f).get("operations", {})


def pct(new, old):
    return 0.0 if old == 0 else (new - old) * 100.0 / old


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", default=DEFAULT_BASELINE)
    ap.add_argument("--tolerance", type=float, default=DEFAULT_TOLERANCE)
    ap.add_argument("--update", action="store_true", help="write the measured numbers as the new baseline")
    ap.add_argument("--input", help="read a saved report instead of running cargo")
    args = ap.parse_args()

    if args.input:
        with open(args.input) as f:
            text = f.read()
    else:
        proc = subprocess.run(CMD, capture_output=True, text=True)
        text = proc.stdout + "\n" + proc.stderr
        if proc.returncode != 0:
            print(text)
            print("error: the resource-budget tests failed to run", file=sys.stderr)
            return 1

    current = parse(text)
    if not current:
        print("error: no RESOURCE lines found in the report", file=sys.stderr)
        return 1

    baseline = load_baseline(args.baseline)
    failures = []
    rows = []
    for name in sorted(current):
        cur = current[name]
        base = baseline.get(name)
        if base is None:
            rows.append((name, cur["cpu"], "-", "-", cur["mem"], "-", "-", "NEW"))
            continue
        d_cpu = pct(cur["cpu"], base["cpu"])
        d_mem = pct(cur["mem"], base["mem"])
        status = "ok"
        if d_cpu > args.tolerance or d_mem > args.tolerance:
            status = "REGRESSION"
            failures.append((name, d_cpu, d_mem))
        rows.append((name, cur["cpu"], base["cpu"], f"{d_cpu:+.1f}%", cur["mem"], base["mem"], f"{d_mem:+.1f}%", status))

    header = ("contract::operation", "cpu", "base", "Δcpu", "mem", "base", "Δmem", "status")
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    table = "\n".join(lines)
    print(table)

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write("## Contract resource budget\n\n" + table + "\n")

    removed = sorted(set(baseline) - set(current))
    if removed:
        print("\nnote: baseline entries no longer measured: " + ", ".join(removed))

    if args.update:
        os.makedirs(os.path.dirname(args.baseline), exist_ok=True)
        with open(args.baseline, "w") as f:
            json.dump({"operations": current}, f, indent=2, sort_keys=True)
            f.write("\n")
        print(f"\nbaseline written to {args.baseline}")
        return 0

    if failures:
        print("\nResource budget regressions (tolerance %.1f%%):" % args.tolerance)
        for name, d_cpu, d_mem in failures:
            print(f"  {name}: cpu {d_cpu:+.1f}%, mem {d_mem:+.1f}%")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
