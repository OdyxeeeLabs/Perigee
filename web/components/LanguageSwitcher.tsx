"use client";

import { useRouter } from "next/router";
import { useId } from "react";
import {
  directionForLocale,
  LOCALE_NAMES,
  SUPPORTED_LOCALES,
  storeLocale,
  type Locale,
} from "@/lib/locale/locales";

/**
 * Language switcher — issue #700.
 *
 * The locale lives in the URL (Next.js i18n routing, `next.config.js`) rather
 * than only in localStorage, so a shared link opens in the reader's language
 * and the back button moves through language changes like any other
 * navigation. The localStorage copy is still written: it is what lets the app
 * restore the reader's *default* language when they land on the unprefixed
 * English URL, and it is what `LocaleSync` reads for `lang`/`dir`.
 *
 * Endonyms, not English names: a reader who cannot read the current UI
 * language still has to find their own in the list. An English-labelled picker
 * defeats the point of having one.
 */
export function LanguageSwitcher({ locale }: { locale?: Locale }) {
  const router = useRouter();
  const selectId = useId();
  const current = normalizeFromRouter(locale ?? router?.locale);

  return (
    <div className="flex items-center gap-2">
      <label htmlFor={selectId} className="text-sm">
        Language
      </label>
      <select
        id={selectId}
        value={current}
        lang={current}
        dir={directionForLocale(current)}
        onChange={(event) => {
          const next = event.target.value as Locale;
          storeLocale(next);
          // `asPath` keeps the reader where they are — switching language on
          // a vault page must not return them to the dashboard.
          void router?.push(router.asPath, router.asPath, { locale: next, scroll: false });
        }}
        className="rounded border px-2 py-1 text-sm"
      >
        {SUPPORTED_LOCALES.map((option) => (
          <option key={option} value={option} lang={option}>
            {LOCALE_NAMES[option]}
          </option>
        ))}
      </select>
    </div>
  );
}

/** `router.locale` is undefined on the Pages Router before the route settles. */
function normalizeFromRouter(value: string | undefined): Locale {
  if (value && (SUPPORTED_LOCALES as readonly string[]).includes(value)) {
    return value as Locale;
  }
  return "en";
}

export default LanguageSwitcher;
