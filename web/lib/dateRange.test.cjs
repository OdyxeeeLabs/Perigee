'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const {
  DEFAULT_DATE_RANGE,
  DATE_RANGE_PARAM,
  DATE_RANGE_PRESETS,
  normalizeDateRange,
  parseDateRange,
  parseDateRangeFromQuery,
  getDateRangePreset,
  dateRangeBounds,
  dateRangeToApiParams,
  dateRangeToQuery,
} = require('./dateRange');

function read(relative) {
  return fs.readFileSync(path.join(__dirname, relative), 'utf8');
}

test('supported presets are a small, stable set with the safe default included', () => {
  assert.deepEqual(
    DATE_RANGE_PRESETS.map((preset) => preset.id),
    ['24h', '7d', '30d', 'all'],
  );
  assert.equal(DEFAULT_DATE_RANGE, '7d');
  assert.equal(DATE_RANGE_PARAM, 'range');
  assert.equal(getDateRangePreset('30d').days, 30);
  assert.equal(getDateRangePreset('all').days, null);
});

test('valid URL ranges are accepted case-insensitively', () => {
  for (const id of ['24h', '7d', '30d', 'all']) {
    assert.equal(parseDateRange(id), id);
    assert.equal(parseDateRange(`  ${id.toUpperCase()} `), id);
  }
});

test('invalid URL ranges fall back to the default without throwing', () => {
  for (const bad of ['', '   ', 'bogus', '90d', '7d;drop', '../etc', 'undefined']) {
    assert.equal(parseDateRange(bad), DEFAULT_DATE_RANGE);
  }
  for (const bad of [null, undefined, 0, 7, {}, true, Symbol('x')]) {
    assert.equal(parseDateRange(bad), DEFAULT_DATE_RANGE);
  }
});

test('query parsing handles arrays, missing keys and hostile values', () => {
  assert.equal(parseDateRangeFromQuery({ range: '30d' }), '30d');
  assert.equal(parseDateRangeFromQuery({ range: ['7d', '30d'] }), '7d');
  assert.equal(parseDateRangeFromQuery({ range: ['bogus', '30d'] }), DEFAULT_DATE_RANGE);
  assert.equal(parseDateRangeFromQuery({}), DEFAULT_DATE_RANGE);
  assert.equal(parseDateRangeFromQuery(null), DEFAULT_DATE_RANGE);
  assert.equal(parseDateRangeFromQuery('range=7d'), DEFAULT_DATE_RANGE);
});

test('range bounds are deterministic for a supplied clock', () => {
  const now = new Date('2026-01-15T10:30:00Z');
  const week = dateRangeBounds('7d', now);
  assert.equal(week.to.toISOString(), '2026-01-15T10:30:00.000Z');
  assert.equal(week.from.toISOString(), '2026-01-08T10:30:00.000Z');

  const day = dateRangeBounds('24h', now);
  assert.equal(day.from.toISOString(), '2026-01-14T10:30:00.000Z');

  const all = dateRangeBounds('all', now);
  assert.equal(all.from, null);
  assert.equal(all.to.toISOString(), now.toISOString());
});

test('API parameter mapping uses the timeframe the collector understands', () => {
  assert.deepEqual(dateRangeToApiParams('30d'), { timeframe: '30d' });
  assert.deepEqual(dateRangeToApiParams('not-a-range'), { timeframe: DEFAULT_DATE_RANGE });
});

test('URL updates preserve unrelated query parameters', () => {
  assert.deepEqual(dateRangeToQuery('30d', { tab: 'events', range: '7d' }), {
    tab: 'events',
    range: '30d',
  });
  assert.deepEqual(dateRangeToQuery('bogus'), { range: DEFAULT_DATE_RANGE });
});

test('browser back/forward is handled by the shared hook', () => {
  const hook = read('../hooks/useDateRange.ts');
  assert.match(hook, /popstate/);
  assert.match(hook, /routeChangeComplete/);
  assert.match(hook, /parseDateRange\(/);
});

test('analytics maps the selected range to the telemetry request', () => {
  const page = read('../pages/analytics/index.tsx');
  assert.match(page, /useDateRange/);
  assert.match(page, /new URLSearchParams\(dateRangeToApiParams\(timeframe\)\)/);
  assert.match(page, /aria-pressed=\{timeframe === preset\.id\}/);
  assert.match(page, /data-testid="range-label"/);
});

test('portfolio exposes a shareable comparison control', () => {
  const page = read('../pages/portfolio.tsx');
  assert.match(page, /useDateRange/);
  assert.match(page, /data-testid="comparison-range"/);
  assert.match(page, /value=\{range\}/);
  assert.match(page, /setRange\(event\.target\.value as DateRangeId\)/);
});

test('date labels go through the shared locale formatters', () => {
  const hook = read('../hooks/useDateRange.ts');
  assert.match(hook, /from "@\/lib\/locale\/format"/);
  assert.match(hook, /formatDate\(from, locale\)/);
  const page = read('../pages/portfolio.tsx');
  assert.match(page, /formatNumber\(/);
});

test('utils re-exports the range helpers for existing import paths', () => {
  const utils = read('./utils.ts');
  assert.match(utils, /from "\.\/dateRange"/);
  assert.match(utils, /parseDateRangeFromQuery/);
});
