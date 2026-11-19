// settings.test.cjs — unit tests for web/lib/settings.ts (#682)
//
// Mirrors the pure logic from lib/settings.ts (same approach as
// __tests__/config.test.cjs) since this test suite runs directly under
// `node --test` without a TypeScript build step. A source guard at the
// bottom keeps the mirrored constants honest against the real file.
//
// Runs with: node --test ./lib/settings.test.cjs

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const MIN_SLIPPAGE = 0.1;
const MAX_SLIPPAGE = 10;

const DEFAULT_SETTINGS = {
  network: { preferredNetwork: 'testnet', customRpcEndpoint: '', chainId: '' },
  trading: {
    slippageTolerance: 0.5,
    gasPriceStrategy: 'standard',
    customGasPrice: 100,
    transactionDeadlineMinutes: 20,
  },
  display: { currency: 'USD', numberFormat: '1,234.56', preferredLanguage: 'en' },
  security: { twoFactorEnabled: false, sessionTimeoutMinutes: 30 },
  advanced: { debugMode: false, featureFlagOverrides: {} },
};

function clampSlippage(value) {
  if (Number.isNaN(value)) return DEFAULT_SETTINGS.trading.slippageTolerance;
  return Math.min(MAX_SLIPPAGE, Math.max(MIN_SLIPPAGE, value));
}

function clampDeadlineMinutes(value) {
  if (Number.isNaN(value)) return DEFAULT_SETTINGS.trading.transactionDeadlineMinutes;
  return Math.min(180, Math.max(1, Math.round(value)));
}

function clampGasPrice(value) {
  if (Number.isNaN(value) || value < 0) return DEFAULT_SETTINGS.trading.customGasPrice;
  return Math.round(value);
}

function mergeWithDefaults(partial) {
  const p = partial && typeof partial === 'object' ? partial : {};
  return {
    network: { ...DEFAULT_SETTINGS.network, ...(p.network ?? {}) },
    trading: {
      ...DEFAULT_SETTINGS.trading,
      ...(p.trading ?? {}),
      slippageTolerance: clampSlippage(
        Number(p.trading?.slippageTolerance ?? DEFAULT_SETTINGS.trading.slippageTolerance),
      ),
      transactionDeadlineMinutes: clampDeadlineMinutes(
        Number(
          p.trading?.transactionDeadlineMinutes ??
            DEFAULT_SETTINGS.trading.transactionDeadlineMinutes,
        ),
      ),
      customGasPrice: clampGasPrice(
        Number(p.trading?.customGasPrice ?? DEFAULT_SETTINGS.trading.customGasPrice),
      ),
    },
    display: { ...DEFAULT_SETTINGS.display, ...(p.display ?? {}) },
    security: { ...DEFAULT_SETTINGS.security, ...(p.security ?? {}) },
    advanced: {
      ...DEFAULT_SETTINGS.advanced,
      ...(p.advanced ?? {}),
      featureFlagOverrides: {
        ...DEFAULT_SETTINGS.advanced.featureFlagOverrides,
        ...(p.advanced?.featureFlagOverrides ?? {}),
      },
    },
  };
}

function parseStoredSettings(raw) {
  if (!raw) return DEFAULT_SETTINGS;
  try {
    return mergeWithDefaults(JSON.parse(raw));
  } catch {
    return DEFAULT_SETTINGS;
  }
}

// ── clampSlippage ────────────────────────────────────────────────────────────

test('clampSlippage: within range is unchanged', () => {
  assert.equal(clampSlippage(2.5), 2.5);
});

test('clampSlippage: below minimum clamps to 0.1', () => {
  assert.equal(clampSlippage(0), 0.1);
  assert.equal(clampSlippage(-5), 0.1);
});

test('clampSlippage: above maximum clamps to 10', () => {
  assert.equal(clampSlippage(50), 10);
});

test('clampSlippage: NaN falls back to the default', () => {
  assert.equal(clampSlippage(NaN), DEFAULT_SETTINGS.trading.slippageTolerance);
});

// ── clampDeadlineMinutes ──────────────────────────────────────────────────────

test('clampDeadlineMinutes: rounds and clamps to [1, 180]', () => {
  assert.equal(clampDeadlineMinutes(20.6), 21);
  assert.equal(clampDeadlineMinutes(0), 1);
  assert.equal(clampDeadlineMinutes(999), 180);
});

// ── clampGasPrice ─────────────────────────────────────────────────────────────

test('clampGasPrice: negative or NaN falls back to the default', () => {
  assert.equal(clampGasPrice(-1), DEFAULT_SETTINGS.trading.customGasPrice);
  assert.equal(clampGasPrice(NaN), DEFAULT_SETTINGS.trading.customGasPrice);
});

test('clampGasPrice: rounds valid input', () => {
  assert.equal(clampGasPrice(150.7), 151);
});

// ── mergeWithDefaults / parseStoredSettings ──────────────────────────────────

test('mergeWithDefaults: empty input yields exactly the defaults', () => {
  assert.deepEqual(mergeWithDefaults({}), DEFAULT_SETTINGS);
  assert.deepEqual(mergeWithDefaults(null), DEFAULT_SETTINGS);
  assert.deepEqual(mergeWithDefaults(undefined), DEFAULT_SETTINGS);
});

test('mergeWithDefaults: partial input is layered onto defaults, not replacing siblings', () => {
  const merged = mergeWithDefaults({ trading: { slippageTolerance: 2 } });
  assert.equal(merged.trading.slippageTolerance, 2);
  assert.equal(merged.trading.gasPriceStrategy, 'standard');
  assert.deepEqual(merged.network, DEFAULT_SETTINGS.network);
});

test('mergeWithDefaults: out-of-range values from a corrupted blob are clamped back in', () => {
  const merged = mergeWithDefaults({ trading: { slippageTolerance: 999, customGasPrice: -5 } });
  assert.equal(merged.trading.slippageTolerance, 10);
  assert.equal(merged.trading.customGasPrice, DEFAULT_SETTINGS.trading.customGasPrice);
});

test('mergeWithDefaults: featureFlagOverrides merges rather than overwrites', () => {
  const merged = mergeWithDefaults({ advanced: { featureFlagOverrides: { newVaultUI: true } } });
  assert.deepEqual(merged.advanced.featureFlagOverrides, { newVaultUI: true });
});

test('parseStoredSettings: null raw value returns defaults', () => {
  assert.deepEqual(parseStoredSettings(null), DEFAULT_SETTINGS);
});

test('parseStoredSettings: malformed JSON never throws, returns defaults', () => {
  assert.deepEqual(parseStoredSettings('{not valid json'), DEFAULT_SETTINGS);
});

test('parseStoredSettings: valid JSON round-trips through merge', () => {
  const stored = JSON.stringify({ display: { currency: 'EUR' } });
  const parsed = parseStoredSettings(stored);
  assert.equal(parsed.display.currency, 'EUR');
  assert.equal(parsed.display.numberFormat, DEFAULT_SETTINGS.display.numberFormat);
});

// ── source guard: keep this mirror honest against lib/settings.ts ───────────

test('lib/settings.ts declares the same default values this suite mirrors', () => {
  const src = fs.readFileSync(path.join(__dirname, 'settings.ts'), 'utf8');
  assert.ok(src.includes('MIN_SLIPPAGE = 0.1'));
  assert.ok(src.includes('MAX_SLIPPAGE = 10'));
  assert.ok(src.includes('slippageTolerance: 0.5'));
  assert.ok(src.includes('transactionDeadlineMinutes: 20'));
  assert.ok(src.includes('customGasPrice: 100'));
  assert.ok(src.includes('sessionTimeoutMinutes: 30'));
});
