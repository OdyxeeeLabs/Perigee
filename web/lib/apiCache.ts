/**
 * apiCache.ts
 *
 * Caching helpers for the Perigee API layer.
 *
 * Responses go through a bounded, in-memory TTL + LRU cache so that repeated
 * reads inside a long-running SPA session cannot grow memory without limit,
 * while the Next.js 13+ "fetch cache" (`next.revalidate` / `next.tags`) is
 * still applied for Server Components and Route Handlers.
 *
 * Resolves WEB-60 (#193): API response caching not leveraged.
 * Resolves FE-018 (#225): apiCache had no TTL eviction and grew unbounded.
 *
 * @see https://nextjs.org/docs/app/building-your-application/data-fetching/fetching-caching-and-revalidating
 */

import { API_URL } from "./api";

/** Default time-to-live for cached API responses (seconds). */
const DEFAULT_REVALIDATE = 60;

/** Default in-memory TTL for cached API responses: five minutes. */
export const DEFAULT_TTL_MS = 5 * 60 * 1000;

/** Default upper bound on the number of entries held in memory (LRU). */
export const DEFAULT_MAX_ENTRIES = 100;

export interface CachedFetchOptions extends RequestInit {
  /**
   * Seconds before the cached response is considered stale and re-fetched in
   * the background (Incremental Static Regeneration semantics).
   * Pass 0 to opt into `no-store` (always fresh).
   * Defaults to 60 s.
   */
  revalidate?: number;
  /**
   * On-demand revalidation tags used with `revalidateTag()` in Route Handlers.
   */
  tags?: string[];
  /**
   * In-memory time-to-live in milliseconds for the shared `apiCache`.
   * Analyze results default to {@link DEFAULT_TTL_MS} (5 minutes).
   * Ignored when `revalidate` is `0` (that path bypasses caching entirely).
   */
  ttlMs?: number;
}

// ---------------------------------------------------------------------------
// In-memory TTL + LRU cache
// ---------------------------------------------------------------------------

interface ApiCacheEntry<V> {
  value: V;
  /** Epoch milliseconds after which the entry is stale. */
  expiresAt: number;
}

export interface ApiCacheOptions {
  /** Default entry TTL in milliseconds. Defaults to {@link DEFAULT_TTL_MS}. */
  ttlMs?: number;
  /** Maximum entries retained before the least-recently-used one is dropped. */
  maxEntries?: number;
  /** Injectable clock (epoch ms); primarily for deterministic tests. */
  now?: () => number;
}

/**
 * Bounded cache with per-entry TTL expiry and least-recently-used eviction.
 *
 * * `get()` drops an entry once its TTL has elapsed and re-inserts a live hit
 *   so the most-recently-used entries are evicted last.
 * * `set()` prunes expired entries and then evicts the oldest entries until the
 *   cache is back at or below `maxEntries`.
 */
export class TtlLruCache<V = unknown> {
  private readonly entries = new Map<string, ApiCacheEntry<V>>();
  private readonly defaultTtlMs: number;
  private readonly maxEntries: number;
  private readonly now: () => number;

  constructor({
    ttlMs = DEFAULT_TTL_MS,
    maxEntries = DEFAULT_MAX_ENTRIES,
    now = Date.now,
  }: ApiCacheOptions = {}) {
    this.defaultTtlMs = ttlMs;
    this.maxEntries = Math.max(1, Math.floor(maxEntries));
    this.now = now;
  }

  /** Number of entries currently held (expired entries included until read). */
  get size(): number {
    return this.entries.size;
  }

  /** Return a live entry, refreshing its LRU position; `undefined` if stale. */
  get(key: string): V | undefined {
    const entry = this.entries.get(key);
    if (!entry) {
      return undefined;
    }
    if (entry.expiresAt <= this.now()) {
      this.entries.delete(key);
      return undefined;
    }
    // Delete + re-insert marks this key as most-recently-used.
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }

  /** Whether a live (non-expired) entry exists for `key`. */
  has(key: string): boolean {
    return this.get(key) !== undefined;
  }

  /** Store `value` under `key`, applying `ttlMs` (defaults to the configured TTL). */
  set(key: string, value: V, ttlMs: number = this.defaultTtlMs): void {
    this.prune();
    if (this.entries.has(key)) {
      this.entries.delete(key);
    }
    this.entries.set(key, { value, expiresAt: this.now() + ttlMs });
    this.evictOverflow();
  }

  /** Remove a single entry. Returns whether it existed. */
  delete(key: string): boolean {
    return this.entries.delete(key);
  }

  /** Remove every entry. */
  clear(): void {
    this.entries.clear();
  }

  /** Drop all expired entries; returns how many were removed. */
  prune(): number {
    const now = this.now();
    let removed = 0;
    for (const [key, entry] of this.entries) {
      if (entry.expiresAt <= now) {
        this.entries.delete(key);
        removed++;
      }
    }
    return removed;
  }

  private evictOverflow(): void {
    while (this.entries.size > this.maxEntries) {
      const oldest = this.entries.keys().next().value as string | undefined;
      if (oldest === undefined) {
        return;
      }
      this.entries.delete(oldest);
    }
  }
}

/** Shared in-memory cache backing `cachedGet`. */
export const apiCache = new TtlLruCache();

/** Manually invalidate every entry in the shared in-memory cache. */
export function clearCache(): void {
  apiCache.clear();
}

/**
 * Canonical cache key for a GET path. Normalises a missing leading slash so
 * `"analyze"` and `"/analyze"` share one entry.
 */
export function cacheKey(path: string): string {
  return `GET ${path.startsWith("/") ? path : `/${path}`}`;
}

// ---------------------------------------------------------------------------
// Fetch helpers
// ---------------------------------------------------------------------------

/**
 * Cached GET helper.  Wraps `fetch` with Next.js `next.revalidate` / `next.tags`
 * so the response is stored in the framework's data cache, and keeps a copy in
 * the bounded in-memory {@link apiCache} so repeat reads in one session do not
 * refetch.  Pass `revalidate: 0` to bypass both caches.
 *
 * @example
 * const data = await cachedGet<AnalyzeResponse>("/analyze", { revalidate: 30 });
 * // Analyze results are cached in memory for 5 minutes by default:
 * const fresh = await cachedGet<AnalyzeResponse>("/analyze");
 */
export async function cachedGet<T>(
  path: string,
  {
    revalidate = DEFAULT_REVALIDATE,
    tags = [],
    ttlMs = DEFAULT_TTL_MS,
    ...init
  }: CachedFetchOptions = {},
): Promise<T> {
  const url = `${API_URL}${path.startsWith("/") ? path : `/${path}`}`;
  const key = cacheKey(path);
  const useMemoryCache = revalidate !== 0;

  if (useMemoryCache) {
    const hit = apiCache.get(key);
    if (hit !== undefined) {
      return hit as T;
    }
  }

  const res = await fetch(url, {
    ...init,
    method: "GET",
    headers: {
      "Content-Type": "application/json",
      ...(init.headers as Record<string, string> | undefined),
    },
    next: {
      revalidate: revalidate === 0 ? undefined : revalidate,
      tags: tags.length > 0 ? tags : undefined,
    },
    // When revalidate === 0 the caller wants no caching at all
    cache: revalidate === 0 ? "no-store" : undefined,
  });

  if (!res.ok) {
    const body = await res.text().catch(() => res.statusText);
    throw new Error(`API ${res.status}: ${body}`);
  }

  const data = (await res.json()) as T;

  if (useMemoryCache) {
    apiCache.set(key, data, ttlMs);
  }

  return data;
}

/**
 * Non-cached POST helper.  POSTs are inherently mutation-bearing so they
 * always bypass the data cache (`cache: "no-store"`).
 *
 * @example
 * const result = await uncachedPost<AnalyzeResponse>("/analyze", payload);
 */
export async function uncachedPost<T>(
  path: string,
  body: unknown,
  init: Omit<RequestInit, "body" | "method"> = {},
): Promise<T> {
  const url = `${API_URL}${path.startsWith("/") ? path : `/${path}`}`;

  const res = await fetch(url, {
    ...init,
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(init.headers as Record<string, string> | undefined),
    },
    body: JSON.stringify(body),
    cache: "no-store",
  });

  if (!res.ok) {
    const body = await res.text().catch(() => res.statusText);
    throw new Error(`API ${res.status}: ${body}`);
  }

  return res.json() as Promise<T>;
}

// ---------------------------------------------------------------------------
// Named cache tag constants — use with revalidateTag() in Route Handlers
// ---------------------------------------------------------------------------

export const CACHE_TAGS = {
  analyze: "perigee-analyze",
  managers: "perigee-managers",
  contracts: "perigee-contracts",
} as const;
