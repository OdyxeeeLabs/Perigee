"use client";

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import {
  initialSectionState,
  isEmptyHistory,
  sectionReducer,
  type AnalyticsSectionAction,
  type AnalyticsSectionState,
} from "../lib/analytics/sectionState";

export interface AnalyticsSectionController<T> {
  state: AnalyticsSectionState<T>;
  retry: () => void;
}

export function useAnalyticsSection<T>(
  load: () => Promise<T | null>,
  reloadKey: string,
): AnalyticsSectionController<T> {
  const [state, dispatch] = useReducer(
    (current: AnalyticsSectionState<T>, action: AnalyticsSectionAction<T>) =>
      sectionReducer(current, action),
    undefined,
    () => initialSectionState<T>(),
  );
  const [retryTick, setRetryTick] = useState(0);
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    let active = true;
    dispatch({ type: "load" });

    loadRef
      .current()
      .then((data) => {
        if (!active) return;
        dispatch({
          type: "succeed",
          data: data as T,
          empty: isEmptyHistory(data),
        });
      })
      .catch(() => {
        if (!active) return;
        dispatch({ type: "fail", message: "This analytics section could not be loaded." });
      });

    return () => {
      active = false;
    };
  }, [reloadKey, retryTick]);

  const retry = useCallback(() => setRetryTick((tick) => tick + 1), []);

  return { state, retry };
}
