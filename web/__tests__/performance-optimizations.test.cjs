'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const WEB_ROOT = path.join(__dirname, '..');
const read = (...parts) => fs.readFileSync(path.join(WEB_ROOT, ...parts), 'utf8');

test('service worker implements the documented cache strategies (#688)', () => {
  const sw = read('public', 'sw.js');
  assert.match(sw, /cacheFirst\(request, IMAGE_CACHE, IMAGE_MAX_AGE_MS\)/);
  assert.match(sw, /staleWhileRevalidate\(event, request\)/);
  assert.match(sw, /API_MAX_AGE_MS = 5 \* 60 \* 1000/);
  assert.match(sw, /addEventListener\("sync"/);
  assert.match(sw, /SKIP_WAITING/);
});

test('next.config enables modern image formats and IPFS patterns (#689)', () => {
  const cfg = read('next.config.js');
  assert.match(cfg, /image\/avif/);
  assert.match(cfg, /image\/webp/);
  assert.match(cfg, /deviceSizes/);
  assert.match(cfg, /ipfs\.io/);
});

test('wallet icons use a blur placeholder (#689)', () => {
  const src = read('components', 'WalletModal.tsx');
  assert.match(src, /placeholder="blur"/);
  assert.match(src, /blurDataURL=/);
});

test('request deduplication defaults to a 2000ms interval (#690)', () => {
  const src = read('lib', 'requestDedupe.ts');
  assert.match(src, /DEFAULT_DEDUPING_INTERVAL_MS = 2000/);
  const cache = read('lib', 'apiCache.ts');
  assert.match(cache, /dedupedFetch/);
});

test('hover prefetch waits 200ms and respects data-saver signals (#691)', () => {
  assert.match(read('lib', 'prefetchPolicy.ts'), /HOVER_PREFETCH_DELAY_MS = 200/);
  assert.match(read('lib', 'prefetchPolicy.ts'), /saveData/);
  assert.match(read('lib', 'prefetchPolicy.ts'), /prefers-reduced-data/);
  assert.match(read('hooks', 'usePrefetchOnHover.ts'), /routeChangeStart/);
});
