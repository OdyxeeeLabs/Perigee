/**
 * Locale-aware formatting built on `Intl` — issue #700.
 *
 * Currency, date and number rendering are the three places where a hardcoded
 * `en-GB` format produces obviously wrong output: a German price group and
 * comma decimals, a Japanese era date, a Hindi lakh/crore grouping. These
 * helpers take the locale from one place so a switcher cannot change the
 * numbers but not the dates.
 *
 * Every helper is total: an invalid date, amount or locale produces
 * something readable rather than "Invalid Date" or `NaN`.
 */

import { intlTag, normalizeLocale, type Locale } from "./locales";

/** `1,234.5` in en-GB, `1.234,5` in de-DE, `1 234,5` in fr-FR. */
export function formatNumber(
  value: number,
  locale: Locale | string = "en",
  options: Intl.NumberFormatOptions = {},
): string {
  const tag = intlTag(normalizeLocale(locale));
  if (!Number.isFinite(value)) return "—";

  try {
    return new Intl.NumberFormat(tag, options).format(value);
  } catch {
    return String(value);
  }
}

/** Compact form for dashboard tiles: 1.2K, 3.4M. */
export function formatCompactNumber(value: number, locale: Locale | string = "en"): string {
  return formatNumber(value, locale, { notation: "compact", maximumFractionDigits: 1 });
}

/** Balance / amount with a bounded number of fraction digits. */
export function formatAmount(
  value: number,
  locale: Locale | string = "en",
  options: Intl.NumberFormatOptions = {},
): string {
  return formatNumber(value, locale, { maximumFractionDigits: 2, ...options });
}

/** Balance plus its asset code, e.g. `1.234,56 XLM` in German. */
export function formatAssetAmount(
  value: number,
  asset: string,
  locale: Locale | string = "en",
  options: Intl.NumberFormatOptions = {},
): string {
  const amount = formatAmount(value, locale, options);
  return asset ? `${amount} ${asset}` : amount;
}

/**
 * The unrounded value, for the assistive-technology counterpart of a rounded
 * visual amount. Formatting is still locale-aware; only the precision is not
 * capped. Protocol values are never rounded before calculations or submits.
 */
export function formatRawNumber(value: number, locale: Locale | string = "en"): string {
  return formatNumber(value, locale, { maximumFractionDigits: 20 });
}

/**
 * Percentage with a fixed number of fraction digits, because "12.3456%" in a
 * stat tile is a bug report waiting to happen. `value` is in percentage
 * points: pass 12.3 for 12.3%, not 0.123.
 */
export function formatPercent(value: number, locale: Locale | string = "en", digits = 1): string {
  return formatNumber(value / 100, locale, {
    style: "percent",
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

/** Signed change, with an explicit "+" for gains and the locale's minus for losses. */
export function formatSignedPercent(
  value: number,
  locale: Locale | string = "en",
  digits = 2,
): string {
  const formatted = formatPercent(value, locale, digits);
  return value > 0 ? `+${formatted}` : formatted;
}

/**
 * Currency. The app's transactions are denominated in XLM and USDC, so the
 * currency code is a parameter rather than a locale default.
 */
export function formatCurrency(
  value: number,
  currency: string,
  locale: Locale | string = "en",
  options: Intl.NumberFormatOptions = {},
): string {
  const tag = intlTag(normalizeLocale(locale));
  if (!Number.isFinite(value)) return "—";

  try {
    return new Intl.NumberFormat(tag, {
      style: "currency",
      currency,
      ...options,
    }).format(value);
  } catch {
    // An unknown currency code throws a RangeError; the amount is still
    // better information than an exception in a render path.
    return `${formatNumber(value, locale)} ${currency}`;
  }
}

export function formatDate(
  value: Date | string | number,
  locale: Locale | string = "en",
  options: Intl.DateTimeFormatOptions = { dateStyle: "medium" },
): string {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) return "—";

  try {
    return new Intl.DateTimeFormat(intlTag(normalizeLocale(locale)), options).format(date);
  } catch {
    return date.toISOString().slice(0, 10);
  }
}

export function formatDateTime(
  value: Date | string | number,
  locale: Locale | string = "en",
): string {
  return formatDate(value, locale, { dateStyle: "medium", timeStyle: "short" });
}

/** Time-only rendering for event streams and recent activity rows. */
export function formatTime(
  value: Date | string | number,
  locale: Locale | string = "en",
): string {
  return formatDate(value, locale, { timeStyle: "short" });
}

/** Relative time ("3 minutes ago") with an absolute fallback. */
export function formatRelativeTime(
  value: Date | string | number,
  locale: Locale | string = "en",
  now: Date = new Date(),
): string {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) return "—";

  const deltaSeconds = Math.round((date.getTime() - now.getTime()) / 1000);
  const absolute = Math.abs(deltaSeconds);

  try {
    const rtf = new Intl.RelativeTimeFormat(intlTag(normalizeLocale(locale)), {
      numeric: "auto",
    });
    if (absolute < 60) return rtf.format(Math.round(deltaSeconds), "second");
    if (absolute < 3600) return rtf.format(Math.round(deltaSeconds / 60), "minute");
    if (absolute < 86_400) return rtf.format(Math.round(deltaSeconds / 3600), "hour");
    return rtf.format(Math.round(deltaSeconds / 86_400), "day");
  } catch {
    return formatDate(date, locale);
  }
}

/** The `Intl.PluralRules` category for a count in a locale. */
export function pluralCategory(
  count: number,
  locale: Locale | string = "en",
): string {
  const normalized = normalizeLocale(locale);
  try {
    return new Intl.PluralRules(intlTag(normalized)).select(count);
  } catch {
    return normalized === "zh" ? "other" : count === 1 ? "one" : "other";
  }
}

/**
 * Pick the right plural form.
 *
 * `forms` may be a flat string ("{count} vault") or a map keyed by plural
 * category. A category the locale does not use falls back to `other`, which
 * every CLDR locale defines, so a missing translation is never silent.
 *
 * The fallback chain deliberately stops at `other`: falling back to the
 * singular form would render "1 vault" for three vaults.
 */
export function pluralize(
  count: number,
  forms: Record<string, string> | string,
  locale: Locale | string = "en",
): string {
  if (typeof forms === "string") {
    return forms.replace("{count}", String(count));
  }

  const normalized = normalizeLocale(locale);
  const category = pluralCategory(count, normalized);
  // `other` is the only safe fallback: falling back to the singular form
  // would read as "1 vault" for 3.
  return (forms[category] ?? forms.other ?? '{count}').replace(
    /\{count\}/g,
    String(count),
  );
}

