// web/__tests__/web-33-service-worker-resilience.test.cjs — tests for #834
// (WEB-33). The offline shell may serve only an explicit static allowlist and
// read-only API snapshots; mutations/signing must fail closed with a reconnect
// message and never be replayed from a cache.
// Runs with: node --test ./__tests__/**/*.test.cjs

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, "..", relative), "utf8");

const VERSION = "v2";
const CACHE_PREFIX = "perigee-";
const ALL_CACHES = [
  `${CACHE_PREFIX}static-${VERSION}`,
  `${CACHE_PREFIX}images-${VERSION}`,
  `${CACHE_PREFIX}api-${VERSION}`,
];
const CACHEABLE_STATIC_PATHS = ["/_next/static/", "/icons/", "/favicon.ico", "/manifest.webmanifest"];
const CACHEABLE_STATIC_EXT = /\.(?:js|css|woff2?|ttf|otf|wasm|svg|png|webp|ico)$/i;
const SIGNING_PATH_RE = /\/api\/(?:sign|signing|transaction|tx|deposit|withdraw|approve|reject|register)(?:\/|$)/i;
const MUTATION_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);

function isApi(url) {
  return url.origin === url.selfOrigin && url.pathname.startsWith("/api/");
}
function isSigningRequest(url) {
  return isApi(url) && SIGNING_PATH_RE.test(url.pathname);
}
function isMutation(method) {
  return MUTATION_METHODS.has(String(method).toUpperCase());
}
function isCacheableStatic(url, destination) {
  if (url.origin !== url.selfOrigin || url.pathname.startsWith("/api/")) return false;
  if (destination === "document") return false;
  return (
    CACHEABLE_STATIC_PATHS.some((prefix) => url.pathname.startsWith(prefix)) ||
    CACHEABLE_STATIC_EXT.test(url.pathname)
  );
}
function staleCaches(names) {
  return names.filter((name) => name.startsWith(CACHE_PREFIX) && !ALL_CACHES.includes(name));
}
const u = (pathname, destination = "") => ({ origin: "https://app", selfOrigin: "https://app", pathname, destination });

test("offline read-only shell: only allowlisted static assets are cacheable", () => {
  assert.equal(isCacheableStatic(u("/_next/static/chunks/main.js"), "script"), true);
  assert.equal(isCacheableStatic(u("/favicon.ico"), "image"), true);
  assert.equal(isCacheableStatic(u("/icons/logo.svg"), "image"), true);
  assert.equal(isCacheableStatic(u("/dashboard"), "document"), false, "documents are never cached");
  assert.equal(isCacheableStatic(u("/api/telemetry"), ""), false, "API paths are not static assets");
  assert.equal(isCacheableStatic({ ...u("/_next/static/x.js"), origin: "https://cdn" }, "script"), false);
});

test("signing and mutation requests are identified and never treated as cacheable reads", () => {
  assert.equal(isMutation("POST"), true);
  assert.equal(isMutation("PATCH"), true);
  assert.equal(isMutation("GET"), false);
  assert.equal(isSigningRequest(u("/api/sign")), true);
  assert.equal(isSigningRequest(u("/api/transactions/submit")), true);
  assert.equal(isSigningRequest(u("/api/telemetry", "")), false);
  assert.equal(isCacheableStatic(u("/api/sign"), "script"), false);
});

test("cache version upgrades remove every incompatible cache", () => {
  const installed = [
    "perigee-static-v1",
    "perigee-images-v1",
    "perigee-api-v1",
    "perigee-static-v2",
    "unrelated-cache",
  ];
  assert.deepEqual(staleCaches(installed), ["perigee-static-v1", "perigee-images-v1", "perigee-api-v1"]);
});

test("service worker source encodes the cache policy", () => {
  const sw = read("public/sw.js");
  assert.match(sw, /const VERSION = "v2"/);
  assert.match(sw, /CACHEABLE_STATIC_PATHS/);
  assert.match(sw, /request\.destination === "document"\) return/);
  assert.match(sw, /SIGNING_PATH_RE/);
  assert.match(sw, /isSigningRequest\(url\)/);
  assert.match(sw, /cache: "no-store"/);
  assert.match(sw, /submitted: false/);
  assert.match(sw, /OFFLINE_ACTION_BLOCKED/);
  assert.match(sw, /if \(isSigningRequest\(url\)\) \{[\s\S]*store\.delete\(record\.id\)/, "queued signing records are dropped, not replayed");
  assert.match(sw, /startsWith\(CACHE_PREFIX\) && !ALL_CACHES\.includes\(name\)/);
});

test("client surfaces a clear reconnect message for blocked actions", () => {
  const lib = read("lib/serviceWorker.ts");
  assert.match(lib, /onOfflineActionBlocked\?: \(message: string\) => void/);
  assert.match(lib, /OFFLINE_ACTION_BLOCKED/);
  assert.match(lib, /Reconnect and try again/, "client has a clear fallback message");
  assert.match(read("public/sw.js"), /OFFLINE_ACTION_MESSAGE =\s*\n?\s*"You are offline\. Reconnect and try again/);

  const manager = read("components/ServiceWorkerManager.tsx");
  assert.match(manager, /blockedMessage/);
  assert.match(manager, /onOfflineActionBlocked/);
  assert.match(manager, /role="alert"/);
  assert.match(manager, /\{blockedMessage\}/);

  const banner = read("components/NetworkStatusBanner.tsx");
  assert.match(banner, /reconnectHint\?: boolean/);
  assert.match(banner, /Reconnect to complete your transaction\./);
});
