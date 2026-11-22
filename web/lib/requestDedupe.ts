/**
 * Request deduplication (issue #690).
 *
 * The repo has no SWR/TanStack Query dependency, so the behaviour described in
 * the issue is provided natively and plugged into `cachedGet`:
 *   - {@link dedupedFetch}: concurrent GETs for the same key share one promise;
 *     a settled result is reused for `dedupingInterval` ms (default 2000);
 *   - {@link coalesceMutation}: rapid repeated mutations (double clicks) are
 *     debounced so only the last call is sent, and every caller gets its result;
 *   - {@link ttlForPath}: configurable cache TTL per endpoint;
 *   - {@link getDedupeStats}: duplicate-request rate, also reported through the
 *     consent-gated telemetry module.
 */

import { trackTelemetryEvent } from "./telemetry";

export const DEFAULT_DEDUPING_INTERVAL_MS = 2000;
export const DEFAULT_MUTATION_DEBOUNCE_MS = 300;

/** Per-endpoint cache TTLs in ms. The longest matching prefix wins. */
export const ENDPOINT_TTL_MS: Record<string, number> = {
  "/analyze": 5 * 60 * 1000,
  "/managers": 60 * 1000,
  "/contracts": 10 * 60 * 1000,
  "/health": 5 * 1000,
};

export function ttlForPath(path: string, fallbackMs = 5 * 60 * 1000): number {
  const normalized = path.startsWith("/") ? path : `/${path}`;
  let best: string | undefined;
  for (const prefix of Object.keys(ENDPOINT_TTL_MS)) {
    if (normalized.startsWith(prefix) && (!best || prefix.length > best.length)) {
      best = prefix;
    }
  }
  return best ? ENDPOINT_TTL_MS[best] : fallbackMs;
}

interface InFlight {
  promise: Promise<unknown>;
  settledAt?: number;
}

const inFlight = new Map<string, InFlight>();
const stats = { total: 0, deduped: 0 };
const REPORT_EVERY = 20;

export function getDedupeStats(): { total: number; deduped: number; duplicateRate: number } {
  return {
    ...stats,
    duplicateRate: stats.total === 0 ? 0 : stats.deduped / stats.total,
  };
}

export function resetDedupeState(): void {
  inFlight.clear();
  stats.total = 0;
  stats.deduped = 0;
}

function maybeReport(): void {
  if (stats.total % REPORT_EVERY !== 0) return;
  trackTelemetryEvent({
    name: "request_dedupe_rate",
    properties: {
      total: stats.total,
      deduped: stats.deduped,
      rate: Number((stats.deduped / stats.total).toFixed(3)),
    },
  });
}

export async function dedupedFetch<T>(
  key: string,
  fetcher: () => Promise<T>,
  dedupingInterval: number = DEFAULT_DEDUPING_INTERVAL_MS,
  now: () => number = Date.now,
): Promise<T> {
  stats.total += 1;

  const existing = inFlight.get(key);
  if (existing) {
    const fresh =
      existing.settledAt === undefined || now() - existing.settledAt <= dedupingInterval;
    if (fresh) {
      stats.deduped += 1;
      maybeReport();
      return existing.promise as Promise<T>;
    }
    inFlight.delete(key);
  }

  const entry: InFlight = { promise: Promise.resolve() };
  entry.promise = fetcher().then(
    (value) => {
      entry.settledAt = now();
      return value;
    },
    (error) => {
      // Never cache failures: the next caller must be able to retry.
      if (inFlight.get(key) === entry) inFlight.delete(key);
      throw error;
    },
  );
  inFlight.set(key, entry);
  maybeReport();
  return entry.promise as Promise<T>;
}

interface PendingMutation {
  timer: ReturnType<typeof setTimeout>;
  waiters: Array<{ resolve: (v: unknown) => void; reject: (e: unknown) => void }>;
}

const pendingMutations = new Map<string, PendingMutation>();

/**
 * Debounce rapid repeated mutations with the same key: only the last `send`
 * is executed after `waitMs`, and all callers receive that single result.
 */
export function coalesceMutation<T>(
  key: string,
  send: () => Promise<T>,
  waitMs: number = DEFAULT_MUTATION_DEBOUNCE_MS,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const previous = pendingMutations.get(key);
    const waiters = previous ? previous.waiters : [];
    if (previous) clearTimeout(previous.timer);
    waiters.push({ resolve: resolve as (v: unknown) => void, reject });

    const timer = setTimeout(() => {
      pendingMutations.delete(key);
      send().then(
        (value) => waiters.forEach((w) => w.resolve(value)),
        (error) => waiters.forEach((w) => w.reject(error)),
      );
    }, waitMs);

    pendingMutations.set(key, { timer, waiters });
  });
}
