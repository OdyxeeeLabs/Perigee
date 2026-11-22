"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const WEB_ROOT = path.join(__dirname, "..");
const MESSAGES_ROOT = path.join(WEB_ROOT, "messages");
const read = (relative) => fs.readFileSync(path.join(WEB_ROOT, relative), "utf8");

const CONSENT_KEY = "perigee_telemetry_consent";
const CONSENT_PREFS_KEY = "perigee_telemetry_prefs";
const CONSENT_TIMESTAMP_KEY = "perigee_telemetry_consent_at";
const PRIVACY_NOTICE_DISMISSED_KEY = "perigee_privacy_notice_dismissed";

const store = new Map();
const emitted = [];
const storage = {
  getItem: (key) => store.get(key) ?? null,
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: (key) => store.delete(key),
  clear: () => store.clear(),
};

function getConsent() {
  const stored = storage.getItem(CONSENT_KEY);
  return stored === "granted" ? true : stored === "denied" ? false : null;
}

function getPreferences() {
  try {
    const raw = storage.getItem(CONSENT_PREFS_KEY);
    if (!raw) return { performance: false, errors: false, analytics: false };
    const parsed = JSON.parse(raw);
    return {
      performance: Boolean(parsed.performance),
      errors: Boolean(parsed.errors),
      analytics: parsed.analytics === undefined ? Boolean(parsed.performance) : Boolean(parsed.analytics),
    };
  } catch {
    return { performance: false, errors: false, analytics: false };
  }
}

function setConsent(consent, prefs) {
  const resolved = prefs
    ? {
        performance: prefs.performance ?? consent,
        errors: prefs.errors ?? consent,
        analytics: prefs.analytics ?? consent,
      }
    : { performance: consent, errors: consent, analytics: consent };
  storage.setItem(CONSENT_KEY, consent ? "granted" : "denied");
  storage.setItem(CONSENT_PREFS_KEY, JSON.stringify(resolved));
  storage.setItem(CONSENT_TIMESTAMP_KEY, String(Date.now()));
}

function optOut() {
  setConsent(false, { performance: false, errors: false, analytics: false });
}

function track(category) {
  if (getConsent() !== true) return;
  const prefs = getPreferences();
  if (category === "performance" && !prefs.performance) return;
  if (category === "error" && !prefs.errors) return;
  if (["funnel", "pageview", "feature"].includes(category) && prefs.analytics === false) return;
  emitted.push(category);
}

function dismissNotice() {
  storage.setItem(PRIVACY_NOTICE_DISMISSED_KEY, "1");
}

test("WEB-22: opt-in persists granular preferences", () => {
  storage.clear();
  emitted.length = 0;
  setConsent(true, { performance: true, errors: false, analytics: true });

  assert.equal(getConsent(), true);
  assert.deepEqual(getPreferences(), { performance: true, errors: false, analytics: true });
  assert.ok(storage.getItem(CONSENT_TIMESTAMP_KEY));

  track("performance");
  track("error");
  track("funnel");
  assert.deepEqual(emitted, ["performance", "funnel"]);
});

test("WEB-22: opt-out takes effect for subsequent events", () => {
  storage.clear();
  emitted.length = 0;
  setConsent(true, { performance: true, errors: true, analytics: true });
  track("pageview");
  assert.deepEqual(emitted, ["pageview"]);

  optOut();
  track("pageview");
  track("error");
  assert.equal(getConsent(), false);
  assert.deepEqual(getPreferences(), { performance: false, errors: false, analytics: false });
  assert.deepEqual(emitted, ["pageview"]);
});

test("WEB-22: consent survives a reload", () => {
  storage.clear();
  setConsent(true, { performance: false, errors: true, analytics: false });

  assert.equal(getConsent(), true);
  assert.deepEqual(getPreferences(), { performance: false, errors: true, analytics: false });
});

test("WEB-22: changing preferences emits no separate tracking event", () => {
  const privacyTab = read(path.join("components", "settings", "PrivacySettingsTab.tsx"));
  assert.match(privacyTab, /setTelemetryConsent\(anyEnabled, next\)/);
  assert.match(privacyTab, /optOutTelemetry/);
  assert.doesNotMatch(privacyTab, /trackTelemetryEvent|trackEvent|trackFeatureUsage/);
});

test("WEB-24: dismissing the notice does not modify consent", () => {
  storage.clear();
  setConsent(true, { performance: true, errors: false, analytics: true });
  const consent = storage.getItem(CONSENT_KEY);
  const prefs = storage.getItem(CONSENT_PREFS_KEY);
  const timestamp = storage.getItem(CONSENT_TIMESTAMP_KEY);

  dismissNotice();

  assert.equal(storage.getItem(PRIVACY_NOTICE_DISMISSED_KEY), "1");
  assert.equal(storage.getItem(CONSENT_KEY), consent);
  assert.equal(storage.getItem(CONSENT_PREFS_KEY), prefs);
  assert.equal(storage.getItem(CONSENT_TIMESTAMP_KEY), timestamp);

  const source = read(path.join("lib", "privacyNotice.ts"));
  assert.match(source, /setItem\(PRIVACY_NOTICE_DISMISSED_KEY, "1"\)/);
  assert.doesNotMatch(source, /CONSENT_KEY|CONSENT_PREFS_KEY|CONSENT_TIMESTAMP_KEY/);
});

test("WEB-24: notice links use external-link rel hardening", () => {
  const source = read(path.join("components", "PrivacyNotice.tsx"));
  assert.match(source, /privacyNotice\.privacyPolicyUrl/);
  assert.match(source, /privacyNotice\.telemetryDocsUrl/);
  assert.equal((source.match(/target="_blank"/g) || []).length, 2);
  assert.equal((source.match(/rel="noopener noreferrer"/g) || []).length, 2);
});

test("WEB-24: all locales include notice, analytics, and settings copy", () => {
  const locales = ["en", "es", "de", "fr", "pt", "zh", "ar", "hi"];
  const keys = [
    "analytics.description",
    "analytics.preferencesBody",
    "analytics.settingsPrivacyLink",
    "privacyNotice.ariaLabel",
    "privacyNotice.title",
    "privacyNotice.intro",
    "privacyNotice.analytics",
    "privacyNotice.errors",
    "privacyNotice.wallet",
    "privacyNotice.privacyPolicy",
    "privacyNotice.details",
    "privacyNotice.legalNoteLabel",
    "privacyNotice.dismiss",
    "privacyNotice.manage",
    "settings.title",
    "settings.save",
    "settings.errorSummaryTitle",
    "settings.privacy.title",
    "settings.privacy.description",
  ];

  for (const locale of locales) {
    const messages = require(path.join(MESSAGES_ROOT, `${locale}.json`));
    for (const key of keys) {
      assert.equal(typeof messages[key], "string", `${locale} missing ${key}`);
      assert.ok(messages[key].length > 0, `${locale} has empty ${key}`);
    }
    assert.doesNotMatch(messages["analytics.description"], /compliant|compliance/i);
    assert.doesNotMatch(messages["analytics.preferencesBody"], /ever collected|never collected/i);
  }
});

test("WEB-24: telemetry copy avoids unsupported compliance guarantees", () => {
  const telemetry = read(path.join("lib", "telemetry.ts"));
  const pii = read(path.join("lib", "telemetryPii.ts"));

  assert.doesNotMatch(telemetry, /GDPR\s*&\s*CCPA compliant|Fully compliant/i);
  assert.doesNotMatch(pii, /Fully compliant|Guarantees zero/i);
});
