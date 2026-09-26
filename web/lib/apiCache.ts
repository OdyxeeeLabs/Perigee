/**
 * apiCache.ts
 *
 * Thin wrappers around the native `fetch` API that leverage Next.js 13+
 * built-in data caching (the "fetch cache").  Use these helpers instead of
 * raw `fetch` calls inside Server Components and Route Handlers so responses
 * are cached at the framework level without an extra CDN layer.
 *
 * Resolves WEB-60 (#193): API response caching not leveraged.
 *
 * @see https://nextjs.org/docs/app/building-your-application/data-fetching/fetching-caching-and-revalidating
 */

import { API_URL } from "./api";

/** Default time-to-live for cached API responses (seconds). */
const DEFAULT_REVALIDATE = 60;

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
}

/**
 * Cached GET helper.  Wraps `fetch` with Next.js `next.revalidate` / `next.tags`
 * so the response is stored in the framework's data cache and invalidated
 * automatically when the TTL expires or when `revalidateTag` is called.
 *
 * @example
 * const data = await cachedGet<AnalyzeResponse>("/analyze", { revalidate: 30 });
 */
export async function cachedGet<T>(
  path: string,
  { revalidate = DEFAULT_REVALIDATE, tags = [], ...init }: CachedFetchOptions = {},
): Promise<T> {
  const url = `${API_URL}${path.startsWith("/") ? path : `/${path}`}`;

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

  return res.json() as Promise<T>;
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
