"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { useHydrateSettings, useSettingsStore } from "@/lib/settingsStore";
import { FormSkeleton } from "@/components/FormSkeleton";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { ErrorSummary, type FieldError } from "@/components/ErrorSummary";
import {
  errorsFromMap,
  fieldErrorsToMap,
  normalizeServerErrors,
  SETTINGS_FIELDS,
  validateSettingsValues,
} from "@/lib/settingsValidation";
import { validateSettingsUpdate } from "@/app/settings/actions";
import { NetworkSettingsTab } from "./NetworkSettingsTab";
import { TradingSettingsTab } from "./TradingSettingsTab";
import { DisplaySettingsTab } from "./DisplaySettingsTab";
import { PrivacySettingsTab } from "./PrivacySettingsTab";
import { SecuritySettingsTab } from "./SecuritySettingsTab";
import { AdvancedSettingsTab } from "./AdvancedSettingsTab";
import {
  SettingsValidationProvider,
  useSettingsValidation,
} from "./SettingsValidationContext";

const TABS = [
  { id: "network", label: "Network", Component: NetworkSettingsTab },
  { id: "trading", label: "Trading", Component: TradingSettingsTab },
  { id: "display", label: "Display", Component: DisplaySettingsTab },
  { id: "privacy", label: "Privacy", Component: PrivacySettingsTab },
  { id: "security", label: "Security", Component: SecuritySettingsTab },
  { id: "advanced", label: "Advanced", Component: AdvancedSettingsTab },
] as const;

type TabId = (typeof TABS)[number]["id"];

export function SettingsPage() {
  const hydrated = useHydrateSettings();
  const settings = useSettingsStore((s) => s.settings);

  if (!hydrated) {
    return <FormSkeleton fields={5} label="Settings loading" />;
  }

  return (
    <SettingsValidationProvider settings={settings}>
      <SettingsPageContent />
    </SettingsValidationProvider>
  );
}

function SettingsPageContent() {
  const t = useTranslations();
  const settings = useSettingsStore((s) => s.settings);
  const resetSettings = useSettingsStore((s) => s.resetSettings);
  const pendingUndo = useSettingsStore((s) => s.pendingUndo);
  const validation = useSettingsValidation();
  const previousPendingUndo = useRef(pendingUndo);
  const [activeTab, setActiveTab] = useState<TabId>("network");
  const [resetStatus, setResetStatus] = useState<"idle" | "done">("idle");
  const [confirmReset, setConfirmReset] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");

  useEffect(() => {
    const requested = new URLSearchParams(window.location.search).get("tab");
    if (requested && TABS.some((tab) => tab.id === requested)) {
      setActiveTab(requested as TabId);
    }
  }, []);

  useEffect(() => {
    if (previousPendingUndo.current && !pendingUndo) {
      validation.syncFromSettings();
    }
    previousPendingUndo.current = pendingUndo;
  }, [pendingUndo, validation.syncFromSettings]);

  const errorList = useMemo<FieldError[]>(
    () => errorsFromMap(validation.errors),
    [validation.errors],
  );

  const handleReset = () => {
    resetSettings();
    validation.syncFromSettings();
    validation.clearErrors();
    setResetStatus("done");
    announceStatus("Settings reset to defaults.");
    setTimeout(() => setResetStatus("idle"), 2000);
  };

  const handleSave = async () => {
    validation.clearErrors();
    setSaveStatus("saving");

    const clientErrors = validateSettingsValues(validation.values, settings);
    if (clientErrors.length > 0) {
      validation.setErrors(fieldErrorsToMap(clientErrors));
      setSaveStatus("error");
      const firstTab = SETTINGS_FIELDS[clientErrors[0].id]?.tab;
      if (firstTab) setActiveTab(firstTab);
      return;
    }

    try {
      const result = await validateSettingsUpdate(settings, validation.values);
      if (!result.ok) {
        validation.setErrors(fieldErrorsToMap(result.errors));
        setSaveStatus("error");
        return;
      }
      setSaveStatus("saved");
      setTimeout(() => setSaveStatus("idle"), 2000);
    } catch {
      validation.setErrors(
        fieldErrorsToMap(normalizeServerErrors({ message: t("settings.errorServer") })),
      );
      setSaveStatus("error");
    }
  };

  const ActiveTabComponent = TABS.find((tab) => tab.id === activeTab)?.Component ?? NetworkSettingsTab;

  return (
    <div className="mx-auto w-full max-w-4xl px-4 py-8 text-slate-100">
      <header className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-slate-100">{t("settings.title")}</h1>
          <p className="mt-1 text-sm text-slate-400">{t("settings.subtitle")}</p>
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
            {saveStatus === "saving"
              ? t("settings.saving")
              : saveStatus === "saved"
                ? t("settings.saved")
                : t("settings.save")}
          </button>
          <button
            type="button"
            onClick={handleReset}
            className="rounded border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-300 hover:bg-slate-800"
          >
            {resetStatus === "done" ? "Reset ✓" : "Reset to defaults"}
          </button>
        </div>
      </header>

      <div className="mb-4">
        <ErrorSummary
          errors={errorList}
          title={t("settings.errorSummaryTitle")}
          onDismiss={validation.clearErrors}
          onNavigate={(id) => {
            const tab = SETTINGS_FIELDS[id]?.tab;
            if (tab) setActiveTab(tab);
          }}
        />
      </div>

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
