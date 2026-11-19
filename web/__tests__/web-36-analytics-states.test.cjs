// web/__tests__/web-36-analytics-states.test.cjs — source-contract + mirrored
// state-machine tests for issue #837 (WEB-36).
//
// The analytics page renders independent sections: one failed request must not
// hide the sections that did load. The reducer logic is DOM-free, so it is
// mirrored here (CJS cannot import the TS module) and the page is checked by
// source contract. Runs with: node --test ./__tests__/**/*.test.cjs

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, "..", relative), "utf8");

function initialSectionState() {
  return { status: "loading", data: null, error: null, attempt: 0 };
}

function sectionReducer(state, action) {
  switch (action.type) {
    case "load":
      return { status: "loading", data: state.data, error: null, attempt: state.attempt + 1 };
    case "succeed":
      return {
        status: action.empty ? "empty" : "ready",
        data: action.data,
        error: null,
        attempt: state.attempt,
      };
    case "fail":
      return { status: "error", data: state.data, error: action.message, attempt: state.attempt };
    default:
      return state;
  }
}

function isEmptyHistory(value) {
  if (value === null || value === undefined) return true;
  if (Array.isArray(value)) return value.length === 0;
  if (typeof value === "object") {
    const record = value;
    if ("sampleCount" in record) return Number(record.sampleCount) === 0;
    if ("recentEvents" in record) return Array.isArray(record.recentEvents) && record.recentEvents.length === 0;
  }
  return false;
}

const load = (state) => sectionReducer(state, { type: "load" });
const succeed = (state, data) =>
  sectionReducer(state, { type: "succeed", data, empty: isEmptyHistory(data) });
const fail = (state) => sectionReducer(state, { type: "fail", message: "unavailable" });

test("partial success: a failed section does not clear a loaded sibling", () => {
  const overview = succeed(load(initialSectionState()), { sampleCount: 3, recentEvents: [{}] });
  const features = fail(load(initialSectionState()));

  assert.equal(overview.status, "ready");
  assert.deepEqual(overview.data, { sampleCount: 3, recentEvents: [{}] });
  assert.equal(features.status, "error");
  assert.ok(features.error, "the failed section carries a user-facing reason");
});

test("total failure: every section is in error, none appear ready", () => {
  const sections = ["overview", "features", "errors", "events"].map((key) =>
    key === "overview"
      ? fail(load(initialSectionState()))
      : fail(load(initialSectionState())),
  );
  assert.ok(sections.every((s) => s.status === "error"));
  assert.ok(sections.every((s) => s.data === null));
});

test("empty history is distinct from a request failure", () => {
  const empty = succeed(load(initialSectionState()), []);
  const failed = fail(load(initialSectionState()));
  assert.equal(empty.status, "empty");
  assert.equal(failed.status, "error");
  assert.equal(empty.error, null);

  const emptySummary = succeed(load(initialSectionState()), { sampleCount: 0, recentEvents: [] });
  assert.equal(emptySummary.status, "empty");
  const populated = succeed(load(initialSectionState()), { sampleCount: 2, recentEvents: [{}, {}] });
  assert.equal(populated.status, "ready");
});

test("recovery: retry reloads a previously failed section into a ready state", () => {
  let section = fail(load(initialSectionState()));
  assert.equal(section.status, "error");

  section = succeed(load(section), [{ name: "contract_analyze", count: 4 }]);
  assert.equal(section.status, "ready");
  assert.equal(section.attempt, 2, "attempts are tracked across retries");
});

test("analytics dashboard wires four independent, individually-retryable sections", () => {
  const page = read("pages/analytics/index.tsx");
  assert.equal((page.match(/useAnalyticsSection</g) || []).length, 4);
  assert.match(page, /state=\{overview\.state\}/);
  assert.match(page, /state=\{features\.state\}/);
  assert.match(page, /state=\{errors\.state\}/);
  assert.match(page, /state=\{events\.state\}/);
  assert.match(page, /onRetry=\{overview\.retry\}/);
  assert.match(page, /onRetry=\{features\.retry\}/);
  assert.match(page, /onRetry=\{errors\.retry\}/);
  assert.match(page, /onRetry=\{events\.retry\}/);
  assert.match(page, /if \(!res\.ok\) \{\s*throw new Error/, "HTTP failures are treated as errors, not empty data");
});

test("unavailable sections are labelled and expose a retry action", () => {
  const component = read("components/analytics/AnalyticsSection.tsx");
  assert.match(component, /role="alert"/);
  assert.match(component, /data is unavailable\./, "unavailable data is labelled");
  assert.match(component, /Retry \{title\.toLowerCase\(\)\}/);
  assert.match(component, /state\.status === "empty"/, "empty history has its own branch");
  assert.match(component, /state\.status === "loading"/, "loading has its own branch");
});

test("the hook never surfaces raw exception text to the UI", () => {
  const hook = read("hooks/useAnalyticsSection.ts");
  assert.match(hook, /dispatch\(\{ type: "fail"/);
  assert.doesNotMatch(hook, /message: String\(error\)|error\.message/);
});

test("portfolio page handles an empty vault set without NaN metrics", () => {
  const portfolio = read("pages/portfolio.tsx");
  assert.match(portfolio, /breakdown\.length === 0 \|\| total <= 0/);
  assert.match(portfolio, /hasVaults \? Math\.max/);
  assert.match(portfolio, /No vaults to analyse yet\./);
});
