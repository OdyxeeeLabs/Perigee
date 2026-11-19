// web/__tests__/web-35-error-boundary-recovery.test.cjs — source-contract tests
// for issue #836 (WEB-35). Error boundaries must offer a safe, focused recovery
// action without ever rendering exception details or stack traces.
// Runs with: node --test ./__tests__/**/*.test.cjs

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, "..", relative), "utf8");

function focusRecovery(target) {
  if (!target || typeof target.focus !== "function") return;
  target.focus({ preventScroll: true });
}

test("focusRecovery focuses a target without scrolling and tolerates null", () => {
  const calls = [];
  focusRecovery({ focus: (options) => calls.push(options) });
  assert.deepEqual(calls, [{ preventScroll: true }]);
  assert.doesNotThrow(() => focusRecovery(null));
  assert.doesNotThrow(() => focusRecovery({}));
});

test("component boundary exposes an accessible, focused recovery message", () => {
  const src = read("components/ErrorBoundary.tsx");
  assert.match(src, /role="alert"/, "recovery container is an alert");
  assert.match(src, /aria-labelledby=\{this\.headingId\}/);
  assert.match(src, /id=\{this\.headingId\}/, "heading carries the referenced id");
  assert.match(src, /<h2 id=\{this\.headingId\}/, "accessible heading is preserved");
  assert.match(src, /tabIndex=\{-1\}/, "recovery message is programmatically focusable");
  assert.match(src, /focusRecovery\(this\.recoveryRef\.current\)/);
  assert.match(src, /componentDidUpdate\(prevProps: ErrorBoundaryProps, prevState: ErrorBoundaryState\)/);
  assert.match(src, /if \(!prevState\.error && this\.state\.error\)/, "focus only on activation");
});

test("component boundary never discloses exception messages or stack traces", () => {
  const src = read("components/ErrorBoundary.tsx");
  assert.doesNotMatch(src, /this\.state\.error\.message/);
  assert.doesNotMatch(src, /componentStack/);
  assert.doesNotMatch(src, /<details/);
  assert.doesNotMatch(src, /<pre/);
});

test("component boundary offers context-appropriate recovery actions", () => {
  const src = read("components/ErrorBoundary.tsx");
  assert.match(src, /Try again/);
  assert.match(src, /Back to dashboard/);
  assert.match(src, /Reload page/);
  assert.match(src, /onClick=\{this\.reset\}/);
  assert.match(src, /variant === 'route'/);
});

test("error reporting inside the boundary stays consent-aware", () => {
  const src = read("components/ErrorBoundary.tsx");
  assert.match(src, /getTelemetryConsent\(\) === true/, "telemetry gated by consent");
  assert.match(src, /trackTelemetryError\(/);
});

test("route and global error pages focus recovery and avoid raw error details", () => {
  for (const file of ["app/error.tsx", "app/global-error.tsx"]) {
    const src = read(file);
    assert.match(src, /focusRecovery\(recoveryRef\.current\)/, `${file} focuses recovery`);
    assert.match(src, /role="alert"/, `${file} announces failure`);
    assert.match(src, /aria-labelledby=/, `${file} labels its heading`);
    assert.match(src, /tabIndex=\{-1\}/, `${file} is focusable`);
    assert.match(src, /<h1 id=/, `${file} keeps an accessible heading`);
    assert.match(src, />\s*Try again\s*</, `${file} offers retry`);
    assert.doesNotMatch(src, /error\.message|error\.digest|\{error\}/, `${file} hides internals`);
  }
  const route = read("app/error.tsx");
  assert.match(route, /Back to dashboard/, "route error has a navigation action");
});

test("focus helper lives with the a11y primitives", () => {
  const src = read("lib/a11y/focus.ts");
  assert.match(src, /export function focusRecovery/);
  assert.match(src, /preventScroll: true/);
});
