// a11y-tooling.test.cjs — the audit suite's own integrity (issue #705).
//
// Runs with: node --test ./scripts/a11y/a11y-tooling.test.cjs
//
// The audit tooling has a failure mode that is invisible from the outside: an
// audit that silently matches nothing looks exactly like a clean audit. These
// tests pin the scanner to known-bad and known-good fixtures, and assert that
// the tooling the codebase imports is actually declared.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const WEB_ROOT = path.join(__dirname, "..", "..");
const readWeb = (rel) => fs.readFileSync(path.join(WEB_ROOT, rel), "utf8");

const AUDIT = path.join(WEB_ROOT, "scripts", "a11y", "static-audit.mjs");

/** Run the scanner over a throwaway fixture and return its findings. */
function auditFixture(files) {
  const dir = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "a11y-audit-"));
  for (const [name, content] of Object.entries(files)) {
    fs.writeFileSync(path.join(dir, name), content, "utf8");
  }
  try {
    const stdout = execFileSync(process.execPath, [AUDIT, dir, "--json"], {
      encoding: "utf8",
    });
    return JSON.parse(stdout).findings;
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const rulesHit = (findings) => new Set(findings.map((f) => f.rule));

// ── the scanner must actually catch things ─────────────────────────────────

test("an image without alt is reported", () => {
  const findings = auditFixture({ "Bad.tsx": `export const A = () => <img src="/a.png" />;` });
  assert.ok(rulesHit(findings).has("img-missing-alt"), JSON.stringify(findings));
});

test("an image with alt is not reported", () => {
  const findings = auditFixture({ "Good.tsx": `export const A = () => <img src="/a.png" alt="Chart" />;` });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

test("an iframe without a title is reported", () => {
  const findings = auditFixture({ "Bad.tsx": `<iframe src="https://x.test" />;` });
  assert.ok(rulesHit(findings).has("iframe-missing-title"));
});

test("a div with onClick and no keyboard path is reported", () => {
  const findings = auditFixture({
    "Bad.tsx": `<div onClick={() => {}}>click</div>;`,
  });
  assert.ok(rulesHit(findings).has("click-non-interactive"));
});

test("a div with a role and tabIndex is not reported", () => {
  const findings = auditFixture({
    "Good.tsx": `<div role="button" tabIndex={0} onClick={() => {}}>click</div>;`,
  });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

test("an aria-hidden backdrop is not reported", () => {
  // A pointer convenience on a decorative overlay: hidden from assistive
  // technology on purpose, with Escape and a close button as the keyboard
  // path. Reporting it would be noise that trains people to skip the check.
  const findings = auditFixture({
    "Backdrop.tsx": `<div aria-hidden="true" onClick={onClose} className="fixed inset-0" />;`,
  });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

test("positive tabindex is reported as a warning", () => {
  const findings = auditFixture({ "Bad.tsx": `<button tabIndex={3}>x</button>;` });
  assert.ok(rulesHit(findings).has("positive-tabindex"));
  assert.equal(findings.find((f) => f.rule === "positive-tabindex").severity, "warning");
});

test("focus:outline-none with a replacement ring is not reported", () => {
  const findings = auditFixture({
    "Good.tsx": `<button className="focus:outline-none focus:ring-2">x</button>;`,
  });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

test("focus:outline-none with nothing replacing it is reported", () => {
  const findings = auditFixture({
    "Bad.tsx": `<button className="focus:outline-none">x</button>;`,
  });
  assert.ok(rulesHit(findings).has("outline-none-without-replacement"));
});

test("aria-hidden on a focusable element is reported", () => {
  const findings = auditFixture({
    "Bad.tsx": `<div aria-hidden="true" tabIndex={0}>x</div>;`,
  });
  assert.ok(rulesHit(findings).has("aria-hidden-focusable"));
});

// ── the two rules that need surrounding markup ────────────────────────────

test("a label wrapping its control is correctly associated", () => {
  const findings = auditFixture({
    "Good.tsx": `<label className="flex"><input type="checkbox" checked={a} />Agree</label>;`,
  });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

test("a label whose control is a sibling is reported", () => {
  // The label names nothing: the input has no accessible name.
  const findings = auditFixture({
    "Bad.tsx": `<label>Email</label>\n<input value={v} />;`,
  });
  assert.ok(rulesHit(findings).has("label-without-associated-control"), JSON.stringify(findings));
});

test("a label with htmlFor is not reported", () => {
  const findings = auditFixture({
    "Good.tsx": `<label htmlFor="email">Email</label><input id="email" />;`,
  });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

test("an icon-only link with no accessible name is reported", () => {
  const findings = auditFixture({
    "Bad.tsx": `<a href="/x"><Icon /></a>;`,
  });
  assert.ok(rulesHit(findings).has("empty-link-name"), JSON.stringify(findings));
});

test("an icon-only link with aria-label is not reported", () => {
  const findings = auditFixture({
    "Good.tsx": `<a href="/x" aria-label="Settings"><Icon /></a>;`,
  });
  assert.equal(rulesHit(findings).size, 0, JSON.stringify(findings));
});

// ── the failure mode that makes an audit worthless ────────────────────────

test("markup inside comments is not audited", () => {
  // DataTable.tsx has an @accessibility JSDoc block describing a visually
  // hidden <label>; before comment stripping that was reported as a real
  // unassociated label in a file that has none.
  const findings = auditFixture({
    "Doc.tsx": [
      "/**",
      " * @accessibility",
      ' * - The input is labelled by a visually hidden <label>.',
      ' *   and another <img src="x">',
      " */",
      "export const A = () => <div>real markup</div>;",
    ].join("\n"),
  });
  assert.equal(findings.length, 0, JSON.stringify(findings));
});

test("line numbers survive comment stripping", () => {
  const findings = auditFixture({
    "Later.tsx": ["/** one", " * two", " * three", " */", '<img src="/x" />;'].join("\n"),
  });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].line, 5, JSON.stringify(findings));
});

test("multi-line opening tags are still matched", () => {
  // A per-line regex misses `<img` when the attributes wrap, which is most
  // of them in this codebase.
  const findings = auditFixture({
    "Wrapped.tsx": `<img\n  src="/a.png"\n  className="x"\n/>;`,
  });
  assert.ok(rulesHit(findings).has("img-missing-alt"), JSON.stringify(findings));
});

test("the scanner reports findings for this repository", () => {
  // Advisory mode must not silently pass because a rule stopped matching.
  const stdout = execFileSync(process.execPath, [AUDIT], { encoding: "utf8", cwd: WEB_ROOT });
  assert.match(stdout, /scanned: \d+ source files/);
  assert.ok(!/^No findings\.$/m.test(stdout), "expected the known baselined findings");
});

// ── declared tooling ───────────────────────────────────────────────────────

test("@axe-core/react is imported but not declared, and CI checks for that", () => {
  const pkg = JSON.parse(readWeb(path.join("package.json")));
  const declared = { ...(pkg.dependencies || {}), ...(pkg.devDependencies || {}) };
  const app = readWeb(path.join("pages", "_app.tsx"));

  assert.match(app, /import\("@axe-core\/react"\)/, "the import is expected to be there");
  assert.equal(
    declared["@axe-core/react"],
    undefined,
    "if this now passes, the dependency was added — update the docs and the CI job",
  );

  const workflow = readWeb(path.join("..", ".github", "workflows", "a11y-audit.yml"));
  assert.match(workflow, /@axe-core\/react/, "CI must assert the import/manifest agreement");
  assert.match(workflow, /declared-tooling/);
});

test("the silent catch in _app.tsx now reports the missing dependency", () => {
  const app = readWeb(path.join("pages", "_app.tsx"));
  assert.match(app, /axeWarningShown/, "the warning must be shown once per session");
  assert.match(app, /yarn add -D @axe-core\/react/, "the message must say how to fix it");
});

test("@playwright/test is not declared either", () => {
  const pkg = JSON.parse(readWeb(path.join("package.json")));
  const declared = { ...(pkg.dependencies || {}), ...(pkg.devDependencies || {}) };
  assert.equal(
    declared["@playwright/test"],
    undefined,
    "if this now passes, the dependency was added — update #707 and the docs",
  );
  // playwright.config.ts exists and cannot resolve a runner without it.
  assert.ok(fs.existsSync(path.join(WEB_ROOT, "playwright.config.ts")));
});

test("the audit workflow runs the accessibility suites with no install step", () => {
  const workflow = readWeb(path.join("..", ".github", "workflows", "a11y-audit.yml"));
  assert.match(workflow, /node --test/);
  // Assert on runnable steps, not on the raw text: the workflow *explains* in a
  // comment that no install is needed, so a bare substring check would match
  // its own justification.
  const runSteps = workflow
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("run:") || /^\s+(yarn|npm|pnpm)\s+install/.test(line));
  assert.equal(
    runSteps.filter((line) => /(yarn|npm|pnpm)\s+(install|add)\b/.test(line)).length,
    0,
    `the suites are dependency-free by design; found an install step: ${runSteps.join(" | ")}`,
  );

  // The workflow must glob the suites, not list them: a hardcoded list fails
  // on any branch that predates one of the suites (or that adds another).
  assert.match(workflow, /lib\/a11y\/\*\.test\.cjs/);
  assert.match(workflow, /No accessibility suites found/, "an empty glob must fail loudly");

  const suites = fs
    .readdirSync(path.join(WEB_ROOT, "lib", "a11y"))
    .filter((name) => name.endsWith(".test.cjs"));
  assert.ok(suites.length > 0, "expected accessibility suites in lib/a11y");
  assert.ok(suites.includes("live-region.test.cjs"));

  // Every suite the glob picks up must actually be runnable standalone.
  for (const suite of suites) {
    const stdout = execFileSync(process.execPath, ["--test", path.join("lib", "a11y", suite)], {
      cwd: WEB_ROOT,
      encoding: "utf8",
    });
    assert.doesNotMatch(stdout, /^ℹ fail [1-9]/m, `${suite} fails`);
  }
});