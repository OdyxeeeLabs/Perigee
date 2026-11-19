'use strict';

const DEFAULT_DATE_RANGE = '7d';
const DATE_RANGE_PARAM = 'range';
const DATE_RANGE_PRESETS = Object.freeze([
  Object.freeze({ id: '24h', label: 'Last 24 hours', days: 1 }),
  Object.freeze({ id: '7d', label: 'Last 7 days', days: 7 }),
  Object.freeze({ id: '30d', label: 'Last 30 days', days: 30 }),
  Object.freeze({ id: 'all', label: 'All time', days: null }),
]);

const DATE_RANGE_IDS = DATE_RANGE_PRESETS.map((preset) => preset.id);
const DATE_RANGE_ID_SET = new Set(DATE_RANGE_IDS);
const DAY_MS = 24 * 60 * 60 * 1000;

function firstString(value) {
  if (Array.isArray(value)) return firstString(value[0]);
  return typeof value === 'string' ? value : '';
}

function normalizeDateRange(value) {
  const candidate = firstString(value).trim().toLowerCase();
  return DATE_RANGE_ID_SET.has(candidate) ? candidate : DEFAULT_DATE_RANGE;
}

function parseDateRange(value) {
  return normalizeDateRange(value);
}

function parseDateRangeFromQuery(query) {
  if (!query || typeof query !== 'object') return DEFAULT_DATE_RANGE;
  return normalizeDateRange(query[DATE_RANGE_PARAM]);
}

function getDateRangePreset(range) {
  const id = normalizeDateRange(range);
  return DATE_RANGE_PRESETS.find((preset) => preset.id === id) || DATE_RANGE_PRESETS[1];
}

function dateRangeBounds(range, now) {
  const id = normalizeDateRange(range);
  const to = now instanceof Date ? new Date(now.getTime()) : new Date();
  if (id === 'all' || Number.isNaN(to.getTime())) return { from: null, to };
  const { days } = getDateRangePreset(id);
  return { from: new Date(to.getTime() - days * DAY_MS), to };
}

function dateRangeToApiParams(range) {
  return { timeframe: normalizeDateRange(range) };
}

function dateRangeToQuery(range, query) {
  const base =
    query && typeof query === 'object' && !Array.isArray(query) ? query : {};
  return { ...base, [DATE_RANGE_PARAM]: normalizeDateRange(range) };
}

module.exports = {
  DEFAULT_DATE_RANGE,
  DATE_RANGE_PARAM,
  DATE_RANGE_PRESETS,
  DATE_RANGE_IDS,
  normalizeDateRange,
  parseDateRange,
  parseDateRangeFromQuery,
  getDateRangePreset,
  dateRangeBounds,
  dateRangeToApiParams,
  dateRangeToQuery,
};
