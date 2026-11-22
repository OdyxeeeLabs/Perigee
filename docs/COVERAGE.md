# Contract test coverage

Line coverage for the Soroban contracts is generated with
[`cargo-tarpaulin`](https://github.com/xd009642/tarpaulin) — the same tool the
[fuzz workflow](../.github/workflows/fuzz.yml) already uses — and enforced in
CI by the [Contracts — coverage](../.github/workflows/contracts-coverage.yml)
workflow.

The gate is **80% aggregate line coverage** by default and fails the build when
coverage drops below it.

## Running locally

```bash
# 80% gate, reports written to ./coverage-report/
bash scripts/coverage.sh

# raise/lower the bar for this run
COVERAGE_THRESHOLD=90 bash scripts/coverage.sh

# measure a subset of crates
COVERAGE_PACKAGES="perigee-bootstrap Perigee-math" bash scripts/coverage.sh
```

Install the tool once:

```bash
cargo install cargo-tarpaulin --locked
```

`cargo-tarpaulin` only produces reliable results on Linux x86-64. On other
platforms run the script inside the CI container (or a Linux VM/container)
instead of expecting local numbers.

## Reports

| File | Format | Use |
| --- | --- | --- |
| `coverage-report/cobertura.xml` | Cobertura XML | Machine readable; the CI gate reads this file |
| `coverage-report/lcov.info` | LCOV | Editors, Codecov, `genhtml` |
| `coverage-report/index.html` | HTML | Human readable, open in a browser |
| `coverage-report/summary.md` | Markdown | Per-package table, also posted to the workflow step summary |

## Configuration

Every knob is an environment variable so CI, scripts and local runs share one
implementation:

| Variable | Default | Meaning |
| --- | --- | --- |
| `COVERAGE_THRESHOLD` | `80` | Minimum aggregate line coverage percentage |
| `COVERAGE_PACKAGES` | auto-discovered | Space separated crate names to measure |
| `COVERAGE_OUTPUT_DIR` | `coverage-report` | Where reports are written |
| `COVERAGE_PROFILE` | `coverage` | Cargo profile used for the build (`""` for tarpaulin's default) |
| `COVERAGE_TIMEOUT` | `300` | Per-test timeout in seconds |
| `COVERAGE_EXCLUDE_FILES` | unset | Optional `tarpaulin --exclude-files` patterns |
| `COVERAGE_TARPAULIN` | auto-detected | Path to the tarpaulin binary |

In CI, set the repository variable `CONTRACT_COVERAGE_THRESHOLD`
(Settings → Secrets and variables → Actions → Variables) to change the gate
without editing the workflow.

## Scope

By default the script measures **every contract crate that is a member of the
Cargo workspace** — currently `perigee-bootstrap`, `Perigee-error-codes`,
`Perigee-math`, `Perigee-guards`, `soroban-staking-rewards` and
`oracle-aggregator`. Membership is discovered at runtime via
`cargo metadata`, so adding a contract to the workspace `members` list is
enough to have it covered.

Contract directories under `contracts/` that are standalone Cargo projects
(not workspace members) cannot be built from this workspace; the script lists
them as skipped so the gap is visible rather than silent. Pass
`COVERAGE_PACKAGES` to measure a specific set instead.

## How the gate works

1. `scripts/coverage.sh` discovers the contract crates and runs tarpaulin once
   over all of them, so the reports are a single merged view.
2. The script parses `lines-covered` / `lines-valid` from the Cobertura report,
   prints a per-package table, and writes `summary.md`.
3. If the aggregate percentage is below `COVERAGE_THRESHOLD` the script exits
   non-zero, which fails the CI job. The report is still uploaded as an
   artefact so the failure can be diagnosed.
