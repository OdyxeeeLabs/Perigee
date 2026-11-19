"use client";

/**
 * Progressive hydration entry for the analytics consent banner (#694).
 *
 * The banner is not needed for first interaction, so it is loaded with
 * `next/dynamic` (client only) inside a `Suspense` boundary and hydrates after
 * the critical UI: header/navigation, command palette, wallet connection and
 * primary CTAs. This wrapper is also where overall hydration completion time
 * is measured, because its effect only runs once the page has hydrated.
 */

import dynamic from "next/dynamic";
import { Suspense, useEffect } from "react";
import { measureHydration } from "../lib/hydrationMetrics";
import { useAnalyticsRouteTracker } from "../hooks/useAnalyticsRouteTracker";

const AnalyticsBanner = dynamic(
  () => import("./AnalyticsBanner").then((mod) => mod.AnalyticsBanner),
  { ssr: false },
);

export function Analytics() {
  useAnalyticsRouteTracker();

  useEffect(() => {
    measureHydration();
  }, []);

  return (
    <Suspense fallback={null}>
      <AnalyticsBanner />
    </Suspense>
  );
}
