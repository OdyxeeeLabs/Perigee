/**
 * Locale registry for the internationalisation layer — issue #700.
 *
 * The app already had `next-intl` wired with a single `messages/en.json`
 * catalogue. What was missing is the part that makes a second locale
 * possible: a single definition of the supported locales, their metadata,
 * and the formatting defaults that go with each one. Both the language
 * switcher and the formatters read from here, so a locale can never be
 * "supported" in the UI and unsupported in the data layer.
 *
 * Kept dependency-free and testable; the React binding is
 * `components/LanguageSwitcher.tsx`.
 */

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

export type Locale = (typeof SUPPORTED_LOCALES)[number];

export const DEFAULT_LOCALE: Locale = "en";

export const LOCALE_STORAGE_KEY = "perigee:locale";

/** Endonyms: a language picker is useless if it cannot be read. */
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

/** BCP 47 tags for `lang`/`Intl` construction. */
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

/** The Intl constructor tag, which needs the full BCP 47 form. */
export const intlTag = (locale: Locale): string => LOCALE_TAGS[locale] ?? locale;

/**
 * Plural categories per locale.
 *
 * `Intl.PluralRules` is the authority at runtime; this table exists so the
 * *test* for pluralisation can assert the expected category for a locale
 * without constructing a PluralRules instance, and so a translator knows
 * which categories their language needs.
 */
export const PLURAL_CATEGORIES: Record<Locale, readonly string[]> = {
  en: ["one", "other"],
  es: ["one", "many", "other"],
  zh: ["other"],
  ar: ["zero", "one", "two", "few", "many", "other"],
  hi: ["one", "other"],
  pt: ["one", "many", "other"],
  fr: ["one", "many", "other"],
  de: ["one", "other"],
};

export function isSupportedLocale(value: unknown): value is Locale {
  return typeof value === "string" && (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/** Coerce anything (URL segment, localStorage, cookie) to a supported locale. */
export function normalizeLocale(value: unknown): Locale {
  const raw = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (isSupportedLocale(raw)) return raw;

  // Accept a regional variant and fall back to its base language:
  // `pt-PT` and `pt_BR` both mean the catalogue we ship.
  const base = raw.split(/[-_]/)[0];
  return isSupportedLocale(base) ? base : DEFAULT_LOCALE;
}

/** `dir` for a locale. Arabic is the only RTL locale we ship. */
export const directionForLocale = (locale: Locale): "ltr" | "rtl" =>
  locale === "ar" ? "rtl" : "ltr";

export interface StoredLocale {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

/** Reads the persisted choice, tolerating private-mode failures. */
export function readStoredLocale(
  storage: StoredLocale | null | undefined = typeof localStorage === "undefined"
    ? null
    : localStorage,
): Locale {
  try {
    return normalizeLocale(storage?.getItem(LOCALE_STORAGE_KEY));
  } catch {
    return DEFAULT_LOCALE;
  }
}

/** Persists the choice; failures are non-fatal by design. */
export function storeLocale(
  locale: Locale,
  storage: StoredLocale | null | undefined = typeof localStorage === "undefined"
    ? null
    : localStorage,
): void {
  try {
    storage?.setItem(LOCALE_STORAGE_KEY, normalizeLocale(locale));
  } catch {
    // A preference that cannot be stored is a preference that does not
    // persist; it must not break the switcher.
  }
}
