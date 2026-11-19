"use client";

import { useSettingsStore } from "@/lib/settingsStore";
import { CURRENCIES, NUMBER_FORMATS, type Currency, type NumberFormat } from "@/lib/settings";
import { REFRESH_INTERVAL_OPTIONS, type RefreshIntervalMs } from "@/lib/refreshInterval";
import { ThemeToggle } from "@/components/ThemeToggle";
import { LOCALE_NAMES, SUPPORTED_LOCALES } from "@/lib/locale/locales";

/**
 * Display tab — issue #682: currency, number format, theme, language.
 *
 * Theme reuses `<ThemeToggle />` as-is rather than re-implementing
 * `<html class="dark">` toggling here — it already owns the `theme`
 * localStorage key.
 *
 * Language: full locale *routing* (`/es/...`) is a Pages Router feature
 * (`next.config.js` `i18n` block) that next-intl's own App Router plugin
 * flags as unsupported for `app/`, and `components/LanguageSwitcher.tsx`
 * depends on `next/router`, which has no App Router instance. Rather than
 * import a component that would throw here, this tab persists a
 * "preferred language" display preference (surfaced for a future App
 * Router-native switcher) and points users at the real switcher in the nav.
 */
export function DisplaySettingsTab() {
  const display = useSettingsStore((s) => s.settings.display);
  const updateDisplay = useSettingsStore((s) => s.updateDisplay);

  return (
    <div className="flex flex-col gap-6">
      <section>
        <label htmlFor="currency" className="text-sm font-semibold text-slate-200">
          Currency
        </label>
        <select
          id="currency"
          value={display.currency}
          onChange={(e) => updateDisplay({ currency: e.target.value as Currency })}
          className="mt-2 w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        >
          {CURRENCIES.map((currency) => (
            <option key={currency} value={currency}>
              {currency}
            </option>
          ))}
        </select>
      </section>

      <section>
        <label htmlFor="number-format" className="text-sm font-semibold text-slate-200">
          Number format
        </label>
        <select
          id="number-format"
          value={display.numberFormat}
          onChange={(e) => updateDisplay({ numberFormat: e.target.value as NumberFormat })}
          className="mt-2 w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        >
          {NUMBER_FORMATS.map((format) => (
            <option key={format} value={format}>
              {format}
            </option>
          ))}
        </select>
      </section>

      <section>
        <label htmlFor="refresh-interval" className="text-sm font-semibold text-slate-200">
          Dashboard refresh rate
        </label>
        <p className="mt-1 text-xs text-slate-400">
          How often read-only dashboards check for new data. The default is every minute.
          Transaction confirmations and security notices are never delayed.
        </p>
        <select
          id="refresh-interval"
          value={display.refreshIntervalMs}
          onChange={(e) => updateDisplay({ refreshIntervalMs: Number(e.target.value) as RefreshIntervalMs })}
          className="mt-2 w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        >
          {REFRESH_INTERVAL_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </section>

      <section>
        <span className="text-sm font-semibold text-slate-200">Theme</span>
        <div className="mt-2">
          <ThemeToggle />
        </div>
      </section>

      <section>
        <label htmlFor="preferred-language" className="text-sm font-semibold text-slate-200">
          Preferred language
        </label>
        <p className="mt-1 text-xs text-slate-400">
          Saved as a preference for an upcoming App Router language switcher. To change the active
          language right now, use the language picker in the navigation bar.
        </p>
        <select
          id="preferred-language"
          value={display.preferredLanguage}
          onChange={(e) => updateDisplay({ preferredLanguage: e.target.value })}
          className="mt-2 w-full max-w-xs rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        >
          {SUPPORTED_LOCALES.map((locale) => (
            <option key={locale} value={locale}>
              {LOCALE_NAMES[locale] ?? locale}
            </option>
          ))}
        </select>
      </section>
    </div>
  );
}
