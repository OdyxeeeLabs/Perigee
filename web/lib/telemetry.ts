/**
 * Privacy-first telemetry helpers with consent gating, granular preferences,
 * PII scrubbing, and multi-provider support (Plausible, Umami, PostHog, Internal).
 *
 * Signals:
 *  - Global Privacy Control (GPC) and Do Not Track (DNT) detection
 *  - Local opt-out with stored preference revocation
 *  - 30-day consent TTL
 *  - Connect Wallet -> View Vault -> Deposit funnel tracking
 */

import {
  TelemetryEvent,
  validateTelemetryEvent,
  getEventCategory,
  FunnelEventName,
} from "./telemetrySchema";
import { sanitizeTelemetryProperties } from "./telemetryPii";
import {
  createAnalyticsDriver,
  InternalDriver,
  AnalyticsDriver,
} from "./telemetryProviders";

export type { TelemetryEvent } from "./telemetrySchema";

/** Granular consent categories. */
export interface TelemetryPreferences {
  /** Anonymous performance analytics (page-loads, timings, feature usage). */
  performance: boolean;
  /** Anonymous error reporting (stack traces with PII scrubbing). */
  errors: boolean;
  /** Anonymous product & funnel analytics (connect wallet, view vault, deposit). */
  analytics?: boolean;
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

export const CONSENT_KEY = "perigee_telemetry_consent";
export const CONSENT_PREFS_KEY = "perigee_telemetry_prefs";
export const CONSENT_TIMESTAMP_KEY = "perigee_telemetry_consent_at";

/** How long (ms) before re-surfacing the consent banner. Default: 30 days. */
export const CONSENT_TTL_MS = 30 * 24 * 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isBrowser(): boolean {
  return typeof window !== "undefined" && typeof window.localStorage !== "undefined";
}

/**
 * Checks whether the browser has signaled Do Not Track (DNT) or Global Privacy Control (GPC).
 */
export function isDoNotTrackEnabled(): boolean {
  if (!isBrowser()) return false;
  const nav = navigator as Navigator & { globalPrivacyControl?: boolean };
  if (nav.globalPrivacyControl === true) return true;
  if (nav.doNotTrack === "1" || (window as unknown as { doNotTrack?: string }).doNotTrack === "1") {
    return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// Drivers singleton instances
// ---------------------------------------------------------------------------

let primaryDriver: AnalyticsDriver | null = null;
let internalDriver: InternalDriver | null = null;

function getPrimaryDriver(): AnalyticsDriver {
  if (!primaryDriver) {
    primaryDriver = createAnalyticsDriver();
  }
  return primaryDriver;
}

function getInternalDriver(): InternalDriver {
  if (!internalDriver) {
    internalDriver = new InternalDriver();
  }
  return internalDriver;
}

// ---------------------------------------------------------------------------
// Consent Management (signals and stored choices)
// ---------------------------------------------------------------------------

/**
 * Returns the top-level consent value:
 *  - `true`  — user accepted (at least one category is on)
 *  - `false` — user explicitly declined everything or DNT/GPC is active without stored grant
 *  - `null`  — no decision recorded yet (banner should show)
 *
 * Returns `null` if the stored consent has expired (TTL exceeded).
 */
export function getTelemetryConsent(): boolean | null {
  if (!isBrowser()) return null;

  const stored = localStorage.getItem(CONSENT_KEY);

  // If no decision yet, check if Do Not Track or GPC is explicitly active
  if (stored === null) {
    if (isDoNotTrackEnabled()) {
      return false;
    }
    return null;
  }

  // Check TTL — if consent was recorded more than CONSENT_TTL_MS ago,
  // treat it as expired so the banner re-surfaces.
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

/**
 * Read the stored granular consent preferences.
 * Defaults to all-off when nothing is saved.
 */
export function getTelemetryPreferences(): TelemetryPreferences {
  if (!isBrowser()) return { performance: false, errors: false, analytics: false };
  try {
    const raw = localStorage.getItem(CONSENT_PREFS_KEY);
    if (!raw) return { performance: false, errors: false, analytics: false };
    const parsed = JSON.parse(raw) as TelemetryPreferences;
    return {
      performance: Boolean(parsed.performance),
      errors: Boolean(parsed.errors),
      analytics: parsed.analytics !== undefined ? Boolean(parsed.analytics) : Boolean(parsed.performance),
    };
  } catch {
    return { performance: false, errors: false, analytics: false };
  }
}

/**
 * Persist a top-level consent decision and record the timestamp.
 */
export function setTelemetryConsent(
  consent: boolean,
  prefs?: Partial<TelemetryPreferences>,
): void {
  if (!isBrowser()) return;

  const resolvedPrefs: TelemetryPreferences = prefs
    ? {
        performance: prefs.performance ?? consent,
        errors: prefs.errors ?? consent,
        analytics: prefs.analytics ?? consent,
      }
    : { performance: consent, errors: consent, analytics: consent };

  localStorage.setItem(CONSENT_KEY, consent ? "granted" : "denied");
  localStorage.setItem(CONSENT_PREFS_KEY, JSON.stringify(resolvedPrefs));
  localStorage.setItem(CONSENT_TIMESTAMP_KEY, String(Date.now()));

  if (consent) {
    initPrivacyTelemetry();
  } else {
    // If declined, destroy any active drivers
    if (primaryDriver) primaryDriver.destroy();
    if (internalDriver) internalDriver.destroy();
  }
}

/**
 * Clear all consent state so the banner will re-appear on the next page load.
 */
export function clearTelemetryConsent(): void {
  if (!isBrowser()) return;
  localStorage.removeItem(CONSENT_KEY);
  localStorage.removeItem(CONSENT_PREFS_KEY);
  localStorage.removeItem(CONSENT_TIMESTAMP_KEY);
  if (primaryDriver) primaryDriver.destroy();
  if (internalDriver) internalDriver.destroy();
}

/**
 * Opt-out helper: explicitly revokes all tracking consent and destroys scripts.
 */
export function optOutTelemetry(): void {
  setTelemetryConsent(false, { performance: false, errors: false, analytics: false });
}

/**
 * Opt-in helper: grants consent and activates drivers.
 */
export function optInTelemetry(prefs?: Partial<TelemetryPreferences>): void {
  setTelemetryConsent(true, prefs);
}

// ---------------------------------------------------------------------------
// Telemetry Initialization & Dispatch
// ---------------------------------------------------------------------------

export function initPrivacyTelemetry(): void {
  if (!isBrowser()) return;
  if (getTelemetryConsent() !== true) return;

  // Initialize configured primary driver (Plausible, Umami, PostHog)
  getPrimaryDriver().init();

  // Initialize internal batching collector driver for the team dashboard
  getInternalDriver().init();

  // Backward compatibility: Plausible legacy script fallback
  if (!document.getElementById("plausible-telemetry-script")) {
    const script = document.createElement("script");
    script.id = "plausible-telemetry-script";
    script.defer = true;
    script.dataset.domain = window.location.hostname;
    script.src = "https://plausible.io/js/script.js";
    document.head.appendChild(script);
  }
}

/**
 * Tracks a telemetry event with strict consent checking and PII scrubbing.
 */
export function trackTelemetryEvent(event: TelemetryEvent): void {
  if (!isBrowser() || getTelemetryConsent() !== true) return;

  const prefs = getTelemetryPreferences();
  const category = event.category || getEventCategory(event.name);

  // Check granular consent categories
  if (category === "performance" && !prefs.performance) return;
  if (category === "error" && !prefs.errors) return;
  if ((category === "funnel" || category === "pageview" || category === "feature") && prefs.analytics === false) {
    return;
  }

  // Schema validation & PII sanitization
  const validation = validateTelemetryEvent(event);
  if (!validation.valid || !validation.event) return;

  const validEvent = validation.event;

  // 1. Dispatch to Plausible legacy window function if present (maintains test compatibility)
  const win = window as unknown as { plausible?: (name: string, opts?: { props?: Record<string, unknown> }) => void };
  if (typeof win.plausible === "function") {
    win.plausible(validEvent.name, { props: validEvent.properties });
  }

  // 2. Dispatch to primary analytics driver (Plausible / Umami / PostHog)
  try {
    getPrimaryDriver().track(validEvent);
  } catch {
    // Swallow driver dispatch errors silently to avoid breaking UI
  }

  // 3. Dispatch to internal collector for the team dashboard
  try {
    getInternalDriver().track(validEvent);
  } catch {
    // Swallow driver dispatch errors silently
  }
}

// ---------------------------------------------------------------------------
// Specialized Tracking Helpers
// ---------------------------------------------------------------------------

/**
 * Tracks a page view.
 */
export function trackPageView(path: string, referrer?: string): void {
  trackTelemetryEvent({
    name: "page_view",
    category: "pageview",
    properties: {
      path,
      ...(referrer ? { referrer } : {}),
    },
  });
}

/**
 * Tracks funnel events (connect wallet -> view vault -> deposit).
 */
export function trackFunnelEvent(
  name: FunnelEventName,
  properties?: Record<string, unknown>,
): void {
  const cleanProps = sanitizeTelemetryProperties(properties);
  trackTelemetryEvent({
    name,
    category: "funnel",
    properties: cleanProps,
  });
}

/**
 * Tracks feature usage (contract analysis, command palette, filters, etc.).
 */
export function trackFeatureUsage(
  featureName: string,
  properties?: Record<string, unknown>,
): void {
  const cleanProps = sanitizeTelemetryProperties(properties);
  trackTelemetryEvent({
    name: featureName,
    category: "feature",
    properties: cleanProps,
  });
}

/**
 * Tracks an error with PII scrubbing (sanitized message, safe component name, no local file paths).
 */
export function trackTelemetryError(
  error: Error | string,
  context?: { component?: string; path?: string },
): void {
  const rawMessage = typeof error === "string" ? error : error.message || error.name || "Unknown error";
  // Strip user home directory file paths (e.g. C:\Users\... or /Users/...)
  const sanitizedMessage = rawMessage
    .replace(/(?:[a-zA-Z]:)?[\\/](?:Users|home)[\\/][^\\/:]+/gi, "[REDACTED_PATH]")
    .slice(0, 200);

  trackTelemetryEvent({
    name: "error_occurred",
    category: "error",
    properties: {
      message: sanitizedMessage,
      ...(context?.component ? { component: context.component } : {}),
      ...(context?.path ? { path: context.path } : {}),
    },
  });
}

/**
 * Tracks a performance metric / Web Vital.
 */
export function trackPerformanceMetric(vital: { name: string; value: number }): void {
  trackTelemetryEvent({
    name: "performance_vital",
    category: "performance",
    properties: {
      name: vital.name,
      value: Math.round(vital.value),
    },
  });
}
