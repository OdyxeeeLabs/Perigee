// focus.test.cjs — focus-management primitives (issue #703).
//
// Runs with: node --test ./lib/a11y/focus.test.cjs
//
// Mirrors web/lib/a11y/focus.ts in plain CommonJS. The module under test is
// deliberately DOM-free, so these need no jsdom; the DOM adapters in
// hooks/useFocusTrap.ts are the only part that needs a real document.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

// ── mirrored implementation ────────────────────────────────────────────────

const FOCUSABLE_SELECTOR = [
  "a[href]", "area[href]", "button:not([disabled])", "input:not([disabled])",
  "select:not([disabled])", "textarea:not([disabled])", "summary", "iframe",
  "object", "embed", "audio[controls]", "video[controls]",
  "[contenteditable]:not([contenteditable='false'])", "[tabindex]",
].join(",");

function tabbability(item) {
  if (item.disabled) return "hidden";
  if (item.inert) return "inert";
  if (item.ariaHidden === true) return "aria-hidden";
  if (item.visible === false) return "hidden";
  if (item.tabIndex < 0) return "negative-tabindex";
  return null;
}

const isTabbable = (item) => tabbability(item) === null;

function orderByTabIndex(items) {
  const positive = [];
  const natural = [];
  items.forEach((item, index) => {
    if (item.tabIndex > 0) positive.push({ item, index });
    else natural.push({ item, index });
  });
  positive.sort((a, b) => a.item.tabIndex - b.item.tabIndex || a.index - b.index);
  return [...positive, ...natural].map((e) => e.item);
}

function nextFocusIndex(count, activeIndex, shiftKey) {
  if (count <= 0) return null;
  if (activeIndex < 0 || activeIndex >= count) return shiftKey ? count - 1 : 0;
  if (shiftKey) return activeIndex === 0 ? count - 1 : activeIndex - 1;
  return activeIndex === count - 1 ? 0 : activeIndex + 1;
}

function resolveInitialFocus(tabbables, requested, marked) {
  if (requested === "container" || (!requested && tabbables.length === 0)) return "container";
  if (typeof requested === "function") return requested() ?? "container";
  if (requested === "custom") return marked[0] ?? tabbables[0] ?? "container";
  return tabbables[0] ?? "container";
}

function shouldRestoreFocus(target) {
  if (!target) return false;
  if (!target.isConnected) return false;
  if (target.disabled) return false;
  if (target.tabIndex < 0) return false;
  if (target.visible === false) return false;
  return true;
}

function pickFocusRestoreTarget(opener, fallback) {
  if (shouldRestoreFocus(opener)) return opener;
  if (
    fallback &&
    fallback.isConnected &&
    !fallback.disabled &&
    fallback.visible !== false
  ) {
    return fallback;
  }
  return null;
}

// ── tests ──────────────────────────────────────────────────────────────────

test("a plain control is tabbable", () => {
  assert.equal(isTabbable({ tabIndex: 0 }), true);
});

test("tabbability reports why an element is skipped", () => {
  assert.equal(tabbability({ tabIndex: -1 }), "negative-tabindex");
  assert.equal(tabbability({ tabIndex: 0, visible: false }), "hidden");
  assert.equal(tabbability({ tabIndex: 0, disabled: true }), "hidden");
  assert.equal(tabbability({ tabIndex: 0, inert: true }), "inert");
  assert.equal(tabbability({ tabIndex: 0, ariaHidden: true }), "aria-hidden");
});

test("a hidden element is not tabbable even with a positive tabindex", () => {
  // The failure NavDrawer had: querySelectorAll returned these.
  assert.equal(isTabbable({ tabIndex: 3, visible: false }), false);
  assert.equal(isTabbable({ tabIndex: 0, ariaHidden: true }), false);
});

test("visibility is optional and defaults to tabbable", () => {
  // Not probing layout is the caller's choice; undefined must not mean hidden.
  assert.equal(isTabbable({ tabIndex: 0 }), true);
  assert.equal(isTabbable({ tabIndex: 0, visible: undefined }), true);
});

test("the selector covers contenteditable, summary and media controls", () => {
  for (const fragment of ["[contenteditable]", "summary", "audio[controls]", "video[controls]"]) {
    assert.ok(FOCUSABLE_SELECTOR.includes(fragment), fragment);
  }
  // tabindex last so an author can always override.
  assert.ok(FOCUSABLE_SELECTOR.endsWith("[tabindex]"));
});

test("positive tabindex comes first, ascending, then document order", () => {
  const items = [
    { id: "a", tabIndex: 0 },
    { id: "b", tabIndex: 5 },
    { id: "c", tabIndex: 0 },
    { id: "d", tabIndex: 2 },
    { id: "e", tabIndex: 0 },
  ];
  assert.deepEqual(
    orderByTabIndex(items).map((i) => i.id),
    ["d", "b", "a", "c", "e"],
  );
});

test("orderByTabIndex does not mutate its input", () => {
  const items = [{ id: "a", tabIndex: 9 }, { id: "b", tabIndex: 0 }];
  const copy = [...items];
  orderByTabIndex(items);
  assert.deepEqual(items, copy);
});

test("equal positive tabindex values keep document order", () => {
  const items = [
    { id: "a", tabIndex: 1 },
    { id: "b", tabIndex: 1 },
  ];
  assert.deepEqual(orderByTabIndex(items).map((i) => i.id), ["a", "b"]);
});

// ── the trap's wrap arithmetic ─────────────────────────────────────────────

test("Tab moves forward and wraps at the end", () => {
  assert.equal(nextFocusIndex(3, 0, false), 1);
  assert.equal(nextFocusIndex(3, 1, false), 2);
  assert.equal(nextFocusIndex(3, 2, false), 0, "last element wraps to first");
});

test("Shift+Tab moves back and wraps at the start", () => {
  assert.equal(nextFocusIndex(3, 2, true), 1);
  assert.equal(nextFocusIndex(3, 1, true), 0);
  assert.equal(nextFocusIndex(3, 0, true), 2, "first element wraps to last");
});

test("an empty container has no target", () => {
  assert.equal(nextFocusIndex(0, -1, false), null);
  assert.equal(nextFocusIndex(0, 0, true), null);
  assert.equal(nextFocusIndex(0, 5, false), null);
});

test("a single focusable element traps on itself", () => {
  assert.equal(nextFocusIndex(1, 0, false), 0);
  assert.equal(nextFocusIndex(1, 0, true), 0);
});

test("focus that escaped the container is pulled back in", () => {
  // The empty-drawer defect: focus was outside, so nothing trapped it.
  assert.equal(nextFocusIndex(3, -1, false), 0);
  assert.equal(nextFocusIndex(3, -1, true), 2);
});

test("a stale index beyond the list is treated as escaped", () => {
  // Happens when a re-render removes the focused node mid-interaction.
  assert.equal(nextFocusIndex(2, 7, false), 0);
  assert.equal(nextFocusIndex(2, 7, true), 1);
});

// ── initial focus ──────────────────────────────────────────────────────────

test("initial focus prefers the first tabbable element", () => {
  const items = [{ id: "a" }, { id: "b" }];
  assert.equal(resolveInitialFocus(items, undefined, []), items[0]);
});

test("[data-autofocus] wins over document order", () => {
  const items = [{ id: "a" }, { id: "b" }];
  assert.equal(resolveInitialFocus(items, "custom", [items[1]]), items[1]);
});

test("initial focus falls back to the container", () => {
  assert.equal(resolveInitialFocus([], undefined, []), "container");
  assert.equal(resolveInitialFocus([], "first", []), "container");
  assert.equal(resolveInitialFocus([{ id: "a" }], "container", []), "container");
});

test("a custom resolver that returns null falls back to the container", () => {
  assert.equal(resolveInitialFocus([{ id: "a" }], () => null, []), "container");
});

test("a custom resolver is honoured", () => {
  const target = { id: "danger" };
  assert.equal(resolveInitialFocus([{ id: "a" }], () => target, []), target);
});

// ── focus restoration ──────────────────────────────────────────────────────

test("focus is restored to a trigger that is still usable", () => {
  assert.equal(shouldRestoreFocus({ isConnected: true, tabIndex: 0 }), true);
});

test("focus is not restored to a trigger that has gone away", () => {
  // Closing a dialog whose trigger row was deleted drops the user at the top
  // of the document — the exact bug this guard prevents.
  assert.equal(shouldRestoreFocus({ isConnected: false, tabIndex: 0 }), false);
  assert.equal(shouldRestoreFocus(null), false);
  assert.equal(shouldRestoreFocus(undefined), false);
});

test("focus is not restored to a disabled or hidden trigger", () => {
  assert.equal(shouldRestoreFocus({ isConnected: true, tabIndex: 0, disabled: true }), false);
  assert.equal(shouldRestoreFocus({ isConnected: true, tabIndex: 0, visible: false }), false);
});

test("a trigger that is only programmatically focusable is skipped", () => {
  assert.equal(shouldRestoreFocus({ isConnected: true, tabIndex: -1 }), false);
});

test("a removed opener falls back to main content, never the body", () => {
  const main = { id: "main-content", isConnected: true, tabIndex: -1 };
  const removed = { isConnected: false, tabIndex: 0 };
  assert.equal(pickFocusRestoreTarget(removed, main), main);
  assert.equal(pickFocusRestoreTarget(null, main), main);
});

test("a usable opener wins over the fallback", () => {
  const opener = { isConnected: true, tabIndex: 0 };
  const main = { isConnected: true, tabIndex: -1 };
  assert.equal(pickFocusRestoreTarget(opener, main), opener);
});

test("a disconnected fallback cannot receive focus", () => {
  assert.equal(
    pickFocusRestoreTarget(null, { isConnected: false, tabIndex: -1 }),
    null,
  );
});

// ── the app must actually use it ───────────────────────────────────────────

const WEB_ROOT = path.join(__dirname, "..", "..");
const readWeb = (rel) => fs.readFileSync(path.join(WEB_ROOT, rel), "utf8");

test("all three overlays delegate to the shared hook", () => {
  for (const file of ["NavDrawer.tsx", "WalletModal.tsx", "vault/VaultDetailModal.tsx"]) {
    const source = readWeb(path.join("components", file));
    assert.match(source, /useFocusTrap/, file);
  }
  const source = readWeb(path.join("components", "NavDrawer.tsx"));
  // The old inline implementation must be gone, or the fix is cosmetic.
  assert.doesNotMatch(source, /function getFocusable/, "inline trap still present");
  assert.doesNotMatch(source, /onKeyDown=\{handleKeyDown\}/, "inline keydown still wired");
});

test("the hook restores to main content when the opener was removed", () => {
  const hook = readWeb(path.join("hooks", "useFocusTrap.ts"));
  assert.match(hook, /pickFocusRestoreTarget/);
  assert.match(hook, /getElementById\("main-content"\)/);
});

test("the hook listens on document so Escape survives focus escaping", () => {
  const hook = readWeb(path.join("hooks", "useFocusTrap.ts"));
  assert.match(hook, /document\.addEventListener\("keydown"/);
  assert.match(hook, /removeEventListener\("keydown"/, "listener is never removed");
});

test("the hook filters hidden elements instead of trusting querySelectorAll", () => {
  const hook = readWeb(path.join("hooks", "useFocusTrap.ts"));
  // Prettier wraps this call across lines, so match loosely rather than
  // pinning the formatter's line breaks.
  assert.match(hook, /filter\(\s*isFocusableElement/);
  // A fixed-position dialog has a null offsetParent; getClientRects is the fix.
  assert.match(hook, /getClientRects/);
});

test("the error summary takes focus rather than an individual field", () => {
  const summary = readWeb(path.join("components", "ErrorSummary.tsx"));
  assert.match(summary, /tabIndex=\{-1\}/);
  assert.match(summary, /ref\.current\?\.focus\(\)/);
  // Links must move focus, not just scroll to a hash.
  assert.match(summary, /target\.focus\(\)/);
});