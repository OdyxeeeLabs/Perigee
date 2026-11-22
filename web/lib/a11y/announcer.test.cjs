// announcer.test.cjs — live-region announcement formatting and the
// verbosity preference (issue #697).
//
// Runs with: node --test ./lib/a11y/announcer.test.cjs
//
// Mirrors web/lib/a11y/announcer.ts in plain CommonJS, matching the style of
// __tests__/config.test.cjs.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const VERBOSITY_LEVELS = ["terse", "normal", "verbose"];
const DEFAULT_VERBOSITY = "normal";
const VERBOSITY_STORAGE_KEY = "perigee:a11y:announcer-verbosity";

function normalizeVerbosity(value) {
  return VERBOSITY_LEVELS.includes(value) ? value : DEFAULT_VERBOSITY;
}

function formatAnnouncement(message, verbosity = DEFAULT_VERBOSITY, options = {}) {
  const text = (message ?? "").trim();
  if (!text) return options.empty ?? "";

  if (verbosity === "verbose" && options.source) return `${text} (${options.source})`;
  if (verbosity !== "terse") return text;

  const limit = options.terseLimit ?? 80;
  if (text.length <= limit) return text.replace(/[.。!！]+$/u, "");

  const clipped = text.slice(0, limit);
  const lastSpace = clipped.lastIndexOf(" ");
  const head = lastSpace > limit * 0.5 ? clipped.slice(0, lastSpace) : clipped;
  return `${head.replace(/[.。!！,，]+$/u, "")}…`;
}

function shouldAnnounce(previous, next) {
  const value = (next ?? "").trim();
  if (!value) return false;
  return value !== (previous ?? "").trim();
}

function readStoredVerbosity(storage) {
  try {
    return normalizeVerbosity(storage?.getItem(VERBOSITY_STORAGE_KEY));
  } catch {
    return DEFAULT_VERBOSITY;
  }
}

function storeVerbosity(verbosity, storage) {
  try {
    storage?.setItem(VERBOSITY_STORAGE_KEY, normalizeVerbosity(verbosity));
  } catch {
    /* ignore */
  }
}

const fakeStorage = () => {
  const map = new Map();
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, v),
    map,
  };
};

// ── verbosity normalization ────────────────────────────────────────────────

test("normalizeVerbosity accepts the three levels", () => {
  for (const level of VERBOSITY_LEVELS) {
    assert.equal(normalizeVerbosity(level), level);
  }
});

test("normalizeVerbosity falls back to normal for anything unknown", () => {
  for (const bad of [undefined, null, "", "LOUD", 42, {}]) {
    assert.equal(normalizeVerbosity(bad), "normal");
  }
});

// ── formatting ─────────────────────────────────────────────────────────────

test("normal keeps the message verbatim", () => {
  assert.equal(formatAnnouncement("Balance updated", "normal"), "Balance updated");
});

test("terse drops terminal punctuation", () => {
  assert.equal(formatAnnouncement("Balance updated.", "terse"), "Balance updated");
});

test("terse keeps short messages whole", () => {
  assert.equal(formatAnnouncement("Saved", "terse"), "Saved");
});

test("terse clips long messages on a word boundary", () => {
  const message =
    "The portfolio rebalanced across seven positions and three liquidity pools while fees were accrued";
  const out = formatAnnouncement(message, "terse", { terseLimit: 40 });
  assert.ok(out.length <= 41, `too long: ${out}`);
  assert.ok(out.endsWith("…"), `expected an ellipsis: ${out}`);
  assert.ok(!out.includes("  "), "clipped on a word boundary");
});

test("terse respects a custom limit", () => {
  assert.equal(formatAnnouncement("abcdefghij", "terse", { terseLimit: 5 }), "abcde…");
});

test("verbose appends the source", () => {
  assert.equal(
    formatAnnouncement("Sync finished", "verbose", { source: "network" }),
    "Sync finished (network)",
  );
});

test("verbose without a source behaves like normal", () => {
  assert.equal(formatAnnouncement("Sync finished", "verbose"), "Sync finished");
});

test("empty messages are dropped unless an empty label is supplied", () => {
  assert.equal(formatAnnouncement("", "verbose"), "");
  assert.equal(formatAnnouncement("   ", "normal"), "");
  assert.equal(
    formatAnnouncement("", "terse", { empty: "Nothing to announce" }),
    "Nothing to announce",
  );
});

test("non-string input does not throw", () => {
  assert.equal(formatAnnouncement(undefined, "terse"), "");
  assert.equal(formatAnnouncement(null, "normal"), "");
});

// ── de-duplication ─────────────────────────────────────────────────────────

test("shouldAnnounce rejects empties and duplicates", () => {
  assert.equal(shouldAnnounce("Balance updated", "Balance updated"), false);
  assert.equal(shouldAnnounce("Balance updated", "  Balance updated  "), false);
  assert.equal(shouldAnnounce(null, ""), false);
  assert.equal(shouldAnnounce("Old", "Balance updated"), true);
  assert.equal(shouldAnnounce(undefined, "Balance updated"), true);
});

// ── persistence ────────────────────────────────────────────────────────────

test("verbosity survives a round trip through storage", () => {
  const storage = fakeStorage();
  storeVerbosity("terse", storage);
  assert.equal(readStoredVerbosity(storage), "terse");
});

test("an unset preference reads as normal", () => {
  assert.equal(readStoredVerbosity(fakeStorage()), "normal");
});

test("a corrupted stored value falls back to normal", () => {
  const storage = fakeStorage();
  storage.setItem(VERBOSITY_STORAGE_KEY, "loud");
  assert.equal(readStoredVerbosity(storage), "normal");
});

test("storage failures are swallowed", () => {
  const hostile = {
    getItem() {
      throw new Error("SecurityError");
    },
    setItem() {
      throw new Error("QuotaExceeded");
    },
  };
  assert.equal(readStoredVerbosity(hostile), "normal");
  assert.doesNotThrow(() => storeVerbosity("terse", hostile));
});

test("a missing storage object is tolerated", () => {
  assert.equal(readStoredVerbosity(null), "normal");
  assert.doesNotThrow(() => storeVerbosity("verbose", null));
});
