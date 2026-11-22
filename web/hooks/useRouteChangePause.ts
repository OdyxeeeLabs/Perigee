"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/router";

/**
 * Pages Router navigation pause for background polling. The timer's own
 * effect cleanup still runs on unmount; this makes the pause explicit on
 * `routeChangeStart` so a slow read cannot be scheduled mid-navigation.
 */
export function useRouteChangePause(): boolean {
  const router = useRouter();
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    if (!router?.events) return;
    const pause = () => setPaused(true);
    const resume = () => setPaused(false);
    router.events.on("routeChangeStart", pause);
    router.events.on("routeChangeComplete", resume);
    router.events.on("routeChangeError", resume);
    return () => {
      router.events.off("routeChangeStart", pause);
      router.events.off("routeChangeComplete", resume);
      router.events.off("routeChangeError", resume);
    };
  }, [router]);

  return paused;
}
