"use client";

import { useCallback, useEffect, useRef } from "react";
import { useRouter } from "next/router";
import { HOVER_PREFETCH_DELAY_MS, shouldPrefetch } from "../lib/prefetchPolicy";

/**
 * Prefetch a route after the pointer has rested on an element for
 * `delayMs` (default 200ms, issue #691). The pending prefetch is cancelled
 * when the pointer leaves, the element loses focus, or a navigation starts
 * (rapid navigation).
 */
export function usePrefetchOnHover(href: string, delayMs: number = HOVER_PREFETCH_DELAY_MS) {
  const router = useRouter();
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const prefetched = useRef(false);

  const cancel = useCallback(() => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  const schedule = useCallback(() => {
    if (prefetched.current || timer.current || !shouldPrefetch()) return;
    timer.current = setTimeout(() => {
      timer.current = null;
      prefetched.current = true;
      void router.prefetch(href);
    }, delayMs);
  }, [delayMs, href, router]);

  useEffect(() => {
    router.events?.on("routeChangeStart", cancel);
    return () => {
      router.events?.off("routeChangeStart", cancel);
      cancel();
    };
  }, [cancel, router.events]);

  return {
    onMouseEnter: schedule,
    onFocus: schedule,
    onMouseLeave: cancel,
    onBlur: cancel,
  };
}

/**
 * Prefetch the next page of a paginated list as soon as the current one has
 * rendered, using the supplied loader (e.g. a `cachedGet` call that warms the
 * shared cache). Does nothing when prefetching is not allowed or there is no
 * next page.
 */
export function usePrefetchNextPage(
  hasNextPage: boolean,
  page: number,
  load: (nextPage: number) => Promise<unknown>,
): void {
  useEffect(() => {
    if (!hasNextPage || !shouldPrefetch()) return;
    let cancelled = false;
    const id = setTimeout(() => {
      if (!cancelled) void load(page + 1).catch(() => undefined);
    }, HOVER_PREFETCH_DELAY_MS);
    return () => {
      cancelled = true;
      clearTimeout(id);
    };
  }, [hasNextPage, page, load]);
}
