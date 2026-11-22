export type AnalyticsSectionStatus = "loading" | "ready" | "empty" | "error";

export interface AnalyticsSectionState<T> {
  status: AnalyticsSectionStatus;
  data: T | null;
  error: string | null;
  attempt: number;
}

export type AnalyticsSectionAction<T> =
  | { type: "load" }
  | { type: "succeed"; data: T; empty: boolean }
  | { type: "fail"; message: string };

export function initialSectionState<T>(): AnalyticsSectionState<T> {
  return { status: "loading", data: null, error: null, attempt: 0 };
}

export function sectionReducer<T>(
  state: AnalyticsSectionState<T>,
  action: AnalyticsSectionAction<T>,
): AnalyticsSectionState<T> {
  switch (action.type) {
    case "load":
      return {
        status: "loading",
        data: state.data,
        error: null,
        attempt: state.attempt + 1,
      };
    case "succeed":
      return {
        status: action.empty ? "empty" : "ready",
        data: action.data,
        error: null,
        attempt: state.attempt,
      };
    case "fail":
      return {
        status: "error",
        data: state.data,
        error: action.message,
        attempt: state.attempt,
      };
  }
}

export function isEmptyHistory(value: unknown): boolean {
  if (value === null || value === undefined) return true;
  if (Array.isArray(value)) return value.length === 0;
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    if ("sampleCount" in record) {
      return Number(record.sampleCount) === 0;
    }
    if ("recentEvents" in record) {
      return Array.isArray(record.recentEvents) && record.recentEvents.length === 0;
    }
  }
  return false;
}
