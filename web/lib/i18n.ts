/**
 * i18n / white-label translation layer.
 *
 * Built on next-intl.  The default English messages live in
 * `messages/en.json`.  Wealth managers can override every visible string
 * by setting the `NEXT_PUBLIC_MANAGER_LABEL` environment variable to a
 * key that maps to a custom messages file (e.g. `messages/acme.json`).
 *
 * Resolves WEB-17 (#103): Navigation/metadata not localized for white-label.
 */
import { getRequestConfig } from "next-intl/server";
import { DEFAULT_LOCALE, normalizeLocale } from "./locale/locales";

async function loadMessages(locale: string): Promise<Record<string, string>> {
  const resolvedLocale = normalizeLocale(locale);
  const englishMessages: Record<string, string> = (
    await import("../messages/en.json")
  ).default;
  const localizedMessages: Record<string, string> =
    resolvedLocale === DEFAULT_LOCALE
      ? englishMessages
      : (await import(`../messages/${resolvedLocale}.json`)).default;
  // Fall back to English for keys a locale has not translated yet.
  const defaultMessages: Record<string, string> = {
    ...englishMessages,
    ...localizedMessages,
  };

  const managerLabel = process.env.NEXT_PUBLIC_MANAGER_LABEL?.trim();
  if (!managerLabel) return defaultMessages;

  try {
    const managerOverrides: Record<string, string | undefined> = (
      await import(`../messages/${managerLabel}.json`)
    ).default;
    const overrides = Object.fromEntries(
      Object.entries(managerOverrides).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );
    return { ...defaultMessages, ...overrides };
  } catch {
    console.warn(
      `[i18n] Manager override "${managerLabel}" messages file not found. Using defaults.`,
    );
    return defaultMessages;
  }
}

export default getRequestConfig(async ({ locale }) => {
  const resolvedLocale = normalizeLocale(locale ?? DEFAULT_LOCALE);
  return {
    locale: resolvedLocale,
    messages: await loadMessages(resolvedLocale),
    timeZone: "UTC",
  };
});
