"use client";

import type { ReactNode } from "react";
import type { AnalyticsSectionState } from "../lib/analytics/sectionState";

interface AnalyticsSectionProps<T> {
  title: string;
  state: AnalyticsSectionState<T>;
  onRetry: () => void;
  emptyLabel: string;
  children: (data: T) => ReactNode;
  className?: string;
  testId?: string;
}

export function AnalyticsSection<T>({
  title,
  state,
  onRetry,
  emptyLabel,
  children,
  className = "",
  testId,
}: AnalyticsSectionProps<T>) {
  const testIdAttr = testId ? `analytics-section-${testId}` : undefined;

  if (state.status === "loading") {
    return (
      <div
        role="status"
        aria-label={`${title} loading`}
        data-testid={testIdAttr}
        className={`rounded-xl border border-slate-800 bg-slate-900/40 p-6 ${className}`}
      >
        <p className="text-xs text-slate-400">Loading {title.toLowerCase()}…</p>
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div
        role="alert"
        aria-label={`${title} unavailable`}
        data-testid={testIdAttr ? `${testIdAttr}-error` : undefined}
        className={`rounded-xl border border-amber-900/60 bg-amber-950/20 p-6 text-amber-200 ${className}`}
      >
        <p className="text-sm font-semibold">{title} data is unavailable.</p>
        <p className="mt-1 text-xs text-amber-300/80">
          You can retry this section without leaving the page.
        </p>
        <button
          type="button"
          onClick={onRetry}
          className="mt-3 rounded-md border border-amber-700 px-3 py-1.5 text-xs font-medium hover:bg-amber-900/40"
        >
          Retry {title.toLowerCase()}
        </button>
      </div>
    );
  }

  if (state.status === "empty") {
    return (
      <div
        data-testid={testIdAttr ? `${testIdAttr}-empty` : undefined}
        className={`rounded-xl border border-slate-800 bg-slate-900/40 p-6 ${className}`}
      >
        <p className="py-4 text-center text-xs text-slate-400">{emptyLabel}</p>
      </div>
    );
  }

  return <>{state.data !== null && children(state.data)}</>;
}
