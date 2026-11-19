/**
 * Decides whether speculative prefetching is acceptable (issue #691).
 *
 * Prefetching is skipped when the user asked to save data
 * (`navigator.connection.saveData` or the `prefers-reduced-data` media query)
 * or when the connection is slow (`effectiveType` of 2g / slow-2g).
 */

interface NetworkInformationLike {
  saveData?: boolean;
  effectiveType?: string;
}

export const HOVER_PREFETCH_DELAY_MS = 200;

export function shouldPrefetch(): boolean {
  if (typeof window === "undefined") return false;

  const connection = (navigator as Navigator & { connection?: NetworkInformationLike }).connection;
  if (connection?.saveData) return false;
  if (connection?.effectiveType === "slow-2g" || connection?.effectiveType === "2g") return false;

  if (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-data: reduce)").matches
  ) {
    return false;
  }

  return true;
}
