// rtl.test.cjs — direction, locale table and RTL grid maths (issue #699).
//
// Runs with: node --test ./lib/i18n/rtl.test.cjs
//
// Mirrors web/lib/i18n/rtl.ts in plain CommonJS.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const RTL_LOCALES = ["ar", "he", "fa", "ur"];
const SUPPORTED_LOCALES = ["en", "es", "zh", "ar", "hi", "pt", "fr", "de"];

const LOCALE_TAGS = {
  en: "en-GB",
  es: "es-ES",
  zh: "zh-Hans",
  ar: "ar",
  hi: "hi-IN",
  pt: "pt-BR",
  fr: "fr-FR",
  de: "de-DE",
};

function directionForLocale(locale) {
  if (!locale) return "ltr";
  const base = String(locale).trim().toLowerCase().split(/[-_]/)[0];
  return RTL_LOCALES.includes(base) ? "rtl" : "ltr";
}

const isRtlLocale = (locale) => directionForLocale(locale) === "rtl";
const inlineStep = (locale) => (isRtlLocale(locale) ? -1 : 1);
const isSupportedLocale = (locale) => SUPPORTED_LOCALES.includes(locale);
const langAttributeFor = (locale) =>
  isSupportedLocale(locale) ? LOCALE_TAGS[locale] : locale;

function isolate(value, base = "ltr") {
  const marks = base === "rtl" ? ["\u2067", "\u2069"] : ["\u2066", "\u2069"];
  return `${marks[0]}${value}${marks[1]}`;
}

// ── direction ──────────────────────────────────────────────────────────────

test("Arabic and Hebrew resolve to rtl", () => {
  for (const locale of ["ar", "he", "fa", "ur"]) {
    assert.equal(directionForLocale(locale), "rtl", locale);
    assert.equal(isRtlLocale(locale), true, locale);
  }
});

test("every shipped locale that is not Arabic is ltr", () => {
  for (const locale of ["en", "es", "zh", "hi", "pt", "fr", "de"]) {
    assert.equal(directionForLocale(locale), "ltr", locale);
  }
});

test("region subtags are stripped, so ar-EG and he-IL are rtl", () => {
  assert.equal(directionForLocale("ar-EG"), "rtl");
  assert.equal(directionForLocale("he-IL"), "rtl");
  assert.equal(directionForLocale("en-GB"), "ltr");
  assert.equal(directionForLocale("pt_BR"), "ltr");
});

test("casing and whitespace do not matter", () => {
  assert.equal(directionForLocale("AR"), "rtl");
  assert.equal(directionForLocale("  He-IL "), "rtl");
});

test("missing or unknown locales fall back to ltr", () => {
  for (const bad of ["", "   ", null, undefined, "xx", "klingon"]) {
    assert.equal(directionForLocale(bad), "ltr", JSON.stringify(bad));
  }
});

// ── grid navigation ────────────────────────────────────────────────────────

test("the inline axis is mirrored in rtl", () => {
  assert.equal(inlineStep("en"), 1);
  assert.equal(inlineStep("ar"), -1);
  assert.equal(inlineStep("he-IL"), -1);
});

// ── locale table ───────────────────────────────────────────────────────────

test("the eight required locales are supported", () => {
  for (const locale of ["en", "es", "zh", "ar", "hi", "pt", "fr", "de"]) {
    assert.equal(isSupportedLocale(locale), true, locale);
  }
});

test("unknown locales are not supported", () => {
  assert.equal(isSupportedLocale("xx"), false);
  assert.equal(isSupportedLocale(""), false);
});

test("lang attributes map to the right BCP 47 tag", () => {
  assert.equal(langAttributeFor("en"), "en-GB");
  assert.equal(langAttributeFor("zh"), "zh-Hans");
  assert.equal(langAttributeFor("ar"), "ar");
});

test("an unsupported locale is passed through unchanged", () => {
  assert.equal(langAttributeFor("xx"), "xx");
});

// ── mixed content ──────────────────────────────────────────────────────────

test("isolate wraps a value in directional isolates", () => {
  assert.equal(isolate("1234", "ltr"), "\u20661234\u2069");
  assert.equal(isolate("1234", "rtl"), "\u20671234\u2069");
});

test("both isolates use the same pop mark so nesting is balanced", () => {
  for (const base of ["ltr", "rtl"]) {
    const out = isolate("x", base);
    assert.equal(out.endsWith("\u2069"), true, base);
    assert.equal(out.length, 3, base);
  }
});
