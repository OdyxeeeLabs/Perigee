// roving-tabindex.test.cjs — composite-widget keyboard maths (issue #698).
//
// Runs with: node --test ./lib/a11y/roving-tabindex.test.cjs
//
// Mirrors web/lib/a11y/roving-tabindex.ts in plain CommonJS.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

function handlesKey(key, orientation = "horizontal") {
  switch (key) {
    case "Home":
    case "End":
      return true;
    case "ArrowRight":
    case "ArrowLeft":
      return orientation === "horizontal" || orientation === "both";
    case "ArrowDown":
    case "ArrowUp":
      return orientation === "vertical" || orientation === "both";
    default:
      return false;
  }
}

function nextRovingIndex(key, current, count, orientation = "horizontal", options = {}) {
  const { wrap = true, rtl = false, isDisabled } = options;
  if (count <= 0) return current;
  if (!handlesKey(key, orientation)) return current;

  const enabled = (index) => (isDisabled ? !isDisabled(index) : true);
  const at = (index) => ((index % count) + count) % count;

  if (key === "Home") {
    for (let step = 0; step < count; step += 1) if (enabled(step)) return step;
    return current;
  }
  if (key === "End") {
    for (let step = count - 1; step >= 0; step -= 1) if (enabled(step)) return step;
    return current;
  }

  const movesForward = key === "ArrowRight" || key === "ArrowDown";
  const forward = rtl ? !movesForward : movesForward;

  let index = current;
  for (let step = 0; step < count; step += 1) {
    const candidate = index + (forward ? 1 : -1);
    if (!wrap && (candidate < 0 || candidate >= count)) return current;
    index = at(candidate);
    if (enabled(index)) return index;
  }
  return current;
}

function rovingTabIndexFor(index, activeIndex) {
  return index === activeIndex ? 0 : -1;
}

function shouldActivate(key, phase = "keydown") {
  if (key === "Enter") return true;
  if (key === " " || key === "Spacebar") return phase === "keydown";
  return false;
}

// ── which keys belong to the widget ────────────────────────────────────────

test("horizontal widgets own the inline arrows", () => {
  assert.equal(handlesKey("ArrowRight", "horizontal"), true);
  assert.equal(handlesKey("ArrowLeft", "horizontal"), true);
  assert.equal(handlesKey("ArrowDown", "horizontal"), false);
  assert.equal(handlesKey("ArrowUp", "horizontal"), false);
});

test("vertical widgets own the block arrows", () => {
  assert.equal(handlesKey("ArrowDown", "vertical"), true);
  assert.equal(handlesKey("ArrowUp", "vertical"), true);
  assert.equal(handlesKey("ArrowRight", "vertical"), false);
});

test("both orientations accept all four arrows", () => {
  for (const key of ["ArrowRight", "ArrowLeft", "ArrowDown", "ArrowUp"]) {
    assert.equal(handlesKey(key, "both"), true, key);
  }
});

test("Home, End and unrelated keys are classified correctly", () => {
  assert.equal(handlesKey("Home"), true);
  assert.equal(handlesKey("End"), true);
  for (const key of ["Tab", "a", "Escape", "Shift", "ArrowUp"]) {
    assert.equal(handlesKey(key), false, key);
  }
});

// ── movement ───────────────────────────────────────────────────────────────

test("arrow keys move to the neighbouring item", () => {
  assert.equal(nextRovingIndex("ArrowRight", 0, 4), 1);
  assert.equal(nextRovingIndex("ArrowLeft", 2, 4), 1);
  assert.equal(nextRovingIndex("ArrowDown", 0, 4, "vertical"), 1);
  assert.equal(nextRovingIndex("ArrowUp", 2, 4, "vertical"), 1);
});

test("movement wraps by default", () => {
  assert.equal(nextRovingIndex("ArrowRight", 3, 4), 0);
  assert.equal(nextRovingIndex("ArrowLeft", 0, 4), 3);
});

test("wrap can be turned off for grid patterns", () => {
  assert.equal(nextRovingIndex("ArrowRight", 3, 4, "horizontal", { wrap: false }), 3);
  assert.equal(nextRovingIndex("ArrowLeft", 0, 4, "horizontal", { wrap: false }), 0);
});

test("RTL swaps the inline arrows", () => {
  assert.equal(nextRovingIndex("ArrowRight", 1, 4, "horizontal", { rtl: true }), 0);
  assert.equal(nextRovingIndex("ArrowLeft", 1, 4, "horizontal", { rtl: true }), 2);
});

test("Home and End land on the first and last enabled item", () => {
  assert.equal(nextRovingIndex("Home", 2, 4), 0);
  assert.equal(nextRovingIndex("End", 0, 4), 3);
});

test("Home and End skip disabled items", () => {
  const isDisabled = (i) => i === 0 || i === 3;
  assert.equal(nextRovingIndex("Home", 2, 4, "horizontal", { isDisabled }), 1);
  assert.equal(nextRovingIndex("End", 0, 4, "horizontal", { isDisabled }), 2);
});

test("arrow movement skips disabled items", () => {
  const isDisabled = (i) => i === 1;
  assert.equal(nextRovingIndex("ArrowRight", 0, 3, "horizontal", { isDisabled }), 2);
});

test("an all-disabled group leaves focus where it is", () => {
  const all = () => true;
  assert.equal(nextRovingIndex("Home", 1, 3, "horizontal", { isDisabled: all }), 1);
  assert.equal(nextRovingIndex("ArrowRight", 1, 3, "horizontal", { isDisabled: all }), 1);
});

test("empty and single-item groups are safe", () => {
  assert.equal(nextRovingIndex("ArrowRight", 0, 0), 0);
  assert.equal(nextRovingIndex("ArrowRight", 0, 1), 0);
});

test("keys the widget does not own leave focus alone", () => {
  assert.equal(nextRovingIndex("Tab", 1, 4), 1);
});

// ── tabindex assignment ────────────────────────────────────────────────────

test("exactly one item is in the tab order", () => {
  assert.equal(rovingTabIndexFor(2, 2), 0);
  assert.equal(rovingTabIndexFor(0, 2), -1);
  assert.equal(rovingTabIndexFor(3, 2), -1);
});

// ── activation ─────────────────────────────────────────────────────────────

test("Enter activates on keydown; Space activates on keydown only", () => {
  assert.equal(shouldActivate("Enter"), true);
  assert.equal(shouldActivate(" "), true);
  assert.equal(shouldActivate(" ", "keyup"), false);
  assert.equal(shouldActivate("Spacebar"), true);
  assert.equal(shouldActivate("a"), false);
  assert.equal(shouldActivate("Tab"), false);
});
