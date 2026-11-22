# API read-route load baseline (API-40, #801)

This directory contains a small, reproducible load scenario for the three
critical read routes:

| Route | Auth | Notes |
| --- | --- | --- |
| `GET /vaults?manager_id=…` | admin JWT | tenant-scoped vault list |
| `GET /managers` | admin JWT | manager directory |
| `GET /fees/history` | public | fee ledger history |

It is **non-production only**. The driver issues `GET` requests exclusively;
all synthetic rows are written by `seed.py` to a disposable local SQLite file.

## Files

| File | Purpose |
| --- | --- |
| `seed.py` | Populates a disposable SQLite DB with synthetic managers, vaults, and fee samples. |
| `read-routes.mjs` | Pagination contract checks + load driver; emits throughput and latency percentiles. |

## Safety

- The driver refuses `APP_ENV=production` and `LOAD_TEST_ENV=production`.
- Remote (non-loopback) targets additionally require
  `LOAD_TEST_ALLOW_REMOTE=true` **and** an explicit
  `LOAD_TEST_ENV=test|staging|dev|development`.
- `seed.py` refuses database paths that look like production unless `--force`
  is passed, and refuses `APP_ENV=production`.
- Seeded identities use `example.invalid` email addresses and fixed
  `GLOADTEST…` Stellar placeholders. No real credentials or personal data are
  used.

## Setup

Requires Node.js 18+ and Python 3.8+. No third-party packages.

```bash
# 1. Create a disposable database and seed it.
python3 tests/load/seed.py \
  --db loadtest-perigee.db \
  --vaults 2000 --managers 200 --fee-samples 1000
# The command prints LOAD_TEST_MANAGER_ID and LOAD_TEST_ADMIN_ADDRESS; use them below.

# 2. Start the API against that database (from the repo root).
PUBLIC_API_RATE_LIMIT_ENABLED=false \
PERIGEE_ADMIN_STELLAR_ADDRESSES='GLOADTESTADMIN0000000000000000000000000000000000000000000000' \
DATABASE_URL='sqlite://loadtest-perigee.db' \
PAGINATION_CURSOR_SECRET='loadtest-only-secret' \
  cargo run -p Perigee-core

# 3. Run the scenario.
LOAD_TEST_MANAGER_ID='00000000-0000-4000-8000-000000000001' \
LOAD_TEST_ADMIN_ADDRESS='GLOADTESTADMIN0000000000000000000000000000000000000000000000' \
LOAD_TEST_OUTPUT_JSON=tests/load/baseline.json \
  node tests/load/read-routes.mjs
```

`PAGINATION_Cursor_SECRET` is pinned so cursors survive the seed/restart
cycle; it is a throwaway value for the disposable database only.

### Environment variables

| Variable | Default | Description |
| --- | --- | --- |
| `API_BASE_URL` | `http://localhost:8080` | Target API. Must be loopback unless remote access is explicitly allowed. |
| `LOAD_TEST_ENV` | unset | `local`, `test`, `staging`, `dev`, or `development`. Required for remote targets. |
| `LOAD_TEST_ALLOW_REMOTE` | `false` | Set to `true` to permit a non-loopback target. |
| `LOAD_TEST_VUS` | `10` | Concurrent virtual users. |
| `LOAD_TEST_DURATION_SECONDS` | `30` | Measured load duration after the pagination checks. |
| `LOAD_TEST_PAGE_SIZES` | `10,25,50,100` | Comma-separated realistic page sizes (1-100). |
| `LOAD_TEST_MAX_PAGES` | `300` | Guardrail for the full page walk. |
| `LOAD_TEST_MANAGER_ID` | seeded admin manager id | `manager_id` used for `/vaults`. |
| `LOAD_TEST_ADMIN_ADDRESS` | seeded synthetic address | Stellar address used for dev auth. |
| `LOAD_TEST_REQUEST_TIMEOUT_MS` | `10000` | Per-request timeout. |
| `LOAD_TEST_OUTPUT_JSON` | unset | Optional path for the machine-readable baseline. |
| `LOAD_TEST_BASELINE_JSON` | unset | Optional prior baseline to print p95 deltas against. |

## What the scenario checks

Before measuring, the driver performs read-only contract checks for every
route × page size:

1. `limit=101` is rejected with HTTP 400 and `limit_too_large` (no silent clamp).
2. A `limit=N` response never contains more than `N` rows.
3. `include_total=true` returns an integer `total_count`.
4. Walking `ceil(total_count / N)` pages terminates with `has_more=false`,
   `next_cursor=null`, and no duplicate row IDs (keyset pagination is bounded).
5. Every response carries the common `data` / `pagination` envelope.

Only after all checks pass does the load phase run. Virtual users cycle through
all three routes and all page sizes, following cursors so deep pages are
exercised rather than repeatedly hitting page one.

## Recorded baseline

**Status: pending.** The harness itself has been validated end-to-end against
a local mock, but a real baseline cannot yet be recorded: `Perigee-core` does
not compile at the current `main` revision. The pre-existing failures are:

- `core/src/cache/mod.rs`: `pub mod redis;` points at a missing module.
- `core/src/fee_store.rs`: duplicated/unterminated statement in
  `batch_insert_samples`.
- `core/src/simulation.rs` and `core/src/stellar_service.rs`: additional
  unterminated delimiters.
- `core/src/main.rs`: duplicated router/layer fragments that are syntactically
  invalid.

These are unrelated to the read routes being measured and are intentionally
not repaired in this change. Once the build is green, run the setup above with
`LOAD_TEST_OUTPUT_JSON=tests/load/baseline.json`, commit that file, and fill in
the table:

| Route | Throughput (req/s) | p50 (ms) | p95 (ms) | p99 (ms) |
| --- | ---: | ---: | ---: | ---: |
| `vaults` | pending | | | |
| `managers` | pending | | | |
| `fees/history` | pending | | | |

To compare a future run with a recorded baseline:

```bash
LOAD_TEST_BASELINE_JSON=tests/load/baseline.json \
  node tests/load/read-routes.mjs
```

The summary prints per-route p95 deltas against the prior baseline.
