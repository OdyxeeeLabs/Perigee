/* Perigee service worker.
 *
 * Cache policy (issue #834):
 *   - only an explicit allowlist of versioned static assets may be cached
 *   - application documents are NEVER cached, so a stale shell cannot be served
 *   - API GET reads use stale-while-revalidate with a bounded max age and are
 *     stamped with the time they were cached
 *   - API mutations/signing requests are network-only and are NEVER served from
 *     a cache; offline signing fails closed with a reconnect message
 *   - a cache version bump deletes every older cache on activate
 */

const VERSION = "v2";
const CACHE_PREFIX = "perigee-";
const STATIC_CACHE = `${CACHE_PREFIX}static-${VERSION}`;
const IMAGE_CACHE = `${CACHE_PREFIX}images-${VERSION}`;
const API_CACHE = `${CACHE_PREFIX}api-${VERSION}`;
const ALL_CACHES = [STATIC_CACHE, IMAGE_CACHE, API_CACHE];

const API_MAX_AGE_MS = 5 * 60 * 1000;
const IMAGE_MAX_AGE_MS = 30 * 24 * 60 * 60 * 1000;
const IMAGE_MAX_ENTRIES = 60;
const CACHED_AT_HEADER = "x-sw-cached-at";

const CACHEABLE_STATIC_PATHS = ["/_next/static/", "/icons/", "/favicon.ico", "/manifest.webmanifest"];
const CACHEABLE_STATIC_EXT = /\.(?:js|css|woff2?|ttf|otf|wasm|svg|png|webp|ico)$/i;
const SIGNING_PATH_RE = /\/api\/(?:sign|signing|transaction|tx|deposit|withdraw|approve|reject|register)(?:\/|$)/i;
const MUTATION_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);
const OFFLINE_ACTION_MESSAGE =
  "You are offline. Reconnect and try again — this action was not submitted.";

const SYNC_TAG = "perigee-replay-mutations";
const DB_NAME = "perigee-sw";
const STORE = "mutations";

self.addEventListener("install", () => {
  // Intentionally no skipWaiting(): wait for the user to accept the update.
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const names = await caches.keys();
      await Promise.all(
        names
          .filter((name) => name.startsWith(CACHE_PREFIX) && !ALL_CACHES.includes(name))
          .map((name) => caches.delete(name)),
      );
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") {
    self.skipWaiting();
  }
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isApi(url) {
  return url.origin === self.location.origin && url.pathname.startsWith("/api/");
}

function isSigningRequest(url) {
  return isApi(url) && SIGNING_PATH_RE.test(url.pathname);
}

function isMutation(request) {
  return MUTATION_METHODS.has(request.method.toUpperCase());
}

/** Only the explicit static allowlist is cacheable; documents never are. */
function isCacheableStatic(url, request) {
  if (url.origin !== self.location.origin || url.pathname.startsWith("/api/")) return false;
  if (request.destination === "document") return false;
  const allowedPath = CACHEABLE_STATIC_PATHS.some((prefix) => url.pathname.startsWith(prefix));
  return allowedPath || CACHEABLE_STATIC_EXT.test(url.pathname);
}

/** Clone a response and stamp it with the time it was cached. */
async function stamp(response) {
  const headers = new Headers(response.headers);
  headers.set(CACHED_AT_HEADER, String(Date.now()));
  return new Response(await response.clone().blob(), {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

function ageOf(response) {
  const cachedAt = Number(response.headers.get(CACHED_AT_HEADER));
  return Number.isFinite(cachedAt) && cachedAt > 0 ? Date.now() - cachedAt : Infinity;
}

function offlineResponse(message, status) {
  return new Response(
    JSON.stringify({ error: "offline", submitted: false, message }),
    {
      status,
      headers: { "Content-Type": "application/json", "Cache-Control": "no-store" },
    },
  );
}

async function cacheFirst(request, cacheName, maxAgeMs) {
  const cache = await caches.open(cacheName);
  const cached = await cache.match(request);
  if (cached && (maxAgeMs === undefined || ageOf(cached) <= maxAgeMs)) {
    return cached;
  }
  try {
    const response = await fetch(request);
    if (response.ok || response.type === "opaque") {
      await cache.put(request, await stamp(response));
      if (cacheName === IMAGE_CACHE) await trimImageCache(cache);
    }
    return response;
  } catch (error) {
    if (cached) return cached;
    throw error;
  }
}

async function trimImageCache(cache) {
  const keys = await cache.keys();
  const overflow = keys.length - IMAGE_MAX_ENTRIES;
  for (let i = 0; i < overflow; i += 1) {
    await cache.delete(keys[i]);
  }
}

async function staleWhileRevalidate(event, request, url) {
  if (isSigningRequest(url)) {
    return fetch(request, { cache: "no-store" });
  }

  const cache = await caches.open(API_CACHE);
  const cached = await cache.match(request);

  const refresh = fetch(request, { cache: "no-store" })
    .then(async (response) => {
      if (response.ok) await cache.put(request, await stamp(response));
      return response;
    })
    .catch(() => undefined);

  if (cached && ageOf(cached) <= API_MAX_AGE_MS) {
    event.waitUntil(refresh);
    return cached;
  }

  const fresh = await refresh;
  if (fresh) return fresh;
  if (cached) return cached;
  return offlineResponse("You are offline. Reconnect to load fresh data.", 503);
}

// ---------------------------------------------------------------------------
// Fetch routing
// ---------------------------------------------------------------------------

self.addEventListener("fetch", (event) => {
  const { request } = event;
  const url = new URL(request.url);

  if (isMutation(request)) {
    if (isApi(url)) event.respondWith(networkOrQueue(request, url));
    return;
  }

  if (request.method !== "GET") return;

  // Application documents are network-only: a stale cached shell must never
  // make a user believe a transaction completed while offline.
  if (request.destination === "document") return;

  if (request.destination === "image") {
    event.respondWith(cacheFirst(request, IMAGE_CACHE, IMAGE_MAX_AGE_MS));
  } else if (isCacheableStatic(url, request)) {
    event.respondWith(cacheFirst(request, STATIC_CACHE));
  } else if (isApi(url)) {
    event.respondWith(staleWhileRevalidate(event, request, url));
  }
});

// ---------------------------------------------------------------------------
// Background sync for failed, retry-safe mutations (never signing requests)
// ---------------------------------------------------------------------------

function openDb() {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB_NAME, 1);
    open.onupgradeneeded = () => {
      open.result.createObjectStore(STORE, { keyPath: "id", autoIncrement: true });
    };
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error);
  });
}

function withStore(mode, run) {
  return openDb().then(
    (db) =>
      new Promise((resolve, reject) => {
        const tx = db.transaction(STORE, mode);
        const result = run(tx.objectStore(STORE));
        tx.oncomplete = () => resolve(result && result.result);
        tx.onerror = () => reject(tx.error);
      }),
  );
}

async function networkOrQueue(request, url) {
  const replayable = request.clone();
  try {
    return await fetch(request, { cache: "no-store" });
  } catch (error) {
    if (isSigningRequest(url)) {
      await notifyClients({ type: "OFFLINE_ACTION_BLOCKED", message: OFFLINE_ACTION_MESSAGE });
      return offlineResponse(OFFLINE_ACTION_MESSAGE, 503);
    }

    const record = {
      url: replayable.url,
      method: replayable.method,
      headers: Array.from(replayable.headers.entries()),
      body: await replayable.text(),
      queuedAt: Date.now(),
    };
    await withStore("readwrite", (store) => store.add(record));
    if (self.registration.sync) {
      await self.registration.sync.register(SYNC_TAG);
    }
    await notifyClients({ type: "MUTATION_QUEUED" });
    return offlineResponse(
      "You are offline. Your change was queued and has NOT been submitted yet.",
      202,
    );
  }
}

async function replayQueuedMutations() {
  const queued = await withStore("readonly", (store) => store.getAll());
  let replayed = 0;
  for (const record of queued || []) {
    const url = new URL(record.url);
    if (isSigningRequest(url)) {
      await withStore("readwrite", (store) => store.delete(record.id));
      continue;
    }
    try {
      const response = await fetch(record.url, {
        method: record.method,
        headers: record.headers,
        body: record.body || undefined,
        cache: "no-store",
      });
      if (response.ok || (response.status >= 400 && response.status < 500)) {
        await withStore("readwrite", (store) => store.delete(record.id));
        replayed += 1;
      } else {
        throw new Error(`replay failed with ${response.status}`);
      }
    } catch (error) {
      throw error;
    }
  }
  if (replayed > 0) await notifyClients({ type: "MUTATIONS_REPLAYED", count: replayed });
}

self.addEventListener("sync", (event) => {
  if (event.tag === SYNC_TAG) event.waitUntil(replayQueuedMutations());
});

async function notifyClients(message) {
  const clients = await self.clients.matchAll({ includeUncontrolled: true });
  clients.forEach((client) => client.postMessage(message));
}
