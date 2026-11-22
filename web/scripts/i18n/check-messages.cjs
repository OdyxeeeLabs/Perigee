"use strict";

/**
 * Message catalogue completeness check — WEB-31 (#832).
 *
 * `messages/en.json` is the canonical key registry. A non-English catalogue
 * is allowed to be partial: next-intl falls back to English for a key that has
 * not been translated yet. What is not allowed is
 *
 *   - dropping a key every locale is expected to translate (`REQUIRED_KEYS`),
 *   - defining a key that does not exist in English (a typo that would never
 *     be reached by the UI).
 *
 * Translated *wording* is never compared across locales, and this check never
 * writes to a catalogue.
 *
 * Runs with: node scripts/i18n/check-messages.cjs [messages-directory]
 */

const fs = require("node:fs");
const path = require("node:path");

const WEB_ROOT = path.join(__dirname, "..", "..");
const DEFAULT_MESSAGES_DIR = path.join(WEB_ROOT, "messages");

const REQUIRED_KEYS = [
  "app.tagline",
  "nav.title",
  "palette.title",
  "a11y.languageSwitcher",
  "a11y.highContrast",
  "a11y.reduceMotion",
];

function readCatalogues(dir = DEFAULT_MESSAGES_DIR) {
  const catalogues = new Map();
  for (const file of fs.readdirSync(dir).filter((name) => name.endsWith(".json")).sort()) {
    const locale = path.basename(file, ".json");
    catalogues.set(locale, JSON.parse(fs.readFileSync(path.join(dir, file), "utf8")));
  }
  return catalogues;
}

function checkCatalogues(catalogues) {
  const problems = [];
  const summary = [];

  if (!catalogues.has("en")) {
    problems.push({ type: "invalid", locale: "en", key: "", message: "en.json is missing" });
    return { ok: false, problems, summary };
  }

  const english = catalogues.get("en");
  if (!english || typeof english !== "object" || Array.isArray(english)) {
    problems.push({ type: "invalid", locale: "en", key: "", message: "en.json is not an object" });
    return { ok: false, problems, summary };
  }
  const englishKeys = new Set(Object.keys(english));

  for (const [locale, catalogue] of catalogues) {
    if (!catalogue || typeof catalogue !== "object" || Array.isArray(catalogue)) {
      problems.push({
        type: "invalid",
        locale,
        key: "",
        message: `${locale}.json is not an object`,
      });
      continue;
    }

    let translated = 0;
    for (const [key, value] of Object.entries(catalogue)) {
      if (typeof value !== "string") {
        problems.push({
          type: "invalid",
          locale,
          key,
          message: `${locale}.json:${key} is not a string`,
        });
      }
      if (!englishKeys.has(key)) {
        problems.push({
          type: "unexpected",
          locale,
          key,
          message: `${locale}.json defines "${key}", which is absent from en.json`,
        });
      }
      translated += 1;
    }

    for (const key of REQUIRED_KEYS) {
      if (!(key in catalogue)) {
        problems.push({
          type: "missing",
          locale,
          key,
          message: `${locale}.json is missing the required key "${key}"`,
        });
      }
    }

    const untranslated = [...englishKeys].filter((key) => !(key in catalogue)).length;
    summary.push({ locale, translated, total: englishKeys.size, untranslated });
  }

  return { ok: problems.length === 0, problems, summary };
}

if (require.main === module) {
  const dir = process.argv[2] ? path.resolve(process.argv[2]) : DEFAULT_MESSAGES_DIR;
  const result = checkCatalogues(readCatalogues(dir));

  for (const row of result.summary) {
    console.log(`${row.locale}: ${row.translated}/${row.total} keys translated`);
  }

  if (!result.ok) {
    for (const problem of result.problems) {
      console.error(`::error::${problem.message}`);
    }
    console.error(
      `Message catalogues are incomplete: ${result.problems.length} problem(s).`,
    );
    process.exit(1);
  }

  console.log(`Message catalogues: ${result.summary.length} locale(s) complete.`);
}

module.exports = {
  REQUIRED_KEYS,
  DEFAULT_MESSAGES_DIR,
  readCatalogues,
  checkCatalogues,
};
