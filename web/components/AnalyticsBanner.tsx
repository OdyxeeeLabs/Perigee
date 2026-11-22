"use client";

/**
 * Analytics consent banner with:
 *  - 30-day TTL persistence — the banner only re-surfaces when the stored
 *    consent has expired (configurable via CONSENT_TTL_MS in telemetry.ts).
 *  - "Manage preferences" — lets users toggle individual analytics categories
 *    (performance analytics, error reporting) independently.
 *  - Fully i18n-ready — all strings are sourced from next-intl message keys.
 *
 * Resolves WEB-analytics (#task-4): consent banner always re-shown; no
 * granular preference control.
 */

import { useEffect, useState } from "react";
import { useTranslations } from "next-intl";
import { PrivacyNotice } from "./PrivacyNotice";
import {
  getTelemetryConsent,
  getTelemetryPreferences,
  setTelemetryConsent,
  initPrivacyTelemetry,
  isDoNotTrackEnabled,
  optOutTelemetry,
  type TelemetryPreferences,
} from "../lib/telemetry";

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function AnalyticsBanner() {
  const t = useTranslations();

  // Banner visibility state
  const [showBanner, setShowBanner] = useState(false);
  // Whether the detailed "manage preferences" panel is open
  const [showPreferences, setShowPreferences] = useState(false);
  // Do Not Track / Global Privacy Control active
  const [dntActive, setDntActive] = useState(false);

  // Granular preference toggles (populated from storage or defaults)
  const [prefs, setPrefs] = useState<TelemetryPreferences>({
    performance: false,
    errors: false,
    analytics: false,
  });

  useEffect(() => {
    const isDnt = isDoNotTrackEnabled();
    setDntActive(isDnt);

    const consent = getTelemetryConsent();
    if (consent === null) {
      // No stored decision or it has expired — show the banner
      const stored = getTelemetryPreferences();
      setPrefs(stored);
      setShowBanner(true);
    } else if (consent === true) {
      // Previously accepted — initialise the telemetry script immediately
      initPrivacyTelemetry();
    }

    // Listen for custom event to reopen preferences anytime (e.g. from footer or command palette)
    const handleOpenPrefs = () => {
      setPrefs(getTelemetryPreferences());
      setShowPreferences(true);
      setShowBanner(true);
    };

    window.addEventListener("perigee:open-privacy-preferences", handleOpenPrefs);
    return () => {
      window.removeEventListener("perigee:open-privacy-preferences", handleOpenPrefs);
    };
  }, []);

  // ---------------------------------------------------------------------------
  // Handlers
  // ---------------------------------------------------------------------------

  function handleAccept() {
    setTelemetryConsent(true);
    setShowBanner(false);
    setShowPreferences(false);
  }

  function handleDecline() {
    setTelemetryConsent(false);
    setShowBanner(false);
    setShowPreferences(false);
  }

  function handleSavePreferences() {
    const anyEnabled = prefs.performance || prefs.errors || Boolean(prefs.analytics);
    setTelemetryConsent(anyEnabled, prefs);
    setShowBanner(false);
    setShowPreferences(false);
  }

  function handleOptOutAll() {
    optOutTelemetry();
    setPrefs({ performance: false, errors: false, analytics: false });
    setShowBanner(false);
    setShowPreferences(false);
  }

  function handleTogglePref(key: keyof TelemetryPreferences) {
    setPrefs((prev) => ({ ...prev, [key]: !prev[key] }));
  }

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  if (!showBanner) return null;

  return (
    <>
      {/* ── Consent Banner ─────────────────────────────────────────────────── */}
      {!showPreferences && (
        <aside
          aria-label={t("analytics.bannerAriaLabel")}
          role="dialog"
          aria-modal="false"
          className="fixed bottom-4 right-4 z-50 max-w-sm rounded-xl border border-slate-800 bg-slate-900/95 p-4 shadow-xl text-slate-200 text-xs backdrop-blur"
        >
          <div className="flex items-center justify-between mb-1">
            <p className="font-semibold text-slate-100">
              {t("analytics.title")}
            </p>
            {dntActive && (
              <span className="rounded-full bg-cyan-950 px-2 py-0.5 text-[10px] font-medium text-cyan-300 border border-cyan-800">
                DNT Active
              </span>
            )}
          </div>
          <p className="text-slate-400 mb-3">{t("analytics.description")}</p>

          <PrivacyNotice />

          {dntActive && (
            <p className="mb-3 rounded bg-slate-800/80 p-2 text-[11px] text-cyan-300 border border-slate-700">
              {t("analytics.gpcDetected")}
            </p>
          )}

          <div className="flex items-center justify-between gap-2">
            {/* Manage preferences link */}
            <button
              type="button"
              onClick={() => setShowPreferences(true)}
              className="text-cyan-400 underline underline-offset-2 hover:text-cyan-300 transition-colors"
            >
              {t("analytics.managePreferences")}
            </button>

            <div className="flex gap-2">
              <button
                type="button"
                onClick={handleDecline}
                className="rounded px-3 py-1.5 border border-slate-700 hover:bg-slate-800 text-slate-300 transition-colors"
              >
                {t("analytics.decline")}
              </button>
              <button
                type="button"
                onClick={handleAccept}
                className="rounded px-3 py-1.5 bg-cyan-600 hover:bg-cyan-500 font-medium text-white transition-colors"
              >
                {t("analytics.accept")}
              </button>
            </div>
          </div>
        </aside>
      )}

      {/* ── Manage Preferences Panel ────────────────────────────────────────── */}
      {showPreferences && (
        <aside
          aria-label={t("analytics.preferencesAriaLabel")}
          role="dialog"
          aria-modal="false"
          className="fixed bottom-4 right-4 z-50 w-84 max-w-[calc(100vw-2rem)] rounded-xl border border-slate-800 bg-slate-900/95 p-4 shadow-xl text-slate-200 text-xs backdrop-blur"
        >
          <p className="font-semibold text-slate-100 mb-1">
            {t("analytics.preferencesTitle")}
          </p>
          <p className="text-slate-400 mb-3">{t("analytics.preferencesBody")}</p>

          <a
            href="/settings?tab=privacy"
            className="mb-3 block text-cyan-400 underline underline-offset-2 hover:text-cyan-300"
          >
            {t("analytics.settingsPrivacyLink")}
          </a>

          {dntActive && (
            <div className="mb-3 rounded bg-cyan-950/40 border border-cyan-800/60 p-2 text-[11px] text-cyan-300">
              {t("analytics.gpcDetected")}
            </div>
          )}

          {/* Performance toggle */}
          <label className="flex items-start gap-3 mb-3 cursor-pointer">
            <input
              type="checkbox"
              checked={prefs.performance}
              onChange={() => handleTogglePref("performance")}
              className="mt-0.5 h-3.5 w-3.5 rounded accent-cyan-500"
            />
            <span>
              <span className="font-medium text-slate-100 block">
                {t("analytics.consentPerformance")}
              </span>
              <span className="text-slate-400">
                {t("analytics.consentPerformanceDesc")}
              </span>
            </span>
          </label>

          {/* Error reporting toggle */}
          <label className="flex items-start gap-3 mb-3 cursor-pointer">
            <input
              type="checkbox"
              checked={prefs.errors}
              onChange={() => handleTogglePref("errors")}
              className="mt-0.5 h-3.5 w-3.5 rounded accent-cyan-500"
            />
            <span>
              <span className="font-medium text-slate-100 block">
                {t("analytics.consentErrors")}
              </span>
              <span className="text-slate-400">
                {t("analytics.consentErrorsDesc")}
              </span>
            </span>
          </label>

          {/* Product & Funnel toggle */}
          <label className="flex items-start gap-3 mb-4 cursor-pointer">
            <input
              type="checkbox"
              checked={Boolean(prefs.analytics)}
              onChange={() => handleTogglePref("analytics")}
              className="mt-0.5 h-3.5 w-3.5 rounded accent-cyan-500"
            />
            <span>
              <span className="font-medium text-slate-100 block">
                {t("analytics.consentProduct")}
              </span>
              <span className="text-slate-400">
                {t("analytics.consentProductDesc")}
              </span>
            </span>
          </label>

          <div className="flex items-center justify-between border-t border-slate-800 pt-3">
            <button
              type="button"
              onClick={handleOptOutAll}
              className="text-[11px] text-red-400 hover:text-red-300 underline underline-offset-2 transition-colors"
            >
              {t("analytics.optOut")}
            </button>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => setShowPreferences(false)}
                className="rounded px-2.5 py-1.5 border border-slate-700 hover:bg-slate-800 text-slate-300 transition-colors"
              >
                {t("analytics.cancel")}
              </button>
              <button
                type="button"
                onClick={handleSavePreferences}
                className="rounded px-3 py-1.5 bg-cyan-600 hover:bg-cyan-500 font-medium text-white transition-colors"
              >
                {t("analytics.savePreferences")}
              </button>
            </div>
          </div>
        </aside>
      )}
    </>
  );
}
