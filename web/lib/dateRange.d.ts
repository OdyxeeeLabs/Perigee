export type DateRangeId = '24h' | '7d' | '30d' | 'all';

export interface DateRangePreset {
  id: DateRangeId;
  label: string;
  days: number | null;
}

export interface DateRangeBounds {
  from: Date | null;
  to: Date;
}

export interface DateRangeApiParams {
  timeframe: DateRangeId;
  [key: string]: string;
}

export const DEFAULT_DATE_RANGE: DateRangeId;
export const DATE_RANGE_PARAM: string;
export const DATE_RANGE_PRESETS: readonly DateRangePreset[];
export const DATE_RANGE_IDS: readonly DateRangeId[];

export function normalizeDateRange(value: unknown): DateRangeId;
export function parseDateRange(value: unknown): DateRangeId;
export function parseDateRangeFromQuery(query: unknown): DateRangeId;
export function getDateRangePreset(range: unknown): DateRangePreset;
export function dateRangeBounds(range: unknown, now?: Date): DateRangeBounds;
export function dateRangeToApiParams(range: unknown): DateRangeApiParams;
export function dateRangeToQuery(
  range: unknown,
  query?: Record<string, string | string[] | undefined> | null,
): Record<string, string | string[]>;
