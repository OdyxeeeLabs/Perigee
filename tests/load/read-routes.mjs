#!/usr/bin/env node
/**
 * API-40 (#801): reproducible read-route load scenario for Perigee.
 *
 * This driver only issues GET requests. Seeding is handled separately by
 * `seed.py` against a disposable SQLite database. It refuses to target a
 * production-looking host unless explicitly allowed, and it requires an
 * explicit test environment name for non-local targets.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { performance } from "node:perf_hooks";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const API_BASE_URL = (process.env.API_BASE_URL || "http://localhost:8080").replace(/\/+$/, "");
const VUS = envInt("LOAD_TEST_VUS", 10);
const DURATION_MS = envInt("LOAD_TEST_DURATION_SECONDS", 30) * 1000;
const PAGE_SIZES = (process.env.LOAD_TEST_PAGE_SIZES || "10,25,50,100")
  .split(",")
  .map((value) => Number(value.trim()))
  .filter((value) => Number.isInteger(value) && value >= 1 && value <= 100);
const MAX_PAGES = envInt("LOAD_TEST_MAX_PAGES", 300);
const REQUEST_TIMEOUT_MS = envInt("LOAD_TEST_REQUEST_TIMEOUT_MS", 10_000);
const MANAGER_ID = process.env.LOAD_TEST_MANAGER_ID || "00000000-0000-4000-8000-000000000001";
const ADMIN_ADDRESS =
  process.env.LOAD_TEST_ADMIN_ADDRESS ||
  "GLOADTESTADMIN0000000000000000000000000000000000000000000000";
const OUTPUT_JSON = process.env.LOAD_TEST_OUTPUT_JSON || "";
const BASELINE_JSON = process.env.LOAD_TEST_BASELINE_JSON || "";
const ALLOW_REMOTE = process.env.LOAD_TEST_ALLOW_REMOTE === "true";
const TARGET_ENV = (process.env.LOAD_TEST_ENV || "").toLowerCase();

const ADMIN_MANAGER_ROUTE = {
  name: "vaults",
  path: (limit, cursor) => {
    const params = new URLSearchParams({ manager_id: MANAGER_ID, limit: String(limit) });
    if (cursor) params.set("cursor", cursor);
    return `/vaults?${params}`;
  },
  idField: "id",
};

const MANAGER_ROUTE = {
  name: "managers",
  path: (limit, cursor) => {
    const params = new URLSearchParams({ limit: String(limit) });
    if (cursor) params.set("cursor", cursor);
    return `/managers?${params}`;
  },
  idField: "id",
};

const FEE_ROUTE = {
  name: "fees/history",
  path: (limit, cursor) => {
    const params = new URLSearchParams({ limit: String(limit) });
    if (cursor) params.set("cursor", cursor);
    return `/fees/history?${params}`;
  },
  idField: "ledger_sequence",
};

const ROUTES = [ADMIN_MANAGER_ROUTE, MANAGER_ROUTE, FEE_ROUTE];

function envInt(name, fallback) {
  const raw = process.env[name];
  if (!raw) return fallback;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return parsed;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function assertSafeTarget() {
  const url = new URL(API_BASE_URL);
  const host = url.hostname.toLowerCase();
  const isLocal = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(host);
  const appEnv = (process.env.APP_ENV || "").toLowerCase();

  assert(!["prod", "production"].includes(appEnv), "APP_ENV=production is not a load-test target");
  assert(!["prod", "production"].includes(TARGET_ENV), "LOAD_TEST_ENV=production is not allowed");
  if (!isLocal) {
    assert(ALLOW_REMOTE, "remote targets require LOAD_TEST_ALLOW_REMOTE=true");
    assert(
      ["test", "staging", "dev", "development"].includes(TARGET_ENV),
      "remote targets require LOAD_TEST_ENV=test|staging|dev|development",
    );
  }
  assert(PAGE_SIZES.length > 0, "LOAD_TEST_PAGE_SIZES must contain at least one value in 1..100");
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async function request(method, path, { token, body } = {}) {
  const headers = { Accept: "application/json" };
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const started = performance.now();
  let response;
  try {
    response = await fetch(`${API_BASE_URL}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
  } catch (error) {
    return { status: 0, body: null, ms: performance.now() - started, error: String(error) };
  }
  const text = await response.text();
  let parsed = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = text;
  }
  return { status: response.status, body: parsed, ms: performance.now() - started, bytes: text.length };
}

/** Dev/test auth: the zero signature is accepted when no real signing key is configured. */
async function obtainToken(address) {
  const challenge = await request("POST", "/auth/challenge", { body: { stellar_address: address } });
  assert(
    challenge.status === 200 || challenge.status === 201,
    `challenge failed: ${challenge.status} ${JSON.stringify(challenge.body)}`,
  );
  const challengeValue = challenge.body?.challenge ?? challenge.body?.data?.challenge ?? challenge.body;
  const verify = await request("POST", "/auth/verify", {
    body: { stellar_address: address, challenge: challengeValue, signature: "00".repeat(64) },
  });
  assert(
    verify.status === 200 || verify.status === 201,
    `verify failed: ${verify.status} ${JSON.stringify(verify.body)}`,
  );
  const token = verify.body?.token ?? verify.body?.access_token ?? verify.body?.data?.token;
  assert(typeof token === "string" && token.length > 0, "verify response did not contain a token");
  return token;
}

// ---------------------------------------------------------------------------
// Pagination contract checks (read-only)
// ---------------------------------------------------------------------------

function pageQuery(route, limit, cursor, includeTotal = false) {
  const base = route.path(limit, cursor);
  if (!includeTotal) return base;
  return `${base}${base.includes("?") ? "&" : "?"}include_total=true`;
}

/** Verify a page-specific response obeys the shared pagination contract. */
function checkPage(route, limit, body, seen) {
  assert(body && typeof body === "object", `${route.name}: response is not an object`);
  assert(Array.isArray(body.data), `${route.name}: response has no data array`);
  assert(body.data.length <= limit, `${route.name}: page returned ${body.data.length} > limit ${limit}`);
  const pagination = body.pagination;
  assert(pagination && typeof pagination === "object", `${route.name}: missing pagination object`);
  assert(typeof pagination.has_more === "boolean", `${route.name}: has_more is not boolean`);

  if (pagination.has_more) {
    assert(
      typeof pagination.next_cursor === "string" && pagination.next_cursor.length > 0,
      `${route.name}: has_more=true without next_cursor`,
    );
  } else {
    assert(pagination.next_cursor == null, `${route.name}: has_more=false with a next_cursor`);
  }

  for (const row of body.data) {
    const id = row?.[route.idField] ?? row?.id;
    assert(id !== undefined, `${route.name}: row is missing ${route.idField}`);
    const key = String(id);
    assert(!seen.has(key), `${route.name}: duplicate row ${key} across pages (keyset is not bounded)`);
    seen.add(key);
  }
}

/** Walk every page for a route/limit combination and assert termination. */
async function assertPaginationBounded(route, limit, token) {
  const first = await request("GET", pageQuery(route, limit, null, true), { token });
  assert(first.status === 200, `${route.name}: first page failed: ${first.status}`);
  const total = first.body?.pagination?.total_count;
  assert(Number.isInteger(total), `${route.name}: include_total did not return an integer total_count`);

  const expectedPages = Math.max(1, Math.ceil(total / limit));
  assert(expectedPages <= MAX_PAGES, `${route.name}: ${expectedPages} pages exceeds MAX_PAGES=${MAX_PAGES}`);

  const seen = new Set();
  let cursor = null;
  for (let page = 0; page < expectedPages; page += 1) {
    const isFirst = page === 0;
    const response = await request(
      "GET",
      pageQuery(route, limit, cursor, isFirst),
      { token },
    );
    assert(response.status === 200, `${route.name}: page ${page + 1} failed: ${response.status}`);
    checkPage(route, limit, response.body, seen);
    assert(
      response.body.pagination.has_more === page + 1 < expectedPages,
      `${route.name}: has_more disagrees with total_count on page ${page + 1}`,
    );
    cursor = response.body.pagination.next_cursor;
  }

  assert(seen.size === total, `${route.name}: walked ${seen.size} rows but total_count=${total}`);
  assert(cursor === null, `${route.name}: last page still exposes a next_cursor`);
  return { pages: expectedPages, rows: seen.size, total };
}

/** limit=101 must be rejected, never silently clamped. */
async function assertLimitIsBounded(route, token) {
  const response = await request("GET", pageQuery(route, 101, null), { token });
  assert(response.status === 400, `${route.name}: limit=101 returned ${response.status}, expected 400`);
  assert(
    JSON.stringify(response.body).includes("limit_too_large"),
    `${route.name}: limit=101 body lacks limit_too_large`,
  );
}

// ---------------------------------------------------------------------------
// Load phase
// ---------------------------------------------------------------------------

const SAMPLES = new Map();
const FAILURES = new Map();
const BYTES = new Map();

function record(name, result) {
  if (!SAMPLES.has(name)) SAMPLES.set(name, []);
  SAMPLES.get(name).push(result.ms);
  BYTES.set(name, (BYTES.get(name) || 0) + (result.bytes || 0));
  if (result.status !== 200) {
    FAILURES.set(name, (FAILURES.get(name) || 0) + 1);
  }
}

async function virtualUser(vuIndex, token, deadline) {
  const cursors = new Map();
  let step = vuIndex;
  while (Date.now() < deadline) {
    const route = ROUTES[step % ROUTES.length];
    const limit = PAGE_SIZES[Math.floor(step / ROUTES.length) % PAGE_SIZES.length];
    const key = `${route.name}:${limit}`;
    const cursor = cursors.get(key) || null;
    const result = await request("GET", pageQuery(route, limit, cursor), { token });
    record(route.name, result);
    if (result.status === 200) {
      if (result.body?.pagination?.has_more && result.body.pagination.next_cursor) {
        cursors.set(key, result.body.pagination.next_cursor);
      } else {
        cursors.delete(key); // restart from page one after the last page
      }
    } else {
      cursors.delete(key);
    }
    step += 1;
  }
}

function percentile(sorted, fraction) {
  if (sorted.length === 0) return null;
  const index = Math.min(sorted.length - 1, Math.ceil((fraction / 100) * sorted.length) - 1);
  return Number(sorted[Math.max(0, index)].toFixed(2));
}

function summarise(elapsedMs) {
  const routes = {};
  let totalRequests = 0;
  let totalFailures = 0;
  for (const route of ROUTES) {
    const samples = [...(SAMPLES.get(route.name) || [])].sort((a, b) => a - b);
    const failures = FAILURES.get(route.name) || 0;
    const count = samples.length;
    totalRequests += count;
    totalFailures += failures;
    routes[route.name] = {
      requests: count,
      failures,
      throughput_rps: Number((count / (elapsedMs / 1000)).toFixed(2)),
      latency_ms: {
        p50: percentile(samples, 50),
        p95: percentile(samples, 95),
        p99: percentile(samples, 99),
      },
      bytes: BYTES.get(route.name) || 0,
    };
  }
  return {
    generated_at: new Date().toISOString(),
    api_base_url: API_BASE_URL,
    target_env: TARGET_ENV || "local",
    vus: VUS,
    duration_seconds: Number((elapsedMs / 1000).toFixed(2)),
    page_sizes: PAGE_SIZES,
    total_requests: totalRequests,
    total_failures: totalFailures,
    overall_throughput_rps: Number((totalRequests / (elapsedMs / 1000)).toFixed(2)),
    routes,
  };
}

function printSummary(summary) {
  console.log("\nAPI read-route baseline");
  console.log("======================");
  console.log(
    `${summary.target_env} | ${summary.vus} VUs | ${summary.duration_seconds}s | ` +
      `${summary.total_requests} requests | ${summary.total_failures} failures`,
  );
  console.log(
    "overall throughput: " +
      `${summary.overall_throughput_rps} req/s | ` +
      `page sizes: ${summary.page_sizes.join(", ")}`,
  );
  console.table(
    Object.entries(summary.routes).map(([name, metrics]) => ({
      route: name,
      rps: metrics.throughput_rps,
      p50_ms: metrics.latency_ms.p50,
      p95_ms: metrics.latency_ms.p95,
      p99_ms: metrics.latency_ms.p99,
      failures: metrics.failures,
    })),
  );
  if (BASELINE_JSON) {
    const baseline = JSON.parse(readFileSync(BASELINE_JSON, "utf8"));
    for (const [name, metrics] of Object.entries(summary.routes)) {
      const prior = baseline.routes?.[name]?.latency_ms?.p95;
      if (typeof prior === "number" && metrics.latency_ms.p95 !== null) {
        const delta = ((metrics.latency_ms.p95 - prior) / prior) * 100;
        console.log(`${name}: p95 ${delta >= 0 ? "+" : ""}${delta.toFixed(1)}% vs baseline`);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

async function main() {
  assertSafeTarget();
  const token = await obtainToken(ADMIN_ADDRESS);

  console.log("Checking pagination bounds (read-only) ...");
  const pagination = {};
  for (const route of ROUTES) {
    for (const limit of PAGE_SIZES) {
      await assertLimitIsBounded(route, token);
      pagination[`${route.name}:${limit}`] = await assertPaginationBounded(route, limit, token);
      console.log(
        `  ${route.name} limit=${limit}: ` +
          `${pagination[`${route.name}:${limit}`].pages} pages, ` +
          `${pagination[`${route.name}:${limit}`].rows} rows`,
      );
    }
  }

  console.log(`Running load: ${VUS} VUs for ${DURATION_MS / 1000}s ...`);
  const deadline = Date.now() + DURATION_MS;
  const started = performance.now();
  await Promise.all(
    Array.from({ length: VUS }, (_, index) => virtualUser(index, token, deadline)),
  );
  const elapsed = performance.now() - started;

  const summary = summarise(elapsed);
  summary.pagination_checks = pagination;
  printSummary(summary);
  if (OUTPUT_JSON) {
    writeFileSync(OUTPUT_JSON, `${JSON.stringify(summary, null, 2)}\n`);
    console.log(`wrote ${OUTPUT_JSON}`);
  }
  if (summary.total_failures > 0) {
    throw new Error(`${summary.total_failures} requests failed; see per-route counts above`);
  }
}

main().catch((error) => {
  console.error(error.message || error);
  process.exitCode = 1;
});
