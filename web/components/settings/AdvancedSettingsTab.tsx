"use client";

import { useState } from "react";
import { useSettingsStore } from "@/lib/settingsStore";
import { exportSettingsAsJson } from "@/lib/settings";
import { featureFlagService, featureFlags, type FeatureFlag } from "@/features/feature-flags";

/**
 * Advanced tab — issue #682: debug mode, feature flags, data export.
 *
 * Feature flags reuse the existing `featureFlagService` singleton
 * (`features/feature-flags/featureFlagService.ts`) directly rather than the
 * React `<FeatureFlagProvider>` context, since that provider is only mounted
 * in the Pages Router tree (`pages/_app.tsx`) and this page renders under
 * the App Router (`app/settings/page.tsx`), which has no such provider.
 * Toggling here calls `featureFlagService.set(...)`, which is the same
 * module-level store the context reads from, so overrides still apply
 * anywhere `isEnabled()` is checked.
 */
export function AdvancedSettingsTab() {
  const advanced = useSettingsStore((s) => s.settings.advanced);
  const updateAdvanced = useSettingsStore((s) => s.updateAdvanced);
  const settings = useSettingsStore((s) => s.settings);
  const [flagVersion, setFlagVersion] = useState(0);
  const [exportStatus, setExportStatus] = useState<"idle" | "done">("idle");

  const flagValues = featureFlagService.getAll();

  const handleToggleFlag = (key: FeatureFlag) => {
    featureFlagService.toggle(key);
    updateAdvanced({ featureFlagOverrides: { [key]: !flagValues[key] } });
    setFlagVersion((v) => v + 1); // force re-read of featureFlagService.getAll()
  };

  const handleExport = () => {
    const json = exportSettingsAsJson(settings);
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "perigee-settings.json";
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);
    setExportStatus("done");
    setTimeout(() => setExportStatus("idle"), 2500);
  };

  return (
    <div className="flex flex-col gap-6" key={flagVersion}>
      <section>
        <div className="flex items-center justify-between max-w-md">
          <div>
            <span className="text-sm font-semibold text-slate-200">Debug mode</span>
            <p className="mt-1 text-xs text-slate-400">
              Surfaces verbose logging and internal state in the console.
            </p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={advanced.debugMode}
            aria-label="Toggle debug mode"
            onClick={() => updateAdvanced({ debugMode: !advanced.debugMode })}
            className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
              advanced.debugMode ? "bg-cyan-500" : "bg-slate-700"
            }`}
          >
            <span
              className={`absolute start-0.5 top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
                advanced.debugMode ? "translate-x-5 rtl:-translate-x-5" : "translate-x-0"
              }`}
            />
          </button>
        </div>
      </section>

      <section>
        <span className="text-sm font-semibold text-slate-200">Feature flags</span>
        <p className="mt-1 text-xs text-slate-400">
          Local overrides for this browser only. Remote/env-configured flags still take effect on
          next load.
        </p>
        <ul className="mt-2 flex max-w-md flex-col gap-2">
          {featureFlags.map((flag) => (
            <li
              key={flag.key}
              className="flex items-center justify-between rounded border border-slate-700 bg-slate-900 px-3 py-2"
            >
              <div className="min-w-0 pe-3">
                <div className="text-sm text-slate-200">{flag.key}</div>
                {flag.description && (
                  <div className="truncate text-xs text-slate-400">{flag.description}</div>
                )}
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={flagValues[flag.key]}
                aria-label={`Toggle ${flag.key}`}
                onClick={() => handleToggleFlag(flag.key)}
                className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
                  flagValues[flag.key] ? "bg-cyan-500" : "bg-slate-700"
                }`}
              >
                <span
                  className={`absolute start-0.5 top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
                    flagValues[flag.key] ? "translate-x-5 rtl:-translate-x-5" : "translate-x-0"
                  }`}
                />
              </button>
            </li>
          ))}
        </ul>
      </section>

      <section>
        <span className="text-sm font-semibold text-slate-200">Data export</span>
        <p className="mt-1 text-xs text-slate-400">
          Download all Perigee settings stored in this browser as a JSON file.
        </p>
        <button
          type="button"
          onClick={handleExport}
          className="mt-2 rounded bg-slate-800 px-3 py-1.5 text-sm font-medium text-slate-200 hover:bg-slate-700"
        >
          {exportStatus === "done" ? "Downloaded ✓" : "Export settings as JSON"}
        </button>
      </section>
    </div>
  );
}
