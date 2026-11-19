// web/lib/apiCache.test.cjs — unit tests for the TTL + LRU in-memory cache in
// web/lib/apiCache.ts (Issue #225 / FE-018). Runs with
// `node --test ./lib/**/*.test.cjs`.
//
// `package.json`'s `test` script only executes `.cjs` files, so — following the
// convention already used by web/lib/sanitize.test.cjs and
// web/lib/api-middleware.test.cjs — the cache logic is mirrored here in plain
// JS and pinned with the same assertions. A final "source contract" test reads
// web/lib/apiCache.ts directly so the real module cannot drift from the mirror.
//
// SCOPE: the mirror covers the in-memory cache (`TtlLruCache`, the shared
// `apiCache`, `clearCache()`) and the memory-cache interaction inside
// `cachedGet`. Network/retry behaviour lives in web/lib/api.ts.
//
// If you change behaviour in web/lib/apiCache.ts, mirror the change here.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

/* ── Mirror of web/lib/apiCache.ts ───────────────────────────────────── */

const DEFAULT_TTL_MS = 5 * 60 * 1000;
const DEFAULT_MAX_ENTRIES = 100;

class TtlLruCache {
  constructor({ ttlMs = DEFAULT_TTL_MS, maxEntries = DEFAULT_MAX_ENTRIES, now = Date.now } = {}) {
    this.entries = new Map();
    this.defaultTtlMs = ttlMs;
    this.maxEntries = Math.max(1, Math.floor(maxEntries));
    this.now = now;
  }

  get size() {
    return this.entries.size;
  }

  get(key) {
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

  has(key) {
    return this.get(key) !== undefined;
  }

  set(key, value, ttlMs = this.defaultTtlMs) {
    this.prune();
    if (this.entries.has(key)) {
      this.entries.delete(key);
    }
    this.entries.set(key, { value, expiresAt: this.now() + ttlMs });
    this.evictOverflow();
  }

  delete(key) {
    return this.entries.delete(key);
  }

  clear() {
    this.entries.clear();
  }

  prune() {
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

  evictOverflow() {
    while (this.entries.size > this.maxEntries) {
      const oldest = this.entries.keys().next().value;
      if (oldest === undefined) {
        return;
      }
      this.entries.delete(oldest);
    }
  }
}

const apiCache = new TtlLruCache();

function clearCache() {
  apiCache.clear();
}

function cacheKey(p) {
  return `GET ${p.startsWith('/') ? p : `/${p}`}`;
}

/** Mirror of `cachedGet`'s memory-cache interaction (fetch is injectable). */
function makeCachedGet({ cache, fetchImpl, apiUrl = 'http://localhost:8080' }) {
  return async function cachedGet(p, { revalidate = 60, ttlMs = DEFAULT_TTL_MS } = {}) {
    const url = `${apiUrl}${p.startsWith('/') ? p : `/${p}`}`;
    const key = cacheKey(p);
    const useMemoryCache = revalidate !== 0;

    if (useMemoryCache) {
      const hit = cache.get(key);
      if (hit !== undefined) {
        return hit;
      }
    }

    const res = await fetchImpl(url);
    if (!res.ok) {
      const body = await res.text().catch(() => res.statusText);
      throw new Error(`API ${res.status}: ${body}`);
    }
    const data = await res.json();
    if (useMemoryCache) {
      cache.set(key, data, ttlMs);
    }
    return data;
  };
}

/* ── Test helpers ────────────────────────────────────────────────────── */

function fakeClock(start = 0) {
  let t = start;
  return {
    now: () => t,
    advance: (ms) => {
      t += ms;
    },
  };
}

function jsonResponse(payload, { ok = true, status = 200 } = {}) {
  return {
    ok,
    status,
    statusText: String(status),
    json: async () => payload,
    text: async () => JSON.stringify(payload),
  };
}

/* ── Defaults ────────────────────────────────────────────────────────── */

test('defaults: TTL is 5 minutes and cap is 100 entries', () => {
  const cache = new TtlLruCache();
  assert.equal(DEFAULT_TTL_MS, 300000);
  assert.equal(DEFAULT_MAX_ENTRIES, 100);
  assert.equal(cache.maxEntries, 100);
  assert.equal(cache.defaultTtlMs, 300000);
});

/* ── TTL expiry ──────────────────────────────────────────────────────── */

test('TTL: an entry survives until the TTL elapses, then expires', () => {
  const clock = fakeClock();
  const cache = new TtlLruCache({ ttlMs: 1000, now: clock.now });

  cache.set('GET /analyze', { ok: true });
  assert.deepEqual(cache.get('GET /analyze'), { ok: true }, 'live before TTL');

  clock.advance(999);
  assert.deepEqual(cache.get('GET /analyze'), { ok: true }, 'still live at TTL-1ms');

  clock.advance(1);
  assert.equal(cache.get('GET /analyze'), undefined, 'expired exactly at TTL');
  assert.equal(cache.size, 0, 'expired entry is evicted on read');
});

test('TTL: a per-entry ttlMs overrides the cache default', () => {
  const clock = fakeClock();
  const cache = new TtlLruCache({ ttlMs: 1000, now: clock.now });

  cache.set('short', 'a', 100);
  cache.set('long', 'b', 10000);

  clock.advance(500);
  assert.equal(cache.get('short'), undefined);
  assert.equal(cache.get('long'), 'b');
});

test('TTL: prune() drops expired entries in bulk', () => {
  const clock = fakeClock();
  const cache = new TtlLruCache({ ttlMs: 1000, now: clock.now });

  cache.set('a', 1);
  cache.set('b', 2);

  clock.advance(1001);
  assert.equal(cache.prune(), 2);
  assert.equal(cache.size, 0);
});

/* ── LRU eviction ────────────────────────────────────────────────────── */

test('LRU: setting beyond the cap evicts the least-recently-used entry', () => {
  const cache = new TtlLruCache({ maxEntries: 3 });

  cache.set('a', 1);
  cache.set('b', 2);
  cache.set('c', 3);
  assert.equal(cache.size, 3);

  cache.set('d', 4);

  assert.equal(cache.size, 3, 'never grows past the cap');
  assert.equal(cache.get('a'), undefined, 'oldest entry evicted');
  assert.equal(cache.get('b'), 2);
  assert.equal(cache.get('c'), 3);
  assert.equal(cache.get('d'), 4);
});

test('LRU: a read refreshes recency so hot keys survive eviction', () => {
  const cache = new TtlLruCache({ maxEntries: 3 });

  cache.set('a', 1);
  cache.set('b', 2);
  cache.set('c', 3);

  cache.get('a'); // 'a' is now most-recently-used; 'b' is oldest

  cache.set('d', 4);

  assert.equal(cache.get('b'), undefined, 'cold key evicted');
  assert.equal(cache.get('a'), 1, 'recently-read key retained');
  assert.equal(cache.get('c'), 3);
  assert.equal(cache.get('d'), 4);
});

test('LRU: overwriting a key keeps it most-recent and does not duplicate', () => {
  const cache = new TtlLruCache({ maxEntries: 2 });

  cache.set('a', 1);
  cache.set('b', 2);
  cache.set('a', 10);

  assert.equal(cache.size, 2);
  assert.equal(cache.get('a'), 10);
  assert.equal(cache.get('b'), 2);
});

/* ── clearCache() ────────────────────────────────────────────────────── */

test('clearCache(): empties the shared cache and is safe to call twice', () => {
  apiCache.set(cacheKey('/analyze'), { ok: true });
  apiCache.set(cacheKey('/managers'), [1, 2, 3]);
  assert.equal(apiCache.size, 2);

  clearCache();

  assert.equal(apiCache.size, 0);
  assert.equal(apiCache.get(cacheKey('/analyze')), undefined);
  assert.doesNotThrow(() => clearCache());
});

/* ── cachedGet(): no refetch on a live hit ───────────────────────────── */

test('cachedGet: a non-expired hit returns the cached value without refetching', async () => {
  const cache = new TtlLruCache();
  let calls = 0;
  const fetchImpl = async () => {
    calls++;
    return jsonResponse({ hit: calls });
  };
  const cachedGet = makeCachedGet({ cache, fetchImpl });

  const first = await cachedGet('/analyze', { ttlMs: 60000 });
  const second = await cachedGet('/analyze');

  assert.deepEqual(first, { hit: 1 });
  assert.deepEqual(second, { hit: 1 }, 'served from memory, not a new response');
  assert.equal(calls, 1, 'fetch called exactly once');
});

test('cachedGet: after clearCache() the next read refetches', async () => {
  const cache = new TtlLruCache();
  let calls = 0;
  const fetchImpl = async () => {
    calls++;
    return jsonResponse({ hit: calls });
  };
  const cachedGet = makeCachedGet({ cache, fetchImpl });

  await cachedGet('/analyze');
  cache.clear();
  const afterClear = await cachedGet('/analyze');

  assert.equal(calls, 2);
  assert.deepEqual(afterClear, { hit: 2 });
});

test('cachedGet: an expired entry is refetched', async () => {
  const cache = new TtlLruCache();
  let calls = 0;
  const fetchImpl = async () => {
    calls++;
    return jsonResponse({ hit: calls });
  };
  const cachedGet = makeCachedGet({ cache, fetchImpl });

  await cachedGet('/analyze', { ttlMs: 0 });
  const second = await cachedGet('/analyze');

  assert.equal(calls, 2, 'expired entry forced a refetch');
  assert.deepEqual(second, { hit: 2 });
});

test('cachedGet: revalidate: 0 bypasses the memory cache entirely', async () => {
  const cache = new TtlLruCache();
  let calls = 0;
  const fetchImpl = async () => {
    calls++;
    return jsonResponse({ hit: calls });
  };
  const cachedGet = makeCachedGet({ cache, fetchImpl });

  await cachedGet('/analyze', { revalidate: 0 });
  await cachedGet('/analyze', { revalidate: 0 });

  assert.equal(calls, 2);
  assert.equal(cache.size, 0, 'nothing stored');
});

test('cacheKey(): normalises a missing leading slash', () => {
  assert.equal(cacheKey('analyze'), 'GET /analyze');
  assert.equal(cacheKey('/analyze'), 'GET /analyze');
});

/* ── Source contract ─────────────────────────────────────────────────── */

test('apiCache.ts still exports the cache surface mirrored above', () => {
  const src = fs.readFileSync(path.join(__dirname, 'apiCache.ts'), 'utf8');

  assert.match(src, /export const DEFAULT_TTL_MS = 5 \* 60 \* 1000/, 'default TTL is 5 minutes');
  assert.match(src, /export const DEFAULT_MAX_ENTRIES = 100/, 'default cap is 100 entries');
  assert.match(src, /export class TtlLruCache/, 'bounded TTL + LRU cache must exist');
  assert.match(src, /export const apiCache = new TtlLruCache\(\)/, 'shared cache instance');
  assert.match(src, /export function clearCache\(\)/, 'manual invalidation hook');
  assert.match(src, /export function cacheKey\(/, 'canonical key helper');

  // cachedGet must actually use the in-memory cache.
  assert.match(src, /apiCache\.get\(key\)/, 'cachedGet reads the in-memory cache');
  assert.match(src, /apiCache\.set\(key, data, ttlMs\)/, 'cachedGet populates the in-memory cache');

  // Existing public API stays intact for current callers.
  assert.match(src, /export async function cachedGet/, 'cachedGet preserved');
  assert.match(src, /export async function uncachedPost/, 'uncachedPost preserved');
  assert.match(src, /export const CACHE_TAGS/, 'CACHE_TAGS preserved');
  assert.match(src, /revalidate\?: number/, 'revalidate option preserved');
});
