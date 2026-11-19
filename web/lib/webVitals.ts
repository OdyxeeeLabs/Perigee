/**
 * Core Web Vitals instrumentation (issue #693).
 *
 * Next.js (Pages Router) calls the exported `reportWebVitals` from `_app.tsx`
 * for LCP, FID, CLS, FCP, TTFB, INP and its own custom metrics. Each metric is
 * enriched with page, device class and connection type and sent to the vitals
 * endpoint with `sendBeacon` — but only when the user consented to
 * performance telemetry.
 */

import { getTelemetryConsent, getTelemetryPreferences } from "./telemetry";

export type VitalName = "LCP" | "FID" | "INP" | "CLS" | "FCP" | "TTFB" | "TTI";

export interface VitalMetric {
  id: string;
  name: string;
  value: number;
}

export interface VitalPayload {
  name: string;
  value: number;
  rating: "good" | "needs-improvement" | "poor" | "unknown";
  page: string;
  device: "mobile" | "tablet" | "desktop";
  connection: string;
  ts: number;
}

/** [good upper bound, poor lower bound]; CLS is unitless, the rest are ms. */
export const VITAL_THRESHOLDS: Record<VitalName, [number, number]> = {
  LCP: [2500, 4000],
  FID: [100, 300],
  INP: [200, 500],
  CLS: [0.1, 0.25],
  FCP: [1800, 3000],
  TTFB: [800, 1800],
  TTI: [3800, 7300],
};

export const VITALS_ENDPOINT = process.env.NEXT_PUBLIC_VITALS_ENDPOINT || "/api/vitals";

export function rateVital(name: string, value: number): VitalPayload["rating"] {
  const t = VITAL_THRESHOLDS[name as VitalName];
  if (!t) return "unknown";
  if (value <= t[0]) return "good";
  if (value <= t[1]) return "needs-improvement";
  return "poor";
}

export function deviceClass(width: number): VitalPayload["device"] {
  if (width < 768) return "mobile";
  if (width < 1024) return "tablet";
  return "desktop";
}

function performanceAllowed(): boolean {
  if (getTelemetryConsent() !== true) return false;
  return getTelemetryPreferences().performance;
}

export function buildVitalPayload(metric: VitalMetric): VitalPayload {
  const connection =
    (navigator as Navigator & { connection?: { effectiveType?: string } }).connection
      ?.effectiveType ?? "unknown";
  return {
    name: metric.name,
    // CLS is unitless; everything else is rounded to whole milliseconds.
    value: metric.name === "CLS" ? Number(metric.value.toFixed(4)) : Math.round(metric.value),
    rating: rateVital(metric.name, metric.value),
    page: window.location.pathname,
    device: deviceClass(window.innerWidth),
    connection,
    ts: Date.now(),
  };
}

/** Signature expected by Next's `reportWebVitals` export. */
export function reportWebVitals(metric: VitalMetric): void {
  if (typeof window === "undefined" || !performanceAllowed()) return;

  const body = JSON.stringify(buildVitalPayload(metric));
  if (typeof navigator.sendBeacon === "function") {
    navigator.sendBeacon(VITALS_ENDPOINT, new Blob([body], { type: "application/json" }));
  } else {
    void fetch(VITALS_ENDPOINT, {
      method: "POST",
      body,
      headers: { "Content-Type": "application/json" },
      keepalive: true,
    }).catch(() => undefined);
  }
}
