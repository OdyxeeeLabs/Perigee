// vault-detail.test.cjs — unit tests for web/lib/vault-detail.ts (#684)
//
// Mirrors the pure seeded-random/mock-generation logic (same approach as
// __tests__/config.test.cjs and lib/settings.test.cjs) since this suite runs
// directly under `node --test` without a TypeScript build step.
//
// Runs with: node --test ./lib/vault-detail.test.cjs

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

function hashSeed(seed) {
  let hash = 5381;
  for (let i = 0; i < seed.length; i++) {
    hash = (hash * 33) ^ seed.charCodeAt(i);
  }
  return hash >>> 0;
}

function mulberry32(seed) {
  let a = seed;
  return function random() {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function buildMockVaultHistory(seed, points = 30) {
  const random = mulberry32(hashSeed(seed));
  const history = [];
  let apy = 4 + random() * 8;
  let tvl = 200_000 + random() * 800_000;
  const now = Math.floor(Date.now() / 86_400_000) * 86_400_000;
  const dayMs = 24 * 60 * 60 * 1000;

  for (let i = points - 1; i >= 0; i--) {
    apy = Math.max(0, apy + (random() - 0.5) * 1.5);
    tvl = Math.max(0, tvl * (1 + (random() - 0.48) * 0.04));
    history.push({
      timestamp: new Date(now - i * dayMs).toISOString(),
      apy: Number(apy.toFixed(2)),
      tvl: Math.round(tvl),
    });
  }

  return history;
}

// ── hashSeed ──────────────────────────────────────────────────────────────

test('hashSeed: deterministic for the same input', () => {
  assert.equal(hashSeed('vault-1'), hashSeed('vault-1'));
});

test('hashSeed: different inputs (usually) produce different seeds', () => {
  assert.notEqual(hashSeed('vault-1'), hashSeed('vault-2'));
});

test('hashSeed: always returns a non-negative 32-bit integer', () => {
  for (const seed of ['', 'a', 'vault-123', 'C'.repeat(56)]) {
    const h = hashSeed(seed);
    assert.ok(h >= 0 && h <= 0xffffffff);
  }
});

// ── mulberry32 ────────────────────────────────────────────────────────────

test('mulberry32: same seed produces the same sequence', () => {
  const a = mulberry32(42);
  const b = mulberry32(42);
  assert.equal(a(), b());
  assert.equal(a(), b());
  assert.equal(a(), b());
});

test('mulberry32: values stay within [0, 1)', () => {
  const random = mulberry32(hashSeed('vault-7'));
  for (let i = 0; i < 50; i++) {
    const value = random();
    assert.ok(value >= 0 && value < 1);
  }
});

// ── buildMockVaultHistory ─────────────────────────────────────────────────

test('buildMockVaultHistory: same vault id yields identical history every call', () => {
  const a = buildMockVaultHistory('vault-42');
  const b = buildMockVaultHistory('vault-42');
  assert.deepEqual(a, b);
});

test('buildMockVaultHistory: different vault ids yield different history', () => {
  const a = buildMockVaultHistory('vault-1');
  const b = buildMockVaultHistory('vault-2');
  assert.notDeepEqual(a, b);
});

test('buildMockVaultHistory: respects the requested point count and is chronological', () => {
  const history = buildMockVaultHistory('vault-9', 10);
  assert.equal(history.length, 10);
  for (let i = 1; i < history.length; i++) {
    assert.ok(new Date(history[i].timestamp) > new Date(history[i - 1].timestamp));
  }
});

test('buildMockVaultHistory: apy and tvl are never negative', () => {
  const history = buildMockVaultHistory('vault-stress', 60);
  for (const point of history) {
    assert.ok(point.apy >= 0);
    assert.ok(point.tvl >= 0);
  }
});

// ── source guard: keep this mirror honest against lib/vault-detail.ts ──────

test('lib/vault-detail.ts implements the same seeded PRNG this suite mirrors', () => {
  const src = fs.readFileSync(path.join(__dirname, 'vault-detail.ts'), 'utf8');
  assert.ok(src.includes('function hashSeed'));
  assert.ok(src.includes('function mulberry32'));
  assert.ok(src.includes('0x6d2b79f5'));
  assert.ok(src.includes('86_400_000'), 'history must anchor to the start of the day for determinism');
});
