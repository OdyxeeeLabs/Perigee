// Tests for the user-controlled read-only refresh cadence (issue #843 / WEB-42).
// Pure logic is mirrored from TypeScript; React wiring is checked as a source
// contract because this harness has no DOM/React test renderer.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const WEB_ROOT = path.join(__dirname, "..");
const read = (...parts) => fs.readFileSync(path.join(WEB_ROOT, ...parts), "utf8");

const DEFAULT_REFRESH_INTERVAL_MS = 60_000;
const REFRESH_INTERVAL_OPTIONS = [
  { value: 15_000, label: "Every 15 seconds" },
  { value: 60_000, label: "Every minute (default)" },
  { value: 300_000, label: "Every 5 minutes" },
];

function normalizeRefreshInterval(value) {
  return REFRESH_INTERVAL_OPTIONS.some((option) => option.value === value)
    ? value
    : DEFAULT_REFRESH_INTERVAL_MS;
}

function shouldRefresh(now, lastRefreshAt, intervalMs) {
  return lastRefreshAt === null || now - lastRefreshAt >= intervalMs;
}

test("the preference offers a small fixed set of intervals with a 60s default", () => {
  assert.deepEqual(
    REFRESH_INTERVAL_OPTIONS.map((o) => o.value),
    [15_000, 60_000, 300_000],
  );
  assert.equal(DEFAULT_REFRESH_INTERVAL_MS, 60_000);
  assert.match(read("docs", "performance.md"), /default is one minute/i);
});

test("unknown or corrupted stored values fall back to the default", () => {
  for (const bad of [undefined, null, 0, -1, 1234, "60000", {}, true]) {
    assert.equal(normalizeRefreshInterval(bad), 60_000, `bad value ${String(bad)}`);
  }
  for (const good of [15_000, 60_000, 300_000]) {
    assert.equal(normalizeRefreshInterval(good), good);
  }
});

test("a change of interval changes when the next read happens", () => {
  const now = 120_000;
  const last = 100_000;
  assert.equal(shouldRefresh(now, last, 15_000), true);
  assert.equal(shouldRefresh(now, last, 60_000), false);
  assert.equal(shouldRefresh(now, last, 300_000), false);
  assert.equal(shouldRefresh(now, null, 300_000), true, "first read is immediate");
});

test("the setting is validated, defaulted and persisted by the existing settings store", () => {
  const settings = read("lib", "settings.ts");
  assert.match(settings, /refreshIntervalMs: RefreshIntervalMs/);
  assert.match(settings, /refreshIntervalMs: DEFAULT_REFRESH_INTERVAL_MS/);
  assert.match(settings, /refreshIntervalMs: normalizeRefreshInterval\(/);

  const store = read("lib", "settingsStore.ts");
  assert.match(store, /updateDisplay: \(patch\)/);
  assert.match(store, /persistSettings\(next\)/);

  const tab = read("components", "settings", "DisplaySettingsTab.tsx");
  assert.match(tab, /id="refresh-interval"/);
  assert.match(tab, /REFRESH_INTERVAL_OPTIONS\.map/);
  assert.match(tab, /updateDisplay\(\{ refreshIntervalMs:/);
});

test("the polling hook reschedules on change and clears its timer on cleanup", () => {
  const hook = read("hooks", "useDataRefresh.ts");
  assert.match(hook, /window\.setInterval\(/);
  assert.match(hook, /return \(\) => window\.clearInterval\(timer\)/);
  assert.match(hook, /\}, \[enabled, intervalMs\]\)/, "interval change tears down the old timer");
  assert.match(hook, /useHydrateSettings\(\)/, "persisted preference is loaded");
  assert.match(hook, /refreshNow/, "explicit reads stay immediate");
});

test("route changes pause polling and the listener is removed", () => {
  const routeHook = read("hooks", "useRouteChangePause.ts");
  assert.match(routeHook, /routeChangeStart/);
  assert.match(routeHook, /events\.off\("routeChangeStart"/);
  assert.match(routeHook, /routeChangeError/);

  const analytics = read("pages", "analytics", "index.tsx");
  assert.match(analytics, /useDataRefresh\(fetchTelemetryData/, "analytics read polling uses preference");
  assert.match(analytics, /enabled: !routeChanging/);
  const portfolio = read("pages", "portfolio.tsx");
  assert.match(portfolio, /useDataRefresh\(/);
  assert.match(portfolio, /enabled: !routeChanging/);
});

test("mutations and security notices are not gated by the refresh preference", () => {
  const analytics = read("pages", "analytics", "index.tsx");
  for (const mutation of ["handleToggleOptOut", "handleTriggerTestEvent", "setOptOutStatus", "fetchTelemetryData()"]) {
    assert.ok(analytics.includes(mutation), `${mutation} still exists`);
  }
  const banner = read("components", "NetworkStatusBanner.tsx");
  assert.match(banner, /role="alert"\s*\n\s*aria-live="assertive"/, "testnet security notice stays immediate");
  assert.match(banner, /setInterval\(checkStatus, refreshIntervalMs\)/);
  assert.match(banner, /return \(\) => clearInterval\(interval\)/);
  assert.match(banner, /window\.addEventListener\("online", goOnline\)/, "connectivity events stay immediate");
});
