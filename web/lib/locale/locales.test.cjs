// locales.test.cjs — locale registry, persistence and Intl formatting
// (issue #700).
//
// Runs with: node --test ./lib/locale/locales.test.cjs
//
// Mirrors web/lib/locale/locales.ts and web/lib/locale/format.ts in plain
// CommonJS, matching the style of __tests__/config.test.cjs.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const SUPPORTED_LOCALES = ["en", "es", "zh", "ar", "hi", "pt", "fr", "de"];
const DEFAULT_LOCALE = "en";
const LOCALE_STORAGE_KEY = "perigee:locale";
const LOCALE_TAGS = {
  en: "en-GB", es: "es-ES", zh: "zh-Hans", ar: "ar",
  hi: "hi-IN", pt: "pt-BR", fr: "fr-FR", de: "de-DE",
};

const isSupportedLocale = (v) => typeof v === "string" && SUPPORTED_LOCALES.includes(v);

function normalizeLocale(value) {
  const raw = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (isSupportedLocale(raw)) return raw;
  const base = raw.split(/[-_]/)[0];
  return isSupportedLocale(base) ? base : DEFAULT_LOCALE;
}

const intlTag = (locale) => LOCALE_TAGS[locale] ?? locale;
const directionForLocale = (locale) => (locale === "ar" ? "rtl" : "ltr");

function readStoredLocale(storage) {
  try {
    return normalizeLocale(storage?.getItem(LOCALE_STORAGE_KEY));
  } catch {
    return DEFAULT_LOCALE;
  }
}

function storeLocale(locale, storage) {
  try {
    storage?.setItem(LOCALE_STORAGE_KEY, normalizeLocale(locale));
  } catch {
    /* ignore */
  }
}

const fakeStorage = () => {
  const map = new Map();
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, v),
  };
};

// ── the required locale set ────────────────────────────────────────────────

test("the eight requested locales are supported", () => {
  for (const locale of ["en", "es", "zh", "ar", "hi", "pt", "fr", "de"]) {
    assert.equal(isSupportedLocale(locale), true, locale);
  }
  assert.equal(SUPPORTED_LOCALES.length, 8);
});

test("direction is rtl only for Arabic", () => {
  assert.equal(directionForLocale("ar"), "rtl");
  for (const locale of ["en", "es", "zh", "hi", "pt", "fr", "de"]) {
    assert.equal(directionForLocale(locale), "ltr", locale);
  }
});

// ── normalization ──────────────────────────────────────────────────────────

test("normalizeLocale accepts the supported locales", () => {
  for (const locale of SUPPORTED_LOCALES) {
    assert.equal(normalizeLocale(locale), locale);
  }
});

test("normalizeLocale maps regional variants onto the shipped catalogue", () => {
  assert.equal(normalizeLocale("pt-PT"), "pt");
  assert.equal(normalizeLocale("pt_BR"), "pt");
  assert.equal(normalizeLocale("de-AT"), "de");
  assert.equal(normalizeLocale("zh-Hant"), "zh");
});

test("normalizeLocale is case- and whitespace-insensitive", () => {
  assert.equal(normalizeLocale("  ES  "), "es");
  assert.equal(normalizeLocale("FR"), "fr");
});

test("normalizeLocale falls back to English for anything unknown", () => {
  for (const bad of ["", "   ", null, undefined, 42, "xx", {}]) {
    assert.equal(normalizeLocale(bad), "en", JSON.stringify(bad));
  }
});

// ── persistence ────────────────────────────────────────────────────────────

test("a locale survives a round trip through storage", () => {
  const storage = fakeStorage();
  storeLocale("hi", storage);
  assert.equal(readStoredLocale(storage), "hi");
});

test("an unset preference reads as English", () => {
  assert.equal(readStoredLocale(fakeStorage()), "en");
});

test("a corrupted stored value is normalized, not trusted", () => {
  const storage = fakeStorage();
  storage.setItem(LOCALE_STORAGE_KEY, "klingon");
  assert.equal(readStoredLocale(storage), "en");
});

test("storage failures are swallowed", () => {
  const hostile = {
    getItem() { throw new Error("SecurityError"); },
    setItem() { throw new Error("QuotaExceeded"); },
  };
  assert.equal(readStoredLocale(hostile), "en");
  assert.doesNotThrow(() => storeLocale("fr", hostile));
  assert.doesNotThrow(() => storeLocale("fr", null));
});

test("intlTag returns the BCP 47 tag", () => {
  assert.equal(intlTag("zh"), "zh-Hans");
  assert.equal(intlTag("ar"), "ar");
});

// ── Intl formatting ────────────────────────────────────────────────────────
// Mirrored from web/lib/locale/format.ts: that file is TypeScript and this
// runner executes plain CommonJS, so the helpers are re-declared here with
// the same bodies and exercised against the real Intl data.

function formatNumber(value, locale = "en", options = {}) {
  const tag = intlTag(normalizeLocale(locale));
  if (!Number.isFinite(value)) return "—";
  try {
    return new Intl.NumberFormat(tag, options).format(value);
  } catch {
    return String(value);
  }
}

function formatCompactNumber(value, locale = "en") {
  return formatNumber(value, locale, { notation: "compact", maximumFractionDigits: 1 });
}

function formatPercent(value, locale = "en", digits = 1) {
  return formatNumber(value / 100, locale, {
    style: "percent",
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

function formatCurrency(value, currency, locale = "en", options = {}) {
  const tag = intlTag(normalizeLocale(locale));
  if (!Number.isFinite(value)) return "—";
  try {
    return new Intl.NumberFormat(tag, { style: "currency", currency, ...options }).format(value);
  } catch {
    return `${formatNumber(value, locale)} ${currency}`;
  }
}

function formatDate(value, locale = "en", options = { dateStyle: "medium" }) {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  try {
    return new Intl.DateTimeFormat(intlTag(normalizeLocale(locale)), options).format(date);
  } catch {
    return date.toISOString().slice(0, 10);
  }
}

function formatDateTime(value, locale = "en") {
  return formatDate(value, locale, { dateStyle: "medium", timeStyle: "short" });
}

function formatRelativeTime(value, locale = "en", now = new Date()) {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  const deltaSeconds = Math.round((date.getTime() - now.getTime()) / 1000);
  const absolute = Math.abs(deltaSeconds);
  try {
    const rtf = new Intl.RelativeTimeFormat(intlTag(normalizeLocale(locale)), { numeric: "auto" });
    if (absolute < 60) return rtf.format(Math.round(deltaSeconds), "second");
    if (absolute < 3600) return rtf.format(Math.round(deltaSeconds / 60), "minute");
    if (absolute < 86400) return rtf.format(Math.round(deltaSeconds / 3600), "hour");
    return rtf.format(Math.round(deltaSeconds / 86400), "day");
  } catch {
    return formatDate(date, locale);
  }
}

function pluralCategory(count, locale = "en") {
  const normalized = normalizeLocale(locale);
  try {
    return new Intl.PluralRules(intlTag(normalized)).select(count);
  } catch {
    return normalized === "zh" ? "other" : count === 1 ? "one" : "other";
  }
}

function pluralize(count, forms, locale = "en") {
  if (typeof forms === "string") return forms.replace("{count}", String(count));
  const normalized = normalizeLocale(locale);
  const category = pluralCategory(count, normalized);
  return (forms[category] ?? forms.other ?? "{count}").replace(/\{count\}/g, String(count));
}

function formatAmount(value, locale = "en", options = {}) {
  return formatNumber(value, locale, { maximumFractionDigits: 2, ...options });
}

function formatAssetAmount(value, asset, locale = "en", options = {}) {
  const amount = formatAmount(value, locale, options);
  return asset ? `${amount} ${asset}` : amount;
}

function formatRawNumber(value, locale = "en") {
  return formatNumber(value, locale, { maximumFractionDigits: 20 });
}

function formatSignedPercent(value, locale = "en", digits = 2) {
  const formatted = formatPercent(value, locale, digits);
  return value > 0 ? `+${formatted}` : formatted;
}

function formatTime(value, locale = "en") {
  return formatDate(value, locale, { timeStyle: "short" });
}

const date = new Date("2026-03-04T10:20:00Z");

test("numbers use the locale grouping and decimal marks", () => {
  const en = formatNumber(1234567.89, "en");
  const de = formatNumber(1234567.89, "de");
  const fr = formatNumber(1234567.89, "fr");

  assert.ok(en.includes("1,234,567.89"), `en: ${en}`);
  assert.ok(de.includes("1.234.567,89"), `de: ${de}`);
  assert.ok(/ | | /.test(fr), `fr should use a narrow/regular space: ${fr}`);
  assert.notEqual(en, de);
});

test("compact notation stays short in every locale", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const out = formatCompactNumber(1_234_567, locale);
    assert.ok(out.length < 12, `${locale} produced ${out}`);
  }
});

test("percentages honour the requested precision", () => {
  assert.equal(formatPercent(12.345, "en", 1), "12.3%");
  assert.equal(formatPercent(12.345, "en", 2), "12.35%");
  assert.equal(formatPercent(50, "en", 0), "50%");
});

test("amounts cap visual precision but keep exact values available", () => {
  const value = 1.234567890123456;
  const visual = formatAmount(value, "en");
  const raw = formatRawNumber(value, "en");
  assert.equal(visual, "1.23");
  assert.ok(raw.length > visual.length, `${raw} should retain more digits than ${visual}`);
  assert.ok(raw.startsWith("1.234567"), raw);
});

test("asset amounts place the asset code after the localized number", () => {
  const de = formatAssetAmount(1234.5, "XLM", "de");
  assert.match(de, /1\.234,5\s*XLM/);
  assert.equal(formatAssetAmount(1234.5, "", "en"), "1,234.5");
});

test("signed percentages mark gains explicitly", () => {
  assert.equal(formatSignedPercent(2.5, "en", 1), "+2.5%");
  assert.equal(formatSignedPercent(-2.5, "en", 1), "-2.5%");
  assert.equal(formatSignedPercent(0, "en", 1), "0.0%");
});

test("time-only formatting uses the locale clock", () => {
  const en = formatTime(date, "en");
  const de = formatTime(date, "de");
  assert.notEqual(en, de);
  assert.ok(en.length > 0);
});

test("currency formats with the requested code and locale", () => {
  const usd = formatCurrency(1234.5, "USD", "en");
  const eur = formatCurrency(1234.5, "EUR", "de");
  assert.ok(usd.includes("1,234.50") || usd.includes("1,234.5"), usd);
  assert.ok(/1\.234,50/.test(eur), eur);
});

test("an unknown currency code degrades instead of throwing", () => {
  const out = formatCurrency(10, "NOTACURRENCY", "en");
  assert.ok(out.includes("10"), out);
});

test("non-finite numbers render as an em dash", () => {
  for (const bad of [NaN, Infinity, -Infinity]) {
    assert.equal(formatNumber(bad, "en"), "—");
    assert.equal(formatCurrency(bad, "USD", "en"), "—");
  }
});

test("dates and date-times are locale-formatted", () => {
  const en = formatDate(date, "en");
  const de = formatDate(date, "de");
  assert.notEqual(en, de);
  assert.ok(formatDateTime(date, "en").length > 0);
});

test("an invalid date renders as an em dash, not 'Invalid Date'", () => {
  assert.equal(formatDate("not-a-date", "en"), "—");
  assert.equal(formatDate(NaN, "en"), "—");
});

test("relative time picks a sensible unit", () => {
  const now = new Date("2026-03-04T10:20:00Z");
  assert.ok(/second|minute|hour|day/.test(formatRelativeTime("2026-03-04T10:19:30Z", "en", now)));
  assert.ok(/minute/.test(formatRelativeTime("2026-03-04T10:15:00Z", "en", now)));
  assert.ok(/hour/.test(formatRelativeTime("2026-03-04T08:20:00Z", "en", now)));
  assert.ok(/day/.test(formatRelativeTime("2026-03-01T10:20:00Z", "en", now)));
  assert.equal(formatRelativeTime("nope", "en", now), "—");
});

// ── pluralisation ──────────────────────────────────────────────────────────

test("plural categories follow the CLDR rules of the locale", () => {
  assert.equal(pluralCategory(1, "en"), "one");
  assert.equal(pluralCategory(2, "en"), "other");
  assert.equal(pluralCategory(0, "en"), "other");
  assert.equal(pluralCategory(2, "zh"), "other");
  assert.equal(pluralCategory(0, "ar"), "zero");
  assert.equal(pluralCategory(2, "ar"), "two");
  assert.equal(pluralCategory(1, "fr"), "one");
  assert.equal(pluralCategory(0, "fr"), "one");
});

test("pluralize picks the form for the locale's category", () => {
  const forms = { one: "{count} vault", other: "{count} vaults" };
  assert.equal(pluralize(1, forms, "en"), "1 vault");
  assert.equal(pluralize(3, forms, "en"), "3 vaults");
  assert.equal(pluralize(0, forms, "en"), "0 vaults");
  assert.equal(pluralize(3, forms, "fr"), "3 vaults");
});

test("pluralize supports a flat template and missing keys", () => {
  assert.equal(pluralize(2, "{count} items", "en"), "2 items");
  assert.equal(pluralize(2, { one: "one only" }, "en"), "2");
});

// ── the app wiring that makes the registry real ─────────────────────────────
// next.config.js and lib/locale/locales.ts are two files that must agree.
// Node cannot execute next.config.js here (it requires next-intl/plugin, and
// node_modules is not installed), so the routing list is asserted as text.

const WEB_ROOT = path.join(__dirname, "..", "..");
const readWeb = (rel) => fs.readFileSync(path.join(WEB_ROOT, rel), "utf8");

test("next.config.js declares i18n routing for exactly our locales", () => {
  const config = readWeb("next.config.js");
  const block = /i18n:\s*\{([\s\S]*?)\}/.exec(config);
  assert.ok(block, "next.config.js has no i18n block; routing would be dead config");

  const listed = /locales:\s*\[([^\]]*)\]/.exec(block[1]);
  assert.ok(listed, "i18n block has no locales array");
  const parsed = listed[1]
    .split(",")
    .map((s) => s.trim().replace(/^["']|["']$/g, ""))
    .filter(Boolean);
  assert.deepEqual(parsed, SUPPORTED_LOCALES, "next.config.js locales drifted from SUPPORTED_LOCALES");

  assert.match(block[1], /defaultLocale:\s*"en"/);
  // Detection off keeps a shared Arabic URL from being rewritten to English.
  assert.match(block[1], /localeDetection:\s*false/);
});

test("a message catalogue exists for every locale", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const rel = path.join("messages", `${locale}.json`);
    const json = JSON.parse(readWeb(rel));
    assert.ok(Object.keys(json).length > 0, `${rel} is empty`);
  }
});

test("non-English catalogues keep the English key set as a subset", () => {
  const en = JSON.parse(readWeb(path.join("messages", "en.json")));
  for (const locale of SUPPORTED_LOCALES) {
    if (locale === "en") continue;
    const catalogue = JSON.parse(readWeb(path.join("messages", `${locale}.json`)));
    for (const key of Object.keys(catalogue)) {
      assert.ok(key in en, `${locale}.json defines "${key}", absent from en.json`);
      assert.equal(typeof catalogue[key], "string", `${locale}.json:${key} is not a string`);
    }
  }
});

test("the locale registry, switcher and sync component stay in sync", () => {
  const registry = readWeb(path.join("lib", "locale", "locales.ts"));
  for (const locale of SUPPORTED_LOCALES) {
    // LOCALE_NAMES / LOCALE_TAGS / PLURAL_CATEGORIES are all keyed by locale.
    const occurrences = (registry.match(new RegExp(`\\b${locale}:`, "g")) || []).length;
    assert.ok(occurrences >= 1, `${locale} missing from the registry`);
  }
  assert.ok(
    fs.existsSync(path.join(WEB_ROOT, "components", "LanguageSwitcher.tsx")),
    "LanguageSwitcher is missing",
  );
  assert.ok(
    fs.existsSync(path.join(WEB_ROOT, "components", "LocaleSync.tsx")),
    "LocaleSync is missing",
  );
});
