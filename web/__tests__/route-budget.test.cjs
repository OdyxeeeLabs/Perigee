// Route JavaScript budget guard regression test (issue #842 / WEB-41).
// The CLI is exercised against a temporary Next build fixture so no local
// `yarn build`/CI command is needed.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const WEB_ROOT = path.join(__dirname, "..");
const SCRIPT = path.join(WEB_ROOT, "scripts", "check-route-budgets.mjs");
const read = (...parts) => fs.readFileSync(path.join(WEB_ROOT, ...parts), "utf8");

function makeFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "route-budget-"));
  const buildDir = path.join(root, ".next");
  fs.mkdirSync(path.join(buildDir, "static", "chunks"), { recursive: true });

  fs.writeFileSync(
    path.join(buildDir, "build-manifest.json"),
    JSON.stringify({
      pages: {
        "/": { files: ["static/chunks/home.js"] },
        "/analytics": { files: ["static/chunks/analytics-abc.js", "static/chunks/shared.js"] },
        "/vault/[id]": { files: ["static/chunks/vault.js"] },
      },
    }),
  );
  fs.writeFileSync(
    path.join(buildDir, "app-build-manifest.json"),
    JSON.stringify({ pages: { "/vault/[id]/page": { files: ["static/chunks/vault-app.js"] } } }),
  );
  fs.writeFileSync(path.join(buildDir, "static", "chunks", "home.js"), Buffer.alloc(100));
  fs.writeFileSync(path.join(buildDir, "static", "chunks", "analytics-abc.js"), Buffer.alloc(900));
  fs.writeFileSync(path.join(buildDir, "static", "chunks", "shared.js"), Buffer.alloc(50));
  fs.writeFileSync(path.join(buildDir, "static", "chunks", "vault.js"), Buffer.alloc(100));
  fs.writeFileSync(path.join(buildDir, "static", "chunks", "vault-app.js"), Buffer.alloc(120));

  const budgetFile = path.join(root, "route-budgets.json");
  const writeBudget = (analyticsMax) => {
    fs.writeFileSync(
      budgetFile,
      JSON.stringify({
        routes: [
          { name: "home", manifestKey: "/", maxBytes: 100_000 },
          { name: "vault", manifestKey: "/vault/[id]", appManifestKey: "/vault/[id]/page", maxBytes: 100_000 },
          { name: "analytics", manifestKey: "/analytics", maxBytes: analyticsMax },
        ],
      }),
    );
  };
  return { root, buildDir, budgetFile, writeBudget };
}

function run(buildDir, budgetFile) {
  return spawnSync(process.execPath, [SCRIPT, buildDir, "--budget-file", budgetFile], {
    cwd: WEB_ROOT,
    encoding: "utf8",
  });
}

test("a breached route budget fails and names the grown chunk", () => {
  const fixture = makeFixture();
  try {
    fixture.writeBudget(500);
    const result = run(fixture.buildDir, fixture.budgetFile);
    assert.equal(result.status, 1, `expected failure, got:\n${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /FAIL analytics \(\/analytics\)/);
    assert.match(result.stdout, /static\/chunks\/analytics-abc\.js: 900 bytes/);
    assert.match(result.stderr, /budget exceeded/);
  } finally {
    fs.rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("routes within budget pass, including App Router chunk mapping", () => {
  const fixture = makeFixture();
  try {
    fixture.writeBudget(950);
    const result = run(fixture.buildDir, fixture.budgetFile);
    assert.equal(result.status, 0, `expected success, got:\n${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /PASS analytics \(\/analytics\): 950 \/ 950 bytes/);
    assert.match(result.stdout, /PASS vault \(\/vault\/\[id\]\/page\): 120 \/ 100,000 bytes/);
    assert.match(result.stdout, /All route JavaScript bundles are within budget/);
  } finally {
    fs.rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("the budget is wired into package.json, docs and CI", () => {
  assert.match(read("package.json"), /"budget:routes": "node scripts\/check-route-budgets\.mjs \.next"/);
  assert.match(read("docs", "performance.md"), /Route JavaScript budgets \(#842/);
  assert.match(read("route-budgets.json"), /"name": "analytics"/);
  assert.match(read("route-budgets.json"), /"appManifestKey": "\/vault\/\[id\]\/page"/);

  const workflow = fs.readFileSync(
    path.join(WEB_ROOT, "..", ".github", "workflows", "web-route-budget.yml"),
    "utf8",
  );
  assert.match(workflow, /working-directory: web/);
  assert.match(workflow, /run: yarn build/);
  assert.match(workflow, /run: yarn budget:routes/);
});
