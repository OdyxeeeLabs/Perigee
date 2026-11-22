"use client";

/**
 * useRpcStatus
 *
 * Shared hook that tracks whether the Perigee backend RPC endpoint is
 * reachable. Consumers (RpcFallbackBanner, views, cached-data overlays) can
 * subscribe to a single source of truth instead of each running their own
 * health-check polling.
 *
 * The hook keeps the last-known-good timestamp so callers can surface
 * "data as of <time>" indicators when serving stale/cached content.
 */

import { useState, useEffect, useCallback, useRef } from "react";

export type RpcHealth = "unknown" | "healthy" | "unreachable";

export interface RpcStatus {
  /** Current reachability state of the backend. */
  health: RpcHealth;
  /** Whether a health check is in-flight. */
  checking: boolean;
  /** Epoch-ms timestamp when the backend was last confirmed reachable. */
  lastHealthyAt: number | null;
  /** Trigger a manual re-check immediately. */
  retry: () => void;
}

const RPC_CHECK_INTERVAL_MS = 30_000;
const RPC_TIMEOUT_MS = 5_000;

async function pingHealth(apiUrl: string): Promise<boolean> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), RPC_TIMEOUT_MS);
    const res = await fetch(`${apiUrl}/health`, {
      method: "HEAD",
      signal: controller.signal,
      cache: "no-store",
    });
    clearTimeout(timer);
    return res.ok;
  } catch {
    return false;
  }
}

export function useRpcStatus(apiUrl: string): RpcStatus {
  const [health, setHealth] = useState<RpcHealth>("unknown");
  const [checking, setChecking] = useState(false);
  const [lastHealthyAt, setLastHealthyAt] = useState<number | null>(null);
  const [retryTick, setRetryTick] = useState(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const runCheck = useCallback(async () => {
    if (!mountedRef.current) return;
    setChecking(true);
    const ok = await pingHealth(apiUrl);
    if (!mountedRef.current) return;
    setChecking(false);
    setHealth(ok ? "healthy" : "unreachable");
    if (ok) setLastHealthyAt(Date.now());
  }, [apiUrl]);

  useEffect(() => {
    runCheck();
    const interval = setInterval(runCheck, RPC_CHECK_INTERVAL_MS);
    return () => clearInterval(interval);
    // retryTick intentionally included so a manual retry re-runs the check
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runCheck, retryTick]);

  const retry = useCallback(() => setRetryTick((n) => n + 1), []);

  return { health, checking, lastHealthyAt, retry };
}
