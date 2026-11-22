/**
 * Hydration timing (issue #694).
 *
 * Records the time from navigation start until the first client effect after
 * hydration as a `perigee:hydration` User Timing measure (visible in browser
 * performance tooling) and sends it as a consent-gated `hydration_time`
 * telemetry event. Safe to call more than once; only the first call measures.
 */

import { trackTelemetryEvent } from "./telemetry";

export const HYDRATION_MEASURE = "perigee:hydration";

let measured = false;

export function measureHydration(): number | null {
  if (measured || typeof performance === "undefined") return null;
  measured = true;

  const duration = performance.now();
  try {
    performance.measure(HYDRATION_MEASURE, { start: 0, end: duration });
  } catch {
    // User Timing options are unsupported in very old browsers; ignore.
  }

  trackTelemetryEvent({
    name: "hydration_time",
    properties: { duration_ms: Math.round(duration) },
  });
  return duration;
}
