"use client";

import { useCallback, useEffect, useState } from "react";
import { useRouter } from "next/router";

import {
  DATE_RANGE_PARAM,
  DEFAULT_DATE_RANGE,
  dateRangeBounds,
  dateRangeToApiParams,
  dateRangeToQuery,
  getDateRangePreset,
  parseDateRange,
} from "@/lib/dateRange";
import type { DateRangeId } from "@/lib/dateRange";
import { formatDate } from "@/lib/locale/format";
import { normalizeLocale } from "@/lib/locale/locales";

export function useDateRange() {
  const router = useRouter();
  const [range, setRangeState] = useState<DateRangeId>(DEFAULT_DATE_RANGE);
  const [now, setNow] = useState<Date | null>(null);

  useEffect(() => {
    setNow(new Date());

    const syncFromUrl = () => {
      const raw =
        typeof window === "undefined"
          ? undefined
          : new URLSearchParams(window.location.search).get(DATE_RANGE_PARAM);
      setRangeState(parseDateRange(raw));
    };

    syncFromUrl();

    const events = router?.events;
    events?.on("routeChangeComplete", syncFromUrl);
    if (typeof window !== "undefined") {
      window.addEventListener("popstate", syncFromUrl);
    }

    return () => {
      events?.off("routeChangeComplete", syncFromUrl);
      if (typeof window !== "undefined") {
        window.removeEventListener("popstate", syncFromUrl);
      }
    };
  }, [router?.events]);

  const setRange = useCallback(
    (next: DateRangeId) => {
      const value = parseDateRange(next);
      setRangeState(value);
      if (!router) return;
      void router.replace(
        { pathname: router.pathname, query: dateRangeToQuery(value, router.query) },
        undefined,
        { shallow: true, scroll: false },
      );
    },
    [router],
  );

  const { from, to } = now
    ? dateRangeBounds(range, now)
    : { from: null, to: new Date(0) };
  const locale = normalizeLocale(router?.locale);
  const preset = getDateRangePreset(range);
  const label = from
    ? `${preset.label} (${formatDate(from, locale)} – ${formatDate(to, locale)})`
    : preset.label;

  return {
    range,
    setRange,
    label,
    from,
    to,
    apiParams: dateRangeToApiParams(range),
  };
}
