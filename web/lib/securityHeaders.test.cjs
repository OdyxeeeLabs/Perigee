// web/lib/securityHeaders.test.cjs
//
// CJS replication of the contract enforced in web/lib/securityHeaders.ts. The
// production source is TypeScript, but per `package.json`'s `test` script only
// `.cjs` files are executed by `node --test`, so the policy is mirrored here to
// keep it enforceable. `web/vercel.json` is read at the bottom so the edge
// config and the app policy are asserted to agree.
//
// If you change behaviour in web/lib/securityHeaders.ts, mirror it here.
// Closes OdyxeeeLabs/Perigee#624

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

// ── Mirror of web/lib/securityHeaders.ts ────────────────────────────────────

const HSTS_MAX_AGE_SECONDS = 31_536_000;

const CSP_DIRECTIVES = [
  "default-src 'self'",
  "script-src 'self' 'nonce-{NONCE}' 'strict-dynamic' https:",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob: https:",
  "font-src 'self' data:",
  "connect-src 'self' https://*.stellar.org https://*.soroban.network wss://*.stellar.org",
  "frame-ancestors 'none'",
  "form-action 'self'",
  "base-uri 'self'",
  "object-src 'none'",
  "manifest-src 'self'",
  "worker-src 'self' blob:",
];

const STATIC_SECURITY_HEADERS = {
  "X-Content-Type-Options": "nosniff",
  "X-Frame-Options": "DENY",
  "Referrer-Policy": "strict-origin-when-cross-origin",
  "Permissions-Policy": [
    "geolocation=()",
    "camera=()",
    "microphone=()",
    "payment=()",
    "usb=()",
    "magnetometer=()",
    "interest-cohort=()",
  ].join(", "),
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
  "Cross-Origin-Resource-Policy": "same-origin",
  "Strict-Transport-Security": `max-age=${HSTS_MAX_AGE_SECONDS}; includeSubDomains; preload`,
  "X-DNS-Prefetch-Control": "off",
};

function buildCsp(nonce) {
  return CSP_DIRECTIVES.map((directive) =>
    directive.replace("{NONCE}", nonce)
  ).join("; ");
}

function buildSecurityHeaders(nonce) {
  return { ...STATIC_SECURITY_HEADERS, "Content-Security-Policy": buildCsp(nonce) };
}

// ── Tests ───────────────────────────────────────────────────────────────────

test("CSP uses strict-dynamic with a per-request nonce", () => {
  const csp = buildCsp("TESTNONCE123");
  assert.match(csp, /script-src [^;]*'nonce-TESTNONCE123'/);
  assert.match(csp, /script-src [^;]*'strict-dynamic'/);
});

test("script-src no longer allows unsafe-inline", () => {
  const scriptSrc = CSP_DIRECTIVES.find((d) => d.startsWith("script-src"));
  assert.ok(scriptSrc, "script-src directive is missing");
  assert.ok(
    !scriptSrc.includes("'unsafe-inline'"),
    "unsafe-inline in script-src defeats the nonce"
  );
});

test("frame-ancestors is none, not self", () => {
  assert.ok(CSP_DIRECTIVES.includes("frame-ancestors 'none'"));
  assert.ok(!CSP_DIRECTIVES.includes("frame-ancestors 'self'"));
});

test("object-src and base-uri are locked down", () => {
  assert.ok(CSP_DIRECTIVES.includes("object-src 'none'"));
  assert.ok(CSP_DIRECTIVES.includes("base-uri 'self'"));
});

test("connect-src names origins instead of wildcarding https", () => {
  const connectSrc = CSP_DIRECTIVES.find((d) => d.startsWith("connect-src"));
  assert.ok(connectSrc);
  assert.ok(!connectSrc.includes("https://*"), "connect-src must not allow any HTTPS origin");
  assert.ok(connectSrc.includes("https://*.stellar.org"));
});

test("every script-src nonce is substituted, none left as a literal placeholder", () => {
  const csp = buildCsp("abc123");
  assert.ok(!csp.includes("{NONCE}"), "unsubstituted nonce placeholder leaked into the policy");
});

test("HSTS is at least one year and preloaded", () => {
  const hsts = STATIC_SECURITY_HEADERS["Strict-Transport-Security"];
  assert.ok(
    hsts.includes(`max-age=${HSTS_MAX_AGE_SECONDS}`),
    "HSTS max-age must be >= 31536000 for preload eligibility"
  );
  assert.ok(hsts.includes("includeSubDomains"));
  assert.ok(hsts.includes("preload"));
});

test("X-Frame-Options is DENY", () => {
  assert.equal(STATIC_SECURITY_HEADERS["X-Frame-Options"], "DENY");
});

test("Referrer-Policy is strict-origin-when-cross-origin", () => {
  assert.equal(
    STATIC_SECURITY_HEADERS["Referrer-Policy"],
    "strict-origin-when-cross-origin"
  );
});

test("Permissions-Policy denies the dangerous device APIs", () => {
  const policy = STATIC_SECURITY_HEADERS["Permissions-Policy"];
  for (const feature of ["geolocation", "camera", "microphone", "payment"]) {
    assert.ok(policy.includes(`${feature}=()`), `${feature} must be denied`);
  }
});

test("cross-origin isolation headers are present", () => {
  assert.equal(STATIC_SECURITY_HEADERS["Cross-Origin-Opener-Policy"], "same-origin");
  assert.equal(STATIC_SECURITY_HEADERS["Cross-Origin-Embedder-Policy"], "require-corp");
  assert.equal(STATIC_SECURITY_HEADERS["Cross-Origin-Resource-Policy"], "same-origin");
});

test("buildSecurityHeaders merges the static set with the CSP", () => {
  const headers = buildSecurityHeaders("NONCE");
  assert.equal(headers["Content-Security-Policy"], buildCsp("NONCE"));
  assert.equal(headers["X-Content-Type-Options"], "nosniff");
  assert.ok("Strict-Transport-Security" in headers);
});

// ── vercel.json must agree with the app policy ──────────────────────────────

test("vercel.json carries the same static header policy as the app", () => {
  const vercelPath = path.join(__dirname, "..", "vercel.json");
  const vercel = JSON.parse(fs.readFileSync(vercelPath, "utf8"));

  const catchAll = vercel.headers.find((entry) => entry.source === "/(.*)");
  assert.ok(catchAll, "vercel.json needs a catch-all header block");

  const edge = Object.fromEntries(
    catchAll.headers.map((header) => [header.key.toLowerCase(), header.value])
  );

  // Headers that do not depend on a per-request nonce must match exactly, so
  // the edge and the app never disagree about what is enforced.
  for (const key of Object.keys(STATIC_SECURITY_HEADERS)) {
    const actual = edge[key.toLowerCase()];
    assert.ok(
      actual !== undefined,
      `vercel.json is missing ${key}; the edge would not enforce it`
    );
    assert.equal(
      actual,
      STATIC_SECURITY_HEADERS[key],
      `${key} differs between vercel.json and the app policy`
    );
  }

  // The edge cannot know a nonce, so CSP must not be pinned there — it comes
  // from the middleware instead.
  assert.equal(edge["content-security-policy"], undefined);
});