// manager-copy.test.cjs — WEB-32 (#833): the manager routes must not contain
// visible copy that bypasses next-intl.
//
// Runs with: node --test ./lib/i18n/manager-copy.test.cjs

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  ROUTES,
  findLiteralCopy,
  findTranslationKeys,
  checkRoutes,
} = require("../../scripts/i18n/check-manager-copy.cjs");

const WEB_ROOT = path.join(__dirname, "..", "..");

function fixture(source, messages = { "managers.title": "Managers" }) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "manager-copy-"));
  fs.mkdirSync(path.join(dir, "messages"), { recursive: true });
  fs.writeFileSync(path.join(dir, "messages", "en.json"), JSON.stringify(messages));
  fs.writeFileSync(path.join(dir, "route.tsx"), source, "utf8");
  return dir;
}

test("the scanner reports JSX text literals but not expressions or comments", () => {
  assert.deepEqual(findLiteralCopy("export const A = () => <h1>Managers</h1>;"), [
    "Managers",
  ]);
  assert.deepEqual(
    findLiteralCopy("export const A = () => <h1>{t('managers.title')}</h1>;"),
    [],
  );
  assert.deepEqual(findLiteralCopy("/* No managers found. */ <p>{value} — {other}</p>"), []);
});

test("the scanner collects both single- and double-quoted message keys", () => {
  const source = `const a = t("managers.title"); const b = t('admin.managers.title');`;
  assert.deepEqual(findTranslationKeys(source), [
    "managers.title",
    "admin.managers.title",
  ]);
});

test("every touched manager route localizes its visible copy", () => {
  assert.equal(ROUTES.length, 4);
  assert.deepEqual(checkRoutes(ROUTES, WEB_ROOT), []);
});

test("a deliberately hardcoded label is detected", () => {
  const dir = fixture("export const A = () => <h1>No managers found</h1>;");
  try {
    const problems = checkRoutes(["route.tsx"], dir);
    assert.equal(problems.length, 1);
    assert.match(problems[0], /untranslated visible copy "No managers found"/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a message key missing from en.json is detected", () => {
  const dir = fixture(`export const A = () => <h1>{t("managers.missing")}</h1>;`);
  try {
    const problems = checkRoutes(["route.tsx"], dir);
    assert.equal(problems.length, 1);
    assert.match(problems[0], /"managers.missing" is missing from en.json/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a fully localized route produces no findings", () => {
  const dir = fixture(`export const A = () => <h1>{t("managers.title")}</h1>;`);
  try {
    assert.deepEqual(checkRoutes(["route.tsx"], dir), []);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
