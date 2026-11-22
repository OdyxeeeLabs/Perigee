// live-region.test.cjs — announcement policy (issue #704).
//
// Runs with: node --test ./lib/a11y/live-region.test.cjs
//
// Mirrors web/lib/a11y/live-region.ts in plain CommonJS; the module under
// test is DOM-free so no jsdom is needed.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

// ── mirrored implementation ────────────────────────────────────────────────

function politenessFor(kind) {
  return kind === "error" ? "assertive" : "polite";
}

function formatElapsed(ms) {
  if (!Number.isFinite(ms) || ms < 0) return "0 seconds";
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return minutes > 0
      ? `${hours} hour${hours === 1 ? "" : "s"} ${minutes} minute${minutes === 1 ? "" : "s"}`
      : `${hours} hour${hours === 1 ? "" : "s"}`;
  }
  if (minutes > 0) {
    return seconds > 0 && minutes < 1
      ? `${seconds} second${seconds === 1 ? "" : "s"}`
      : `${minutes} minute${minutes === 1 ? "" : "s"}`;
  }
  return `${seconds} second${seconds === 1 ? "" : "s"}`;
}

function nextMilestoneFrom(announcedUpTo, stepMs, firstStepMs = stepMs) {
  if (stepMs <= 0) return null;
  if (announcedUpTo < firstStepMs) return firstStepMs;
  const steps = Math.floor(announcedUpTo / stepMs);
  return (steps + 1) * stepMs;
}

function shouldAnnounce(lastMessage, nextMessage) {
  if (lastMessage === null) return true;
  return lastMessage !== nextMessage;
}

function connectivityAnnouncement({ lastMessage, state, heldForMs, settleMs }) {
  const message =
    state === "offline"
      ? "You are offline. Some features are unavailable."
      : state === "degraded"
        ? "Connection is slow. Data may be out of date."
        : "Connection restored.";
  if (heldForMs < settleMs) return { announce: false, message };
  return { announce: shouldAnnounce(lastMessage, message), message };
}

function connectionState({ online, reachable }) {
  if (!online) return "offline";
  if (reachable === false) return "degraded";
  return "online";
}

// ── politeness ─────────────────────────────────────────────────────────────

test("only errors interrupt", () => {
  assert.equal(politenessFor("error"), "assertive");
  for (const kind of ["status", "connectivity", "confirmation"]) {
    assert.equal(politenessFor(kind), "polite", kind);
  }
});

// ── the flood problem ──────────────────────────────────────────────────────

test("a timer announces at milestones, never every tick", () => {
  // The argument is the last milestone announced, so each call is one step
  // ahead regardless of how often the interval fires.
  assert.equal(nextMilestoneFrom(0, 60_000), 60_000);
  assert.equal(nextMilestoneFrom(60_000, 60_000), 120_000);
  assert.equal(nextMilestoneFrom(120_000, 60_000), 180_000);
  assert.equal(nextMilestoneFrom(60_000, 5_000, 5_000), 65_000);
  assert.equal(nextMilestoneFrom(0, 5_000, 5_000), 5_000);
});

test("passing current time instead of the last milestone swallows an announcement", () => {
  // The trap the rename exists to prevent: at exactly 60s, "next" computed
  // from the current time is 120s, so the one-minute milestone never fires.
  assert.equal(nextMilestoneFrom(60_000, 60_000), 120_000);
  // The correct loop passes the previous milestone and waits for the clock.
  assert.equal(nextMilestoneFrom(0, 60_000), 60_000);
});

test("60 seconds of ticking yields exactly one announcement", () => {
  // Mirrors LiveTimer's loop exactly, including the announcedUpTo bookkeeping.
  // The regression this guards: an aria-live region updated once per second.
  const announced = [];
  let announcedUpTo = 0;
  for (let t = 0; t <= 60_000; t += 1000) {
    const milestone = nextMilestoneFrom(announcedUpTo, 60_000);
    if (milestone === null || t < milestone) continue;
    announcedUpTo = milestone;
    announced.push({ at: t, text: `running for ${milestone / 1000} minute` });
  }
  assert.equal(announced.length, 1);
  assert.deepEqual(announced[0], { at: 60_000, text: "running for 60 minute" });
});

test("ten minutes of ticking yields ten announcements, not 600", () => {
  const announced = [];
  let announcedUpTo = 0;
  for (let t = 0; t <= 600_000; t += 1000) {
    const milestone = nextMilestoneFrom(announcedUpTo, 60_000);
    if (milestone === null || t < milestone) continue;
    announcedUpTo = milestone;
    announced.push(t);
  }
  assert.equal(announced.length, 10);
  assert.deepEqual(announced, [60_000, 120_000, 180_000, 240_000, 300_000,
    360_000, 420_000, 480_000, 540_000, 600_000]);
});

test("the first milestone is not at zero", () => {
  // "0 seconds" is noise; nothing is announced until the first step.
  assert.equal(nextMilestoneFrom(0, 60_000), 60_000);
  assert.notEqual(nextMilestoneFrom(0, 60_000), 0);
});

test("a degenerate step disables milestones instead of looping forever", () => {
  // stepMs=0 would otherwise make Math.floor(x/0) = Infinity -> NaN.
  assert.equal(nextMilestoneFrom(0, 0), null);
  assert.equal(nextMilestoneFrom(5_000, 0), null);
  assert.equal(nextMilestoneFrom(0, 60_000, 5_000), 5_000);
});

// ── elapsed formatting ─────────────────────────────────────────────────────

test("elapsed time is formatted for speech", () => {
  assert.equal(formatElapsed(0), "0 seconds");
  assert.equal(formatElapsed(1_000), "1 second");
  assert.equal(formatElapsed(45_000), "45 seconds");
  assert.equal(formatElapsed(60_000), "1 minute");
  assert.equal(formatElapsed(180_000), "3 minutes");
  assert.equal(formatElapsed(3_600_000), "1 hour");
  assert.equal(formatElapsed(7_260_000), "2 hours 1 minute");
});

test("elapsed time degrades instead of rendering NaN", () => {
  assert.equal(formatElapsed(NaN), "0 seconds");
  assert.equal(formatElapsed(Infinity), "0 seconds");
  assert.equal(formatElapsed(-5_000), "0 seconds");
});

// ── the identical-message problem ──────────────────────────────────────────

test("the first announcement always fires", () => {
  // null = nothing announced yet, which is also how the region gets registered.
  assert.equal(shouldAnnounce(null, "Anything"), true);
});

test("an identical repeat is skipped by policy", () => {
  assert.equal(shouldAnnounce("Address copied", "Address copied"), false);
  assert.equal(shouldAnnounce("Analysis complete", "Analysis failed"), true);
});

// ── connectivity ───────────────────────────────────────────────────────────

test("connection state is derived from both signals", () => {
  assert.equal(connectionState({ online: false, reachable: null }), "offline");
  assert.equal(connectionState({ online: false, reachable: true }), "offline");
  assert.equal(connectionState({ online: true, reachable: false }), "degraded");
  assert.equal(connectionState({ online: true, reachable: null }), "online");
  assert.equal(connectionState({ online: true, reachable: true }), "online");
});

test("a state that has not settled is not announced", () => {
  const result = connectivityAnnouncement({
    lastMessage: null, state: "offline", heldForMs: 400, settleMs: 2000,
  });
  assert.equal(result.announce, false);
  assert.equal(result.message, "You are offline. Some features are unavailable.");
});

test("a settled state is announced", () => {
  const result = connectivityAnnouncement({
    lastMessage: "Connection restored.", state: "offline", heldForMs: 2500, settleMs: 2000,
  });
  assert.equal(result.announce, true);
});

test("the same state twice in a row is not announced twice", () => {
  const result = connectivityAnnouncement({
    lastMessage: "You are offline. Some features are unavailable.",
    state: "offline", heldForMs: 5000, settleMs: 2000,
  });
  assert.equal(result.announce, false);
});

test("a flap inside the settle window produces no announcements", () => {
  // The bug: offline -> online -> offline inside 2s produced three messages.
  const messages = [];
  let last = null;
  for (const [state, held] of [
    ["offline", 300], ["online", 300], ["offline", 300], ["online", 300],
  ]) {
    const r = connectivityAnnouncement({ lastMessage: last, state, heldForMs: held, settleMs: 2000 });
    if (r.announce) { messages.push(r.message); last = r.message; }
  }
  assert.deepEqual(messages, []);
});

test("recovery says something different from the failure", () => {
  const offline = connectivityAnnouncement({
    lastMessage: null, state: "offline", heldForMs: 3000, settleMs: 2000,
  }).message;
  const online = connectivityAnnouncement({
    lastMessage: offline, state: "online", heldForMs: 3000, settleMs: 2000,
  }).message;
  assert.notEqual(offline, online);
  assert.match(online, /restored/i);
});

// ── the app must actually use it ───────────────────────────────────────────

const WEB_ROOT = path.join(__dirname, "..", "..");
const readWeb = (rel) => fs.readFileSync(path.join(WEB_ROOT, rel), "utf8");

test("the existing regions render even when empty", () => {
  // A region that mounts with its message is frequently never announced.
  const live = readWeb(path.join("components", "LiveRegion.tsx"));
  assert.doesNotMatch(live, /message &&/, "regions must not be conditionally rendered");
  assert.match(live, /aria-live="polite"/);
  assert.match(live, /role="alert"/);
});

test("the timer never puts its per-second value in the live region", () => {
  const timer = readWeb(path.join("components", "LiveTimer.tsx"));
  // The visual readout must be aria-hidden: a value changing every second
  // inside a live region is the flood this issue exists to prevent.
  assert.match(timer, /aria-hidden="true"/);
  assert.match(timer, /nextMilestoneFrom/);
  assert.match(timer, /shouldAnnounce/);
});

test("the timer honours reduced motion", () => {
  const timer = readWeb(path.join("components", "LiveTimer.tsx"));
  assert.match(timer, /usePrefersReducedMotion/);
  assert.match(timer, /prefersReduced !== true && setElapsed|prefersReduced !== true && elapsed/);
});

test("the network region applies hysteresis", () => {
  const region = readWeb(path.join("components", "NetworkStatusRegion.tsx"));
  assert.match(region, /connectivityAnnouncement/);
  assert.match(region, /settleMs/);
  assert.match(region, /politenessFor/);
});

test("the announcer hook handles the identical-message case", () => {
  const hook = readWeb(path.join("hooks", "useLiveAnnouncer.ts"));
  // Clear-then-set: without it a repeat is silently dropped.
  assert.match(hook, /setMessage\(""\)/);
  assert.match(hook, /clearTimeout/);
  // A pending timer must not fire into an unmounted component.
  assert.match(hook, /useEffect\(\(\) => clearTimer/);
});