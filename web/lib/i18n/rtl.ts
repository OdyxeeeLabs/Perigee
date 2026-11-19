/**
 * Right-to-left language support — issue #699.
 *
 * Layout mirroring is a CSS concern (logical properties in `styles/rtl.css`),
 * but the *document* direction, the locale list and the small amount of
 * JavaScript that has to reason about direction are not. Keeping that logic
 * here means it can be unit tested, and means the locale table has exactly
 * one definition that both the layout and the language switcher read.
 */

export const RTL_LOCALES = ["ar", "he", "fa", "ur"] as const;

export type Locale = (typeof SUPPORTED_LOCALES)[number];

export const SUPPORTED_LOCALES = [
  "en",
  "es",
  "zh",
  "ar",
  "hi",
  "pt",
  "fr",
  "de",
] as const;

export type Direction = "ltr" | "rtl";

/** BCP 47 tags for the eight supported locales. */
export const LOCALE_TAGS: Record<Locale, string> = {
  en: "en-GB",
  es: "es-ES",
  zh: "zh-Hans",
  ar: "ar",
  hi: "hi-IN",
  pt: "pt-BR",
  fr: "fr-FR",
  de: "de-DE",
};

export const LOCALE_NAMES: Record<Locale, string> = {
  en: "English",
  es: "Español",
  zh: "中文",
  ar: "العربية",
  hi: "हिन्दी",
  pt: "Português",
  fr: "Français",
  de: "Deutsch",
};

/**
 * Direction for a locale.
 *
 * The region subtag is stripped first, so `ar-EG` and `he-IL` resolve the
 * same way as their base language. An unknown locale is LTR: that is the
 * historical default and the safe failure mode for a layout.
 */
export function directionForLocale(locale: string | null | undefined): Direction {
  if (!locale) return "ltr";
  const base = String(locale).trim().toLowerCase().split(/[-_]/)[0];
  return (RTL_LOCALES as readonly string[]).includes(base) ? "rtl" : "ltr";
}

export const isRtlLocale = (locale: string | null | undefined): boolean =>
  directionForLocale(locale) === "rtl";

/**
 * The logical offset of an item, for grid navigation in a composite widget.
 * In RTL the inline axis is mirrored, so "next" is index - 1.
 */
export const inlineStep = (locale: string | null | undefined): 1 | -1 =>
  isRtlLocale(locale) ? -1 : 1;

/** True when `locale` is one of the shipped message catalogues. */
export function isSupportedLocale(locale: string): locale is Locale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(locale);
}

/** The `lang` attribute value for a locale, e.g. `zh-Hans`. */
export function langAttributeFor(locale: string): string {
  return isSupportedLocale(locale) ? LOCALE_TAGS[locale] : locale;
}

/**
 * Mixed-direction safety: wraps a value in an isolate so an RTL string
 * cannot reorder the surrounding punctuation of an LTR sentence.
 */
export function isolate(value: string, base: Direction = "ltr"): string {
  const marks = base === "rtl" ? ["\u2067", "\u2069"] : ["\u2066", "\u2069"];
  return `${marks[0]}${value}${marks[1]}`;
}
