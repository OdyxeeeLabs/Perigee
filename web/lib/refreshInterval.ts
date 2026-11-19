/**
 * User-controlled cadence for read-only data (issue #843 / WEB-42).
 *
 * Transaction confirmations, wallet/security notices and explicit Retry
 * buttons are never scheduled through this module — only background reads of
 * dashboards and status banners are. The default is one minute.
 */

export const DEFAULT_REFRESH_INTERVAL_MS = 60_000;

export const REFRESH_INTERVAL_OPTIONS = [
  { value: 15_000, label: "Every 15 seconds" },
  { value: 60_000, label: "Every minute (default)" },
  { value: 300_000, label: "Every 5 minutes" },
] as const;

export type RefreshIntervalMs = (typeof REFRESH_INTERVAL_OPTIONS)[number]["value"];

export function isRefreshInterval(value: unknown): value is RefreshIntervalMs {
  return REFRESH_INTERVAL_OPTIONS.some((option) => option.value === value);
}

/** Untrusted stored values fall back to the documented default. */
export function normalizeRefreshInterval(value: unknown): RefreshIntervalMs {
  return isRefreshInterval(value) ? value : DEFAULT_REFRESH_INTERVAL_MS;
}

/** True when at least one full interval has elapsed since the last read. */
export function shouldRefresh(now: number, lastRefreshAt: number | null, intervalMs: RefreshIntervalMs): boolean {
  return lastRefreshAt === null || now - lastRefreshAt >= intervalMs;
}
