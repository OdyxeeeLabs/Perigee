"use client";

import { useCallback, useEffect, useRef } from "react";
import { useHydrateSettings, useSettingsStore } from "@/lib/settingsStore";
import {
  normalizeRefreshInterval,
  type RefreshIntervalMs,
} from "@/lib/refreshInterval";

/** The persisted read-only refresh cadence, validated on every read. */
export function useRefreshInterval(): RefreshIntervalMs {
  useHydrateSettings();
  const stored = useSettingsStore((s) => s.settings.display.refreshIntervalMs);
  return normalizeRefreshInterval(stored);
}

export interface DataRefreshOptions {
  /** Pause background reads (e.g. while leaving a route) without cancelling mutations. */
  enabled?: boolean;
}

/**
 * Schedules a read-only refresh using the user's preference. The interval is
 * torn down on unmount, on route changes (the owning page unmounts) and when
 * the preference changes, so no timer fires into a stale route.
 */
export function useDataRefresh(refresh: () => void, { enabled = true }: DataRefreshOptions = {}) {
  const intervalMs = useRefreshInterval();
  const refreshRef = useRef(refresh);

  useEffect(() => {
    refreshRef.current = refresh;
  }, [refresh]);

  useEffect(() => {
    if (!enabled || typeof window === "undefined") return;
    const timer = window.setInterval(() => refreshRef.current(), intervalMs);
    return () => window.clearInterval(timer);
  }, [enabled, intervalMs]);

  return { intervalMs, refreshNow: useCallback(() => refreshRef.current(), []) };
}
