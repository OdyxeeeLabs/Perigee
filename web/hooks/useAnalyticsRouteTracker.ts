"use client";

import { useEffect, useRef } from "react";
import { useRouter } from "next/router";
import { trackPageView, getTelemetryConsent } from "../lib/telemetry";

/**
 * Sanitizes a URL path to prevent sensitive query parameters
 * (e.g. auth tokens, private keys, email addresses) from leaking into pageview telemetry.
 */
export function sanitizeAnalyticsPath(url: string): string {
  try {
    const parsed = new URL(url, "https://perigee.app");
    const params = new URLSearchParams(parsed.search);
    const forbidden = ["token", "key", "secret", "auth", "email", "address", "sig", "code"];

    for (const key of Array.from(params.keys())) {
      const lower = key.toLowerCase();
      if (forbidden.some((f) => lower.includes(f))) {
        params.delete(key);
      }
    }

    const cleanSearch = params.toString();
    return parsed.pathname + (cleanSearch ? `?${cleanSearch}` : "");
  } catch {
    return url.split("?")[0];
  }
}

/**
 * Hook to automatically track page views on client-side navigation.
 * Respects consent gating, GPC, and Do Not Track.
 */
export function useAnalyticsRouteTracker(): void {
  const router = useRouter();
  const lastTrackedPath = useRef<string | null>(null);

  useEffect(() => {
    if (!router) return;

    const handleRouteChange = (url: string) => {
      if (getTelemetryConsent() !== true) return;
      const cleanPath = sanitizeAnalyticsPath(url);

      if (lastTrackedPath.current === cleanPath) return;
      lastTrackedPath.current = cleanPath;

      const referrer = typeof document !== "undefined" ? document.referrer : undefined;
      trackPageView(cleanPath, referrer);
    };

    // Track initial page view on mount
    handleRouteChange(router.asPath || window.location.pathname);

    router.events.on("routeChangeComplete", handleRouteChange);
    return () => {
      router.events.off("routeChangeComplete", handleRouteChange);
    };
  }, [router]);
}
