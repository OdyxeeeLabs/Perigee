"use client";

import { useState } from "react";
import { useHydrateSettings, useSettingsStore } from "@/lib/settingsStore";
import { FormSkeleton } from "@/components/FormSkeleton";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { NetworkSettingsTab } from "./NetworkSettingsTab";
import { TradingSettingsTab } from "./TradingSettingsTab";
import { DisplaySettingsTab } from "./DisplaySettingsTab";
import { SecuritySettingsTab } from "./SecuritySettingsTab";
import { AdvancedSettingsTab } from "./AdvancedSettingsTab";

const TABS = [
  { id: "network", label: "Network", Component: NetworkSettingsTab },
  { id: "trading", label: "Trading", Component: TradingSettingsTab },
  { id: "display", label: "Display", Component: DisplaySettingsTab },
  { id: "security", label: "Security", Component: SecuritySettingsTab },
  { id: "advanced", label: "Advanced", Component: AdvancedSettingsTab },
] as const;

type TabId = (typeof TABS)[number]["id"];

/**
 * Settings page — issue #682.
 *
 * Persistence: there is no backend settings service yet, so this is a
 * client-only first cut backed by `localStorage` (see `lib/settings.ts` /
 * `lib/settingsStore.ts`). `useHydrateSettings()` defers reading
 * `localStorage` until after mount so the server-rendered markup and the
 * first client render match, showing `<FormSkeleton />` for that brief
 * window instead of a flash of default values.
 */
export function SettingsPage() {
  const hydrated = useHydrateSettings();
  const resetSettings = useSettingsStore((s) => s.resetSettings);
  const [activeTab, setActiveTab] = useState<TabId>("network");
  const [resetStatus, setResetStatus] = useState<"idle" | "done">("idle");
  const [confirmReset, setConfirmReset] = useState(false);

  const handleReset = () => {
    resetSettings();
    setResetStatus("done");
    setTimeout(() => setResetStatus("idle"), 2000);
  };

  const ActiveTabComponent = TABS.find((t) => t.id === activeTab)?.Component ?? NetworkSettingsTab;

  return (
    <div className="mx-auto w-full max-w-4xl px-4 py-8 text-slate-100">
      <header className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-slate-100">Settings</h1>
          <p className="mt-1 text-sm text-slate-400">
            Preferences are saved to this browser.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setConfirmReset(true)}
          className="rounded border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-300 hover:bg-slate-800"
        >
          {resetStatus === "done" ? "Reset ✓" : "Reset to defaults"}
        </button>
      </header>

      {!hydrated ? (
        <FormSkeleton fields={5} label="Settings loading" />
      ) : (
        <div className="overflow-hidden rounded-xl border border-slate-800">
          <div
            role="tablist"
            aria-label="Settings sections"
            className="flex flex-wrap gap-1 border-b border-slate-800 bg-slate-900 px-2 pt-2"
          >
            {TABS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                role="tab"
                id={`settings-tab-${tab.id}`}
                aria-selected={activeTab === tab.id}
                aria-controls={`settings-panel-${tab.id}`}
                onClick={() => setActiveTab(tab.id)}
                className={`rounded-t-md px-4 py-2 text-sm font-medium transition-colors ${
                  activeTab === tab.id
                    ? "bg-slate-950 text-cyan-400"
                    : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>

          <div
            role="tabpanel"
            id={`settings-panel-${activeTab}`}
            aria-labelledby={`settings-tab-${activeTab}`}
            className="bg-slate-950 p-6"
          >
            <ActiveTabComponent />
          </div>
        </div>
      )}
      <ConfirmDialog
        open={confirmReset}
        action="Reset settings"
        target="saved preferences"
        consequence="Every preference in this browser returns to its default value."
        confirmLabel="Reset settings"
        onConfirm={() => {
          setConfirmReset(false);
          handleReset();
        }}
        onCancel={() => setConfirmReset(false)}
      />
    </div>
  );
}
