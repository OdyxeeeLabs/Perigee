// messages.test.cjs — WEB-31 (#832): deterministic message-key completeness.
//
// Runs with: node --test ./lib/locale/messages.test.cjs

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const {
  REQUIRED_KEYS,
  DEFAULT_MESSAGES_DIR,
  readCatalogues,
  checkCatalogues,
} = require("../../scripts/i18n/check-messages.cjs");

const CHECK = path.join(__dirname, "..", "..", "scripts", "i18n", "check-messages.cjs");

function fixture(catalogues) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "i18n-messages-"));
  for (const [locale, messages] of Object.entries(catalogues)) {
    fs.writeFileSync(path.join(dir, `${locale}.json`), JSON.stringify(messages));
  }
  return dir;
}

const base = () => Object.fromEntries(REQUIRED_KEYS.map((key) => [key, key]));

test("the shipped catalogues pass the check", () => {
  const result = checkCatalogues(readCatalogues(DEFAULT_MESSAGES_DIR));
  assert.deepEqual(result.problems, []);
  assert.equal(result.ok, true);
  assert.equal(result.summary.length, 8);
  for (const row of result.summary) {
    assert.ok(row.translated >= REQUIRED_KEYS.length, row.locale);
  }
});

test("a missing required key is reported for the locale that dropped it", () => {
  const es = base();
  delete es["a11y.highContrast"];
  const result = checkCatalogues(
    new Map([
      ["en", base()],
      ["es", es],
    ]),
  );

  assert.equal(result.ok, false);
  const missing = result.problems.filter((p) => p.type === "missing");
  assert.equal(missing.length, 1);
  assert.equal(missing[0].locale, "es");
  assert.equal(missing[0].key, "a11y.highContrast");
});

test("a key absent from en.json is reported as unexpected", () => {
  const result = checkCatalogues(
    new Map([
      ["en", base()],
      ["es", { ...base(), "palette.typoKey": "Texto" }],
    ]),
  );

  const unexpected = result.problems.filter((p) => p.type === "unexpected");
  assert.equal(unexpected.length, 1);
  assert.equal(unexpected[0].key, "palette.typoKey");
});

test("different translated wording for the same key is accepted", () => {
  const en = base();
  const es = { ...base(), "nav.title": "Navegación" };
  assert.notEqual(en["nav.title"], es["nav.title"]);
  const result = checkCatalogues(
    new Map([
      ["en", en],
      ["es", es],
    ]),
  );
  assert.deepEqual(result.problems, []);
});

test("a partial catalogue is allowed to fall back to English", () => {
  const result = checkCatalogues(
    new Map([
      ["en", { ...base(), "extra.onlyInEnglish": "English" }],
      ["es", base()],
    ]),
  );
  assert.deepEqual(result.problems, []);
  assert.equal(result.summary.find((row) => row.locale === "es").untranslated, 1);
});

test("the checker never rewrites a catalogue", () => {
  const dir = fixture({ en: base(), es: { ...base(), bogus: "x" } });
  try {
    const before = fs.readFileSync(path.join(dir, "es.json"), "utf8");
    checkCatalogues(readCatalogues(dir));
    assert.equal(fs.readFileSync(path.join(dir, "es.json"), "utf8"), before);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the CLI exits non-zero on a deliberately missing key", () => {
  const es = base();
  delete es["palette.title"];
  const dir = fixture({ en: base(), es });
  try {
    assert.throws(
      () => execFileSync(process.execPath, [CHECK, dir], { encoding: "utf8", stdio: "pipe" }),
      (error) => error.status === 1 && /missing the required key/.test(error.stderr.toString()),
    );
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a catalogue that is not a JSON object is rejected", () => {
  const dir = fixture({ en: base(), es: [] });
  try {
    const result = checkCatalogues(readCatalogues(dir));
    assert.equal(result.ok, false);
    assert.equal(result.problems[0].type, "invalid");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a non-string message value is rejected", () => {
  const result = checkCatalogues(
    new Map([
      ["en", base()],
      ["fr", { ...base(), "nav.title": 42 }],
    ]),
  );
  assert.equal(result.problems.some((p) => p.type === "invalid" && p.key === "nav.title"), true);
});
