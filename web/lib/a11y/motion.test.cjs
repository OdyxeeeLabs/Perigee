// motion.test.cjs — reduced-motion decision logic (issue #702).
//
// Runs with: node --test ./lib/a11y/motion.test.cjs
//
// Mirrors web/lib/a11y/motion.ts in plain CommonJS, matching the style of
// lib/locale/locales.test.cjs and lib/a11y/contrast.test.cjs.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const REDUCED_LOADER_CUE = Object.freeze({
  spinMs: 3000, ariaBusy: true, requiresText: true, keepVisible: true,
});

const DISABLED_WHEN_REDUCED = new Set(["decorative", "transform", "layout"]);

function shouldAnimate(kind, preference) {
  if (preference !== "reduce") return true;
  return !DISABLED_WHEN_REDUCED.has(kind);
}

function loaderCue() {
  return REDUCED_LOADER_CUE;
}

function transitionDuration(baseMs, preference) {
  if (preference !== "reduce") return Math.max(0, baseMs);
  return 0.01;
}

function motionTransition(kind, base = { duration: 0.3 }, preference = null) {
  if (shouldAnimate(kind, preference)) {
    return base.delay === undefined ? base : { duration: base.duration, delay: base.delay };
  }
  return { duration: 0 };
}

// ── what gets disabled ──────────────────────────────────────────────────────

test("decorative and transform motion is dropped under reduce", () => {
  for (const kind of ["decorative", "transform", "layout"]) {
    assert.equal(shouldAnimate(kind, "reduce"), false, kind);
  }
});

test("communicating motion survives reduce", () => {
  // A progress bar that stops moving conveys nothing; a loader that stops
  // looks like a crash.
  assert.equal(shouldAnimate("loader", "reduce"), true);
  assert.equal(shouldAnimate("status", "reduce"), true);
});

test("nothing is disabled without the preference", () => {
  for (const kind of ["decorative", "transform", "layout", "loader", "status"]) {
    assert.equal(shouldAnimate(kind, "no-preference"), true, kind);
    assert.equal(shouldAnimate(kind, null), true, kind);
    assert.equal(shouldAnimate(kind, undefined), true, kind);
  }
});

// ── unresolved preference ──────────────────────────────────────────────────

test("an unresolved preference renders like the server, not like reduce", () => {
  // `null` arrives during SSR and the first client render. Treating it as
  // `reduce` would break hydration on every page.
  assert.equal(shouldAnimate("decorative", null), true);
  assert.equal(transitionDuration(300, null), 300);
  assert.equal(motionTransition("decorative", { duration: 0.3 }, null).duration, 0.3);
});

// ── loaders ────────────────────────────────────────────────────────────────

test("a loader keeps a non-motion cue under reduce", () => {
  const cue = loaderCue("reduce");
  assert.equal(cue.keepVisible, true, "a stopped ring reads as a broken UI");
  assert.equal(cue.ariaBusy, true);
  assert.equal(cue.requiresText, true, "progress must never be visual-only");
  assert.ok(cue.spinMs >= 1000, "a reduced spinner should not read as motion");
  assert.ok(cue.spinMs <= 5000, "and it must still visibly turn");
});

test("the loader cue is frozen so no caller can mutate the contract", () => {
  assert.throws(() => { "use strict"; REDUCED_LOADER_CUE.spinMs = 10; }, TypeError);
});

// ── durations ──────────────────────────────────────────────────────────────

test("transition durations collapse to the 0.01ms idiom under reduce", () => {
  // 0 and not 0: a zero-duration transition never fires `transitionend`, which
  // would leave completion listeners waiting forever.
  assert.equal(transitionDuration(300, "reduce"), 0.01);
  assert.equal(transitionDuration(0, "reduce"), 0.01);
  assert.equal(transitionDuration(300, "no-preference"), 300);
  assert.equal(transitionDuration(-50, "no-preference"), 0);
});

test("framer transitions collapse to duration 0", () => {
  assert.deepEqual(motionTransition("transform", { duration: 0.4 }, "reduce"), { duration: 0 });
  assert.deepEqual(
    motionTransition("decorative", { duration: 0.4, delay: 0.2 }, "reduce"),
    { duration: 0 },
  );
  assert.deepEqual(
    motionTransition("loader", { duration: 0.4, delay: 0.2 }, "reduce"),
    { duration: 0.4, delay: 0.2 },
    "a loader keeps its timing, including the delay",
  );
});

// ── the app must actually honour it ────────────────────────────────────────

const WEB_ROOT = path.join(__dirname, "..", "..");
const readWeb = (rel) => fs.readFileSync(path.join(WEB_ROOT, rel), "utf8");

test("the global stylesheet still neutralises CSS animation", () => {
  const css = readWeb(path.join("styles", "globals.css"));
  const block = /@media \(prefers-reduced-motion: reduce\) \{([\s\S]*?)\n\}/.exec(css);
  assert.ok(block, "no prefers-reduced-motion block in globals.css");
  const inner = block[1];
  assert.match(inner, /transition-duration: 0\.01ms !important/);
  assert.match(inner, /animation-duration: 0\.01ms !important/);
  // Smooth scrolling is a common unaddressed vestibular trigger.
  assert.match(inner, /scroll-behavior: auto !important/);
});

test("Framer Motion is configured to follow the user setting", () => {
  const provider = readWeb(path.join("components", "MotionProvider.tsx"));
  assert.match(provider, /reducedMotion="user"/);
});

test("the hook is wired for consumers that cannot use Framer", () => {
  const hook = readWeb(path.join("hooks", "usePrefersReducedMotion.ts"));
  assert.match(hook, /matchMedia\("\(prefers-reduced-motion: reduce\)"\)/);
  // The live OS switch has to be observed, not read once.
  assert.match(hook, /addEventListener\("change"/);
  assert.match(hook, /typeof window === "undefined"/, "SSR guard is required");
});