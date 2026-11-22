"use client";

import { useEffect, useState, type ReactNode } from "react";
import { NextIntlClientProvider } from "next-intl";
import { ToastProvider } from "@/context/ToastContext";
import { DEFAULT_LOCALE, readStoredLocale, type Locale } from "@/lib/locale/locales";
import ar from "@/messages/ar.json";
import de from "@/messages/de.json";
import en from "@/messages/en.json";
import es from "@/messages/es.json";
import fr from "@/messages/fr.json";
import hi from "@/messages/hi.json";
import pt from "@/messages/pt.json";
import zh from "@/messages/zh.json";

const catalogs: Record<Locale, Record<string, string>> = {
  ar,
  de,
  en,
  es,
  fr,
  hi,
  pt,
  zh,
};

export function SettingsIntlProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState<Locale>(DEFAULT_LOCALE);

  useEffect(() => {
    setLocale(readStoredLocale());
  }, []);

  return (
    <NextIntlClientProvider locale={locale} messages={catalogs[locale]} timeZone="UTC">
      <ToastProvider>{children}</ToastProvider>
    </NextIntlClientProvider>
  );
}
