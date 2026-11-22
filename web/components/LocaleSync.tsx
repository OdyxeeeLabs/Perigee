"use client";

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import {
  DEFAULT_LOCALE,
  directionForLocale,
  normalizeLocale,
  readStoredLocale,
  type Locale,
} from "@/lib/locale/locales";

/**
 * The active locale, and the context that carries it.
 *
 * Declared beside the provider so the two cannot drift apart.
 */
const LocaleContext = createContext<Locale>(DEFAULT_LOCALE);

export const useLocale = (): Locale => useContext(LocaleContext);

/**
 * Applies the persisted locale to the document and keeps it in sync.
 *
 * The subtlety here is hydration. `localStorage` does not exist during SSR,
 * so the server always renders the default locale. If the client rendered the
 * stored locale immediately, React would report a hydration mismatch on every
 * page for every user who chose a non-default language. So the first client
 * render deliberately matches the server, and the stored preference is
 * applied in an effect — one frame of the default locale instead of a
 * hydration warning, and the `lang`/`dir` attributes are corrected within a
 * tick.
 *
 * `next-intl` handles the message catalogue; this component handles the
 * document-level attributes that CSS and assistive technology depend on.
 */
export function LocaleSync({ children }: { children?: ReactNode }) {
  const [locale, setLocale] = useState<Locale>(DEFAULT_LOCALE);

  useEffect(() => {
    const stored = readStoredLocale();
    setLocale(stored);
    applyLocale(stored);

    // Keep multiple tabs (and the language switcher) in agreement.
    const onStorage = (event: StorageEvent) => {
      if (!event.key) return;
      const next = normalizeLocale(event.newValue);
      setLocale(next);
      applyLocale(next);
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  return <LocaleContext.Provider value={locale}>{children}</LocaleContext.Provider>;
}

/** Sets `lang` and `dir` on `<html>`; safe to call outside React. */
export function applyLocale(locale: Locale): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.setAttribute("lang", locale);
  root.setAttribute("dir", directionForLocale(locale));
}
