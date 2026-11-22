"use client";

/**
 * RpcFallbackBanner
 *
 * Displays a non-intrusive amber banner when the Perigee backend is
 * unreachable, informing users that:
 *  - Live analysis / simulation is paused.
 *  - Any previously loaded data (analysis history, vault list) is still
 *    visible and served from the local cache.
 *  - The "as of <timestamp>" freshness indicator shows when data was last
 *    fetched successfully.
 *
 * The banner auto-dismisses the moment connectivity is restored and exposes
 * a context so sibling views can read `isRpcDown` / `lastHealthyAt` to
 * render their own stale-data overlays.
 *
 * Resolves WEB-29 (#115): no graceful RPC fallback + cached data display.
 */

import React, {
  createContext,
  useContext,
  useCallback,
  useMemo,
} from "react";
import { useTranslations } from "next-intl";
import { supportLinks } from "../lib/config";
import { useRpcStatus } from "../hooks/useRpcStatus";

// ---------------------------------------------------------------------------
// Context — lets any child component read the current RPC health state
// ---------------------------------------------------------------------------

export interface RpcFallbackContextValue {
  /** True while the backend is confirmed unreachable. */
  isRpcDown: boolean;
  /** True on the initial check before a result is known. */
  isChecking: boolean;
  /** Epoch-ms when the backend was last reachable (null = never seen healthy). */
  lastHealthyAt: number | null;
  /** Trigger an immediate re-check. */
  retry: () => void;
}

const RpcFallbackContext = createContext<RpcFallbackContextValue>({
  isRpcDown: false,
  isChecking: false,
  lastHealthyAt: null,
  retry: () => undefined,
});

/**
 * Hook for consuming the RPC fallback context in any child of
 * `<RpcFallbackBanner>`.  Use it to show stale-data badges or disable
 * live-data actions while the backend is unreachable.
 *
 * @example
 * const { isRpcDown, lastHealthyAt } = useRpcFallback();
 */
export function useRpcFallback(): RpcFallbackContextValue {
  return useContext(RpcFallbackContext);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatRelativeTime(epochMs: number): string {
  const diffMs = Date.now() - epochMs;
  const diffMin = Math.floor(diffMs / 60_000);
  if (diffMin < 1) return "just now";
  if (diffMin === 1) return "1 minute ago";
  if (diffMin < 60) return `${diffMin} minutes ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr === 1) return "1 hour ago";
  if (diffHr < 24) return `${diffHr} hours ago`;
  return new Date(epochMs).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export interface RpcFallbackBannerProps {
  apiUrl: string;
  /** Optional child subtree that consumes `useRpcFallback()`. */
  children?: React.ReactNode;
}

export function RpcFallbackBanner({ apiUrl, children }: RpcFallbackBannerProps) {
  const t = useTranslations();
  const { health, checking, lastHealthyAt, retry } = useRpcStatus(apiUrl);

  const isRpcDown = health === "unreachable";
  const isChecking = health === "unknown" || checking;

  const ctxValue = useMemo<RpcFallbackContextValue>(
    () => ({ isRpcDown, isChecking, lastHealthyAt, retry }),
    [isRpcDown, isChecking, lastHealthyAt, retry],
  );

  const handleRetry = useCallback(() => {
    retry();
  }, [retry]);

  return (
    <RpcFallbackContext.Provider value={ctxValue}>
      {isRpcDown && (
        <div
          role="alert"
          aria-live="assertive"
          aria-atomic="true"
          className="flex items-start justify-between gap-3 bg-amber-900/80 border-b border-amber-700 px-4 py-2.5 text-sm text-amber-100"
        >
          {/* Left — icon + message */}
          <div className="flex items-start gap-2 min-w-0">
            {/* Warning triangle */}
            <svg
              xmlns="http://www.w3.org/2000/svg"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              strokeLinecap="round"
              strokeLinejoin="round"
              className="h-4 w-4 shrink-0 mt-0.5 text-amber-300"
              aria-hidden="true"
            >
              <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>

            <span className="leading-snug">
              <strong>{t("rpc.unreachableTitle")}</strong>{" "}
              {t("rpc.unreachableBody")}
              {lastHealthyAt && (
                <>
                  {" "}
                  <span className="opacity-75 text-xs">
                    {t("rpc.cachedDataAsOf", {
                      time: formatRelativeTime(lastHealthyAt),
                    })}
                  </span>
                </>
              )}
              {!lastHealthyAt && (
                <>
                  {" "}
                  <span className="opacity-75 text-xs">
                    {t("rpc.cachedDataNote")}
                  </span>
                </>
              )}
              {"  "}
              <a
                href={supportLinks.supportUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2 hover:opacity-100 opacity-90"
              >
                {t("rpc.getHelp")}
              </a>
            </span>
          </div>

          {/* Right — action buttons */}
          <div className="flex shrink-0 items-center gap-2">
            <a
              href={supportLinks.statusUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="rounded border border-amber-600 px-2.5 py-1 text-xs font-medium
                         text-amber-200 hover:bg-amber-800 focus:outline-none focus:ring-2
                         focus:ring-amber-400 transition-colors"
            >
              {t("rpc.statusPage")}
            </a>
            <button
              type="button"
              onClick={handleRetry}
              disabled={checking}
              aria-label={t("rpc.retryAriaLabel")}
              className="rounded border border-amber-600 px-2.5 py-1 text-xs font-medium
                         text-amber-200 hover:bg-amber-800 focus:outline-none focus:ring-2
                         focus:ring-amber-400 focus:ring-offset-2 focus:ring-offset-amber-900
                         disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {checking ? t("rpc.checking") : t("rpc.retry")}
            </button>
          </div>
        </div>
      )}

      {children}
    </RpcFallbackContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// CachedDataBadge — lightweight overlay for individual views/cards
// ---------------------------------------------------------------------------

/**
 * Drop this badge into any view that serves stale data when the RPC is down.
 * It renders nothing when the backend is healthy.
 *
 * @example
 * <div className="relative">
 *   <VaultCard vault={staleVault} />
 *   <CachedDataBadge />
 * </div>
 */
export function CachedDataBadge({ className = "" }: { className?: string }) {
  const t = useTranslations();
  const { isRpcDown, lastHealthyAt } = useRpcFallback();

  if (!isRpcDown) return null;

  return (
    <span
      title={
        lastHealthyAt
          ? t("rpc.cachedDataAsOf", { time: formatRelativeTime(lastHealthyAt) })
          : t("rpc.cachedDataNote")
      }
      className={[
        "inline-flex items-center gap-1 rounded-full bg-amber-900/70 border border-amber-600/50",
        "px-2 py-0.5 text-[10px] font-medium text-amber-300 select-none",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
      aria-label={t("rpc.cachedBadgeAriaLabel")}
    >
      {/* Clock icon */}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        className="h-2.5 w-2.5"
        aria-hidden="true"
      >
        <circle cx="12" cy="12" r="10" />
        <polyline points="12 6 12 12 16 14" />
      </svg>
      {t("rpc.cachedBadgeLabel")}
    </span>
  );
}
