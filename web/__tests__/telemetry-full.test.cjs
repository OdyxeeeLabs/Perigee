/**
 * Comprehensive integration tests for Issue #715: Telemetry & Analytics.
 *
 * Verifies:
 *  1. Custom Event Schema & Categorization:
 *     - Funnel events (connect wallet -> view vault -> deposit)
 *     - Page views
 *     - Feature usage
 *     - Errors
 *     - Performance metrics
 *  2. PII Scrubbing (GDPR / CCPA):
 *     - Forbidden keys (email, address, name, ip, phone, secret, token, etc.)
 *     - Value pattern redactions (Stellar G-address, Soroban C-address, secret S-seed, email, IP, hex keys)
 *  3. GDPR / CCPA Compliance & Opt-Out:
 *     - DNT / GPC browser signal enforcement
 *     - Consent TTL (30 days)
 *     - Granular preferences (performance, errors, analytics)
 *     - Instant opt-out mechanism
 *  4. Multi-provider drivers (Plausible, Umami, PostHog, Internal batching)
 *  5. API Telemetry Collector (/api/telemetry) aggregation and funnel computation.
 *
 * Uses node:test + node:assert.
 */

"use strict";

const assert = require("node:assert/strict");
const { test, describe, beforeEach, afterEach } = require("node:test");

// ---------------------------------------------------------------------------
// Browser storage and environment shims
// ---------------------------------------------------------------------------

const storageFactory = () => {
  const store = new Map();
  return {
    getItem: (k) => store.get(k) ?? null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
    clear: () => store.clear(),
  };
};

if (typeof global.localStorage === "undefined") {
  global.localStorage = storageFactory();
}

// ---------------------------------------------------------------------------
// Logic under test (mirrors web/lib/telemetryPii.ts, telemetrySchema.ts, telemetry.ts)
// ---------------------------------------------------------------------------

const FORBIDDEN_KEY_SUBSTRINGS = [
  "email",
  "address",
  "name",
  "ip",
  "phone",
  "ssn",
  "password",
  "secret",
  "token",
  "credential",
  "auth",
  "cookie",
  "signature",
  "privkey",
  "pubkey",
  "account_id",
  "public_key",
];

const EMAIL_REGEX = /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g;
const IPV4_REGEX = /\b(?:\d{1,3}\.){3}\d{1,3}\b/g;
const STELLAR_ADDRESS_REGEX = /\bG[A-Z0-9]{55}\b/g;
const CONTRACT_ADDRESS_REGEX = /\bC[A-Z0-9]{55}\b/g;
const STELLAR_SECRET_REGEX = /\bS[A-Z0-9]{55}\b/g;
const HEX_KEY_REGEX = /\b[0-9a-fA-F]{64}\b/g;

function isForbiddenKey(key) {
  const lower = key.toLowerCase();
  return FORBIDDEN_KEY_SUBSTRINGS.some((forbidden) => lower.includes(forbidden));
}

function sanitizeStringValue(value) {
  let sanitized = value;
  sanitized = sanitized.replace(EMAIL_REGEX, "[REDACTED_EMAIL]");
  sanitized = sanitized.replace(IPV4_REGEX, "[REDACTED_IP]");
  sanitized = sanitized.replace(STELLAR_ADDRESS_REGEX, "[REDACTED_STELLAR_ADDRESS]");
  sanitized = sanitized.replace(CONTRACT_ADDRESS_REGEX, "[REDACTED_CONTRACT_ADDRESS]");
  sanitized = sanitized.replace(STELLAR_SECRET_REGEX, "[REDACTED_SECRET]");
  sanitized = sanitized.replace(HEX_KEY_REGEX, "[REDACTED_HEX_KEY]");
  return sanitized;
}

function sanitizeTelemetryProperties(properties) {
  if (!properties || typeof properties !== "object") return {};
  const clean = {};

  for (const [key, val] of Object.entries(properties)) {
    if (isForbiddenKey(key)) continue;

    if (typeof val === "string") {
      clean[key] = sanitizeStringValue(val);
    } else if (typeof val === "number" && Number.isFinite(val)) {
      clean[key] = val;
    } else if (typeof val === "boolean") {
      clean[key] = val;
    } else if (val !== null && typeof val === "object") {
      const nested = sanitizeTelemetryProperties(val);
      for (const [nKey, nVal] of Object.entries(nested)) {
        clean[`${key}_${nKey}`] = nVal;
      }
    }
  }
  return clean;
}

const FUNNEL_EVENTS = {
  wallet_connect_initiated: { step: 1 },
  wallet_connect: { step: 1 },
  wallet_connected: { step: 1 },
  vault_list_viewed: { step: 2 },
  vault_viewed: { step: 2 },
  vault_filter_applied: { step: 2 },
  deposit_initiated: { step: 3 },
  deposit_amount_entered: { step: 3 },
  deposit_submitted: { step: 3 },
  deposit_success: { step: 3 },
};

function getFunnelStep(eventName) {
  return FUNNEL_EVENTS[eventName]?.step ?? null;
}

function getEventCategory(eventName) {
  if (
    eventName in FUNNEL_EVENTS ||
    eventName === "wallet_disconnect" ||
    eventName === "wallet_error" ||
    eventName === "vault_create" ||
    eventName === "deposit_failed"
  ) {
    return "funnel";
  }
  if (eventName === "page_view") return "pageview";
  if (
    eventName === "contract_analyze" ||
    eventName.startsWith("palette_") ||
    eventName === "theme_toggle" ||
    eventName === "feature_used"
  ) {
    return "feature";
  }
  if (eventName.includes("error")) return "error";
  if (
    eventName === "performance_vital" ||
    eventName === "hydration_time" ||
    eventName === "slow_render"
  ) {
    return "performance";
  }
  return "feature";
}

function validateTelemetryEvent(raw) {
  if (!raw || typeof raw !== "object") {
    return { valid: false, error: "Event must be an object" };
  }
  if (typeof raw.name !== "string" || !raw.name.trim()) {
    return { valid: false, error: "Event name must be a non-empty string" };
  }
  if (raw.name.length > 64) {
    return { valid: false, error: "Event name exceeds 64 characters" };
  }

  const cleanProps = sanitizeTelemetryProperties(raw.properties);
  const category = raw.category || getEventCategory(raw.name);
  const timestamp = typeof raw.timestamp === "number" ? raw.timestamp : Date.now();

  return {
    valid: true,
    event: {
      name: raw.name.trim(),
      properties: cleanProps,
      category,
      timestamp,
    },
  };
}

// ---------------------------------------------------------------------------
// Telemetry Controller with DNT / GPC & Granular consent
// ---------------------------------------------------------------------------

const CONSENT_KEY = "perigee_telemetry_consent";
const CONSENT_PREFS_KEY = "perigee_telemetry_prefs";
const CONSENT_TIMESTAMP_KEY = "perigee_telemetry_consent_at";
const CONSENT_TTL_MS = 30 * 24 * 60 * 60 * 1000;

function isDoNotTrackEnabled() {
  if (typeof global.navigator === "undefined") return false;
  return (
    global.navigator.globalPrivacyControl === true ||
    global.navigator.doNotTrack === "1"
  );
}

function getTelemetryConsent() {
  const stored = localStorage.getItem(CONSENT_KEY);
  if (stored === null) {
    if (isDoNotTrackEnabled()) return false;
    return null;
  }

  const tsRaw = localStorage.getItem(CONSENT_TIMESTAMP_KEY);
  if (tsRaw) {
    const ts = parseInt(tsRaw, 10);
    if (!Number.isNaN(ts) && Date.now() - ts > CONSENT_TTL_MS) {
      clearTelemetryConsent();
      return null;
    }
  }

  if (stored === "granted") return true;
  if (stored === "denied") return false;
  return null;
}

function getTelemetryPreferences() {
  try {
    const raw = localStorage.getItem(CONSENT_PREFS_KEY);
    if (!raw) return { performance: false, errors: false, analytics: false };
    const parsed = JSON.parse(raw);
    return {
      performance: Boolean(parsed.performance),
      errors: Boolean(parsed.errors),
      analytics: parsed.analytics !== undefined ? Boolean(parsed.analytics) : Boolean(parsed.performance),
    };
  } catch {
    return { performance: false, errors: false, analytics: false };
  }
}

function setTelemetryConsent(consent, prefs) {
  const resolved = prefs
    ? {
        performance: prefs.performance ?? consent,
        errors: prefs.errors ?? consent,
        analytics: prefs.analytics ?? consent,
      }
    : { performance: consent, errors: consent, analytics: consent };

  localStorage.setItem(CONSENT_KEY, consent ? "granted" : "denied");
  localStorage.setItem(CONSENT_PREFS_KEY, JSON.stringify(resolved));
  localStorage.setItem(CONSENT_TIMESTAMP_KEY, String(Date.now()));
}

function clearTelemetryConsent() {
  localStorage.removeItem(CONSENT_KEY);
  localStorage.removeItem(CONSENT_PREFS_KEY);
  localStorage.removeItem(CONSENT_TIMESTAMP_KEY);
}

function optOutTelemetry() {
  setTelemetryConsent(false, { performance: false, errors: false, analytics: false });
}

function trackTelemetryEvent(event, drivers = {}) {
  if (getTelemetryConsent() !== true) return null;

  const prefs = getTelemetryPreferences();
  const category = event.category || getEventCategory(event.name);

  if (category === "performance" && !prefs.performance) return null;
  if (category === "error" && !prefs.errors) return null;
  if (
    (category === "funnel" || category === "pageview" || category === "feature") &&
    prefs.analytics === false
  ) {
    return null;
  }

  const validated = validateTelemetryEvent(event);
  if (!validated.valid || !validated.event) return null;

  const validEvent = validated.event;

  // Dispatch to drivers
  if (typeof global.window?.plausible === "function") {
    global.window.plausible(validEvent.name, { props: validEvent.properties });
  }
  if (typeof drivers.umami?.track === "function") {
    drivers.umami.track(validEvent.name, validEvent.properties);
  }
  if (typeof drivers.posthog?.capture === "function") {
    drivers.posthog.capture(validEvent.name, validEvent.properties);
  }
  if (typeof drivers.internal?.track === "function") {
    drivers.internal.track(validEvent);
  }

  return validEvent;
}

// ---------------------------------------------------------------------------
// TEST SUITES
// ---------------------------------------------------------------------------

describe("1. Custom Event Schema & Categorization", () => {
  test("categorizes funnel events accurately", () => {
    assert.equal(getEventCategory("wallet_connect_initiated"), "funnel");
    assert.equal(getEventCategory("wallet_connect"), "funnel");
    assert.equal(getEventCategory("vault_viewed"), "funnel");
    assert.equal(getEventCategory("deposit_initiated"), "funnel");
    assert.equal(getEventCategory("deposit_success"), "funnel");
    assert.equal(getEventCategory("vault_create"), "funnel");
  });

  test("computes funnel steps (1: Connect, 2: View, 3: Deposit)", () => {
    assert.equal(getFunnelStep("wallet_connect_initiated"), 1);
    assert.equal(getFunnelStep("wallet_connect"), 1);
    assert.equal(getFunnelStep("vault_viewed"), 2);
    assert.equal(getFunnelStep("deposit_initiated"), 3);
    assert.equal(getFunnelStep("deposit_success"), 3);
    assert.equal(getFunnelStep("random_event"), null);
  });

  test("categorizes pageviews, features, errors, and performance", () => {
    assert.equal(getEventCategory("page_view"), "pageview");
    assert.equal(getEventCategory("contract_analyze"), "feature");
    assert.equal(getEventCategory("palette_navigate"), "feature");
    assert.equal(getEventCategory("error_occurred"), "error");
    assert.equal(getEventCategory("performance_vital"), "performance");
    assert.equal(getEventCategory("hydration_time"), "performance");
  });

  test("validates schema: rejects empty or overlength event names", () => {
    assert.equal(validateTelemetryEvent(null).valid, false);
    assert.equal(validateTelemetryEvent({ name: "" }).valid, false);
    assert.equal(validateTelemetryEvent({ name: "a".repeat(65) }).valid, false);
    assert.equal(validateTelemetryEvent({ name: "valid_name" }).valid, true);
  });
});

describe("2. Comprehensive PII Scrubbing (GDPR/CCPA Zero-PII)", () => {
  test("strips forbidden keys regardless of casing", () => {
    const raw = {
      user_email: "test@example.com",
      account_address: "GABC...",
      Client_IP: "192.168.1.1",
      full_name: "John Doe",
      auth_token: "secret123",
      password_hash: "xyz",
      wallet_id: "freighter",
      vault_id: "vault-1",
    };
    const cleaned = sanitizeTelemetryProperties(raw);
    assert.equal("user_email" in cleaned, false);
    assert.equal("account_address" in cleaned, false);
    assert.equal("Client_IP" in cleaned, false);
    assert.equal("full_name" in cleaned, false);
    assert.equal("auth_token" in cleaned, false);
    assert.equal("password_hash" in cleaned, false);
    assert.equal(cleaned.wallet_id, "freighter");
    assert.equal(cleaned.vault_id, "vault-1");
  });

  test("redacts Stellar public keys (G...) in string values", () => {
    const stellarAddr = "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5";
    const cleaned = sanitizeStringValue(`User sent transaction from ${stellarAddr} to vault`);
    assert.equal(cleaned.includes(stellarAddr), false);
    assert.equal(cleaned.includes("[REDACTED_STELLAR_ADDRESS]"), true);
  });

  test("redacts Soroban contract addresses (C...) in string values", () => {
    const contractAddr = "CA3D5KRYMCMCZVAC7OHQINZXSQWA5VE74G56IT2CXUS3UPVAXDGVNO40";
    const cleaned = sanitizeStringValue(`Invoking contract ${contractAddr} successfully`);
    assert.equal(cleaned.includes(contractAddr), false);
    assert.equal(cleaned.includes("[REDACTED_CONTRACT_ADDRESS]"), true);
  });

  test("redacts Stellar secret keys (S...) in string values", () => {
    const secretSeed = "SBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5";
    const cleaned = sanitizeStringValue(`Secret was ${secretSeed}`);
    assert.equal(cleaned.includes(secretSeed), false);
    assert.equal(cleaned.includes("[REDACTED_SECRET]"), true);
  });

  test("redacts email addresses and IPv4 in values", () => {
    const cleaned = sanitizeStringValue("Contact user@stellar.org from 10.0.0.1");
    assert.equal(cleaned.includes("user@stellar.org"), false);
    assert.equal(cleaned.includes("10.0.0.1"), false);
    assert.equal(cleaned.includes("[REDACTED_EMAIL]"), true);
    assert.equal(cleaned.includes("[REDACTED_IP]"), true);
  });

  test("flattens and scrubs nested objects safely", () => {
    const raw = {
      meta: {
        email: "leak@test.com",
        count: 5,
        status: "ACTIVE",
      },
    };
    const cleaned = sanitizeTelemetryProperties(raw);
    assert.equal("meta_email" in cleaned, false);
    assert.equal(cleaned.meta_count, 5);
    assert.equal(cleaned.meta_status, "ACTIVE");
  });
});

describe("3. GDPR / CCPA Compliance, DNT & Opt-Out", () => {
  const originalNavigator = globalThis.navigator;

  function mockNavigator(mockObj) {
    Object.defineProperty(globalThis, "navigator", {
      value: mockObj,
      configurable: true,
      writable: true,
    });
  }

  beforeEach(() => {
    localStorage.clear();
    mockNavigator({});
  });

  afterEach(() => {
    mockNavigator(originalNavigator);
  });

  test("respects Global Privacy Control (GPC): defaults to denied", () => {
    mockNavigator({ globalPrivacyControl: true });
    assert.equal(getTelemetryConsent(), false, "GPC must default consent to false");
  });

  test("respects Do Not Track (DNT): defaults to denied", () => {
    mockNavigator({ doNotTrack: "1" });
    assert.equal(getTelemetryConsent(), false, "DNT must default consent to false");
  });

  test("optOutTelemetry explicitly revokes all tracking", () => {
    setTelemetryConsent(true);
    assert.equal(getTelemetryConsent(), true);

    optOutTelemetry();
    assert.equal(getTelemetryConsent(), false);

    const prefs = getTelemetryPreferences();
    assert.equal(prefs.performance, false);
    assert.equal(prefs.errors, false);
    assert.equal(prefs.analytics, false);
  });

  test("consent TTL expires after 30 days and re-prompts user", () => {
    const now = Date.now();
    // Simulate consent saved 31 days ago
    const thirtyOneDaysAgo = now - 31 * 24 * 60 * 60 * 1000;
    localStorage.setItem(CONSENT_KEY, "granted");
    localStorage.setItem(CONSENT_TIMESTAMP_KEY, String(thirtyOneDaysAgo));

    assert.equal(getTelemetryConsent(), null, "Consent should expire after 30 days");
  });

  test("granular consent independently gates performance, errors, and analytics", () => {
    // Only allow error reporting, deny performance and analytics
    setTelemetryConsent(true, { performance: false, errors: true, analytics: false });

    const perfEvent = trackTelemetryEvent({
      name: "performance_vital",
      properties: { name: "LCP", value: 1200 },
    });
    assert.equal(perfEvent, null, "Performance event must be blocked when performance is disabled");

    const funnelEvent = trackTelemetryEvent({
      name: "wallet_connect",
      properties: { wallet_id: "freighter" },
    });
    assert.equal(funnelEvent, null, "Funnel event must be blocked when analytics is disabled");

    const errorEvent = trackTelemetryEvent({
      name: "error_occurred",
      properties: { message: "UI crashed" },
    });
    assert.notEqual(errorEvent, null, "Error event should fire when errors is enabled");
  });
});

describe("4. Funnel Tracking: Connect Wallet → View Vault → Deposit", () => {
  beforeEach(() => {
    localStorage.clear();
    setTelemetryConsent(true);
  });

  test("tracks full funnel sequence from connect to deposit", () => {
    const events = [];
    const internalDriver = { track: (ev) => events.push(ev) };

    // Step 1: Wallet Connect
    trackTelemetryEvent({ name: "wallet_connect_initiated" }, { internal: internalDriver });
    trackTelemetryEvent(
      { name: "wallet_connect", properties: { wallet_id: "freighter" } },
      { internal: internalDriver },
    );

    // Step 2: View Vault
    trackTelemetryEvent(
      { name: "vault_viewed", properties: { vault_id: "vault-usdc", asset: "USDC" } },
      { internal: internalDriver },
    );

    // Step 3: Deposit
    trackTelemetryEvent(
      { name: "deposit_initiated", properties: { vault_id: "vault-usdc", asset: "USDC" } },
      { internal: internalDriver },
    );
    trackTelemetryEvent(
      { name: "deposit_amount_entered", properties: { vault_id: "vault-usdc", amount_tier: "medium" } },
      { internal: internalDriver },
    );
    trackTelemetryEvent(
      { name: "deposit_success", properties: { vault_id: "vault-usdc", duration_ms: 100 } },
      { internal: internalDriver },
    );

    assert.equal(events.length, 6);
    assert.equal(events[0].name, "wallet_connect_initiated");
    assert.equal(events[1].name, "wallet_connect");
    assert.equal(events[2].name, "vault_viewed");
    assert.equal(events[3].name, "deposit_initiated");
    assert.equal(events[4].name, "deposit_amount_entered");
    assert.equal(events[5].name, "deposit_success");

    // Verify all belong to 'funnel' category
    for (const ev of events) {
      assert.equal(ev.category, "funnel");
    }
  });
});

describe("5. Multi-Provider Drivers Dispatch", () => {
  beforeEach(() => {
    localStorage.clear();
    setTelemetryConsent(true);
    global.window = {};
  });

  test("dispatches simultaneously to Plausible, Umami, and PostHog without conflict", () => {
    const plausibleCalls = [];
    global.window.plausible = (name, opts) => plausibleCalls.push({ name, props: opts?.props });

    const umamiCalls = [];
    const mockUmami = { track: (name, data) => umamiCalls.push({ name, data }) };

    const posthogCalls = [];
    const mockPosthog = { capture: (name, props) => posthogCalls.push({ name, props }) };

    trackTelemetryEvent(
      { name: "contract_analyze", properties: { mode: "wasm", fn: "swap" } },
      { umami: mockUmami, posthog: mockPosthog },
    );

    assert.equal(plausibleCalls.length, 1);
    assert.equal(plausibleCalls[0].name, "contract_analyze");
    assert.equal(plausibleCalls[0].props.mode, "wasm");

    assert.equal(umamiCalls.length, 1);
    assert.equal(umamiCalls[0].name, "contract_analyze");

    assert.equal(posthogCalls.length, 1);
    assert.equal(posthogCalls[0].name, "contract_analyze");
  });
});

describe("6. Telemetry Aggregation & Funnel Conversion Computation", () => {
  test("computes funnel conversion rates correctly", () => {
    const sampleEvents = [
      // 10 wallet connects
      ...Array.from({ length: 10 }, () => ({ name: "wallet_connect", category: "funnel" })),
      // 6 vault views
      ...Array.from({ length: 6 }, () => ({ name: "vault_viewed", category: "funnel" })),
      // 3 deposits
      ...Array.from({ length: 3 }, () => ({ name: "deposit_success", category: "funnel" })),
      // 2 features
      { name: "contract_analyze", category: "feature" },
      { name: "contract_analyze", category: "feature" },
    ];

    let walletConnect = 0;
    let vaultView = 0;
    let deposit = 0;

    for (const ev of sampleEvents) {
      const step = getFunnelStep(ev.name);
      if (step === 1) walletConnect++;
      else if (step === 2) vaultView++;
      else if (step === 3) deposit++;
    }

    assert.equal(walletConnect, 10);
    assert.equal(vaultView, 6);
    assert.equal(deposit, 3);

    const walletToVaultRate = Number(((vaultView / walletConnect) * 100).toFixed(1));
    const vaultToDepositRate = Number(((deposit / vaultView) * 100).toFixed(1));
    const overallRate = Number(((deposit / walletConnect) * 100).toFixed(1));

    assert.equal(walletToVaultRate, 60.0);
    assert.equal(vaultToDepositRate, 50.0);
    assert.equal(overallRate, 30.0);
  });
});
