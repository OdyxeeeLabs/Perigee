"use strict";

/**
 * Source check for WEB-32 (#833): manager and onboarding routes must render
 * their visible copy through next-intl. Developer diagnostics (TODOs, code
 * identifiers, log messages) are not user-facing and are deliberately ignored.
 *
 * Runs with: node scripts/i18n/check-manager-copy.cjs
 */

const fs = require("node:fs");
const path = require("node:path");

const WEB_ROOT = path.join(__dirname, "..", "..");

const ROUTES = [
  "app/managers/page.tsx",
  "app/admin/managers/page.tsx",
  "pages/managers/onboarding.tsx",
  "pages/admin/managers.tsx",
];

/** JSX text children that are plain literals rather than expressions. */
function findLiteralCopy(source) {
  const withoutComments = source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/[^\n]*/g, "");

  const findings = [];
  for (const match of withoutComments.matchAll(/>([^<>{}]+)</g)) {
    const text = match[1].trim();
    if (/[A-Za-z]/.test(text)) findings.push(text);
  }
  return findings;
}

/** Every message key passed to t("...") in a source file. */
function findTranslationKeys(source) {
  const keys = new Set();
  for (const match of source.matchAll(/\bt\(\s*["']([^"']+)["']/g)) {
    keys.add(match[1]);
  }
  return [...keys];
}

function checkRoutes(routes = ROUTES, webRoot = WEB_ROOT) {
  const messages = JSON.parse(
    fs.readFileSync(path.join(webRoot, "messages", "en.json"), "utf8"),
  );

  const problems = [];
  for (const route of routes) {
    const source = fs.readFileSync(path.join(webRoot, route), "utf8");
    for (const text of findLiteralCopy(source)) {
      problems.push(`${route}: untranslated visible copy "${text}"`);
    }
    for (const key of findTranslationKeys(source)) {
      if (!(key in messages)) {
        problems.push(`${route}: message key "${key}" is missing from en.json`);
      }
    }
  }
  return problems;
}

if (require.main === module) {
  const problems = checkRoutes();
  if (problems.length > 0) {
    for (const problem of problems) console.error(`::error::${problem}`);
    process.exit(1);
  }
  console.log(`Manager copy: ${ROUTES.length} routes have no untranslated visible copy.`);
}

module.exports = { ROUTES, findLiteralCopy, findTranslationKeys, checkRoutes };
