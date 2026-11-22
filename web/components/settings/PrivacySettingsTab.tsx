"use client";

import { useEffect, useState } from "react";
import { useTranslations } from "next-intl";
import {
  getTelemetryConsent,
  getTelemetryPreferences,
  isDoNotTrackEnabled,
  optOutTelemetry,
  setTelemetryConsent,
  type TelemetryPreferences,
} from "@/lib/telemetry";
import { PrivacyNotice } from "@/components/PrivacyNotice";

const CATEGORIES: Array<{
  key: keyof TelemetryPreferences;
  label: string;
  description: string;
  inputId: string;
}> = [
  {
    key: "performance",
    label: "analytics.consentPerformance",
    description: "analytics.consentPerformanceDesc",
    inputId: "telemetry-performance",
  },
  {
    key: "errors",
    label: "analytics.consentErrors",
    description: "analytics.consentErrorsDesc",
    inputId: "telemetry-errors",
  },
  {
    key: "analytics",
    label: "analytics.consentProduct",
    description: "analytics.consentProductDesc",
    inputId: "telemetry-analytics",
  },
];

export function PrivacySettingsTab() {
  const t = useTranslations();
  const [prefs, setPrefs] = useState<TelemetryPreferences>({
    performance: false,
    errors: false,
    analytics: false,
  });
  const [consent, setConsent] = useState<boolean | null>(null);
  const [dntActive, setDntActive] = useState(false);
  const [savedAt, setSavedAt] = useState(0);

  useEffect(() => {
    setPrefs(getTelemetryPreferences());
    setConsent(getTelemetryConsent());
    setDntActive(isDoNotTrackEnabled());
  }, []);

  function applyPreferences(next: TelemetryPreferences) {
    const anyEnabled = next.performance || next.errors || Boolean(next.analytics);
    setTelemetryConsent(anyEnabled, next);
    setPrefs(next);
    setConsent(anyEnabled);
    setSavedAt(Date.now());
  }

  function handleToggle(key: keyof TelemetryPreferences) {
    applyPreferences({ ...prefs, [key]: !prefs[key] });
  }

  function handleOptOut() {
    optOutTelemetry();
    setPrefs({ performance: false, errors: false, analytics: false });
    setConsent(false);
    setSavedAt(Date.now());
  }

  return (
    <div className="flex flex-col gap-6">
      <section>
        <h3 className="text-sm font-semibold text-slate-200">
          {t("settings.privacy.title")}
        </h3>
        <p className="mt-1 max-w-2xl text-xs text-slate-400">
          {t("settings.privacy.description")}
        </p>
        <p className="mt-2 text-xs text-slate-300">
          <span className="font-medium">{t("settings.privacy.currentChoice")}: </span>
          {consent === true
            ? t("settings.privacy.enabled")
            : t("settings.privacy.disabled")}
        </p>
        {dntActive ? (
          <p className="mt-2 rounded border border-cyan-800 bg-cyan-950/40 p-2 text-xs text-cyan-300">
            {t("analytics.gpcDetected")}
          </p>
        ) : null}
      </section>

      <section aria-labelledby="privacy-notice-heading">
        <h4 id="privacy-notice-heading" className="mb-2 text-xs font-semibold text-slate-400">
          {t("settings.privacy.notice")}
        </h4>
        <PrivacyNotice />
      </section>

      <section aria-label={t("analytics.preferencesTitle")} className="flex flex-col gap-3">
        {CATEGORIES.map((category) => {
          const checked = Boolean(prefs[category.key]);
          return (
            <label
              key={category.key}
              htmlFor={category.inputId}
              className="flex max-w-2xl items-start gap-3 rounded border border-slate-700 bg-slate-900 p-3"
            >
              <input
                id={category.inputId}
                type="checkbox"
                checked={checked}
                onChange={() => handleToggle(category.key)}
                aria-describedby={`${category.inputId}-description`}
                className="mt-0.5 h-4 w-4 accent-cyan-500"
              />
              <span>
                <span className="block text-sm font-medium text-slate-100">
                  {t(category.label)}
                </span>
                <span
                  id={`${category.inputId}-description`}
                  className="mt-0.5 block text-xs text-slate-400"
                >
                  {t(category.description)}
                </span>
              </span>
            </label>
          );
        })}

        <button
          type="button"
          onClick={handleOptOut}
          className="self-start rounded border border-red-800 px-3 py-1.5 text-xs font-medium text-red-300 hover:bg-red-950/40"
        >
          {t("settings.privacy.reset")}
        </button>
      </section>

      <div role="status" aria-live="polite" className="text-xs text-cyan-400">
        {savedAt > 0 ? t("settings.privacy.statusSaved") : ""}
      </div>
    </div>
  );
}
