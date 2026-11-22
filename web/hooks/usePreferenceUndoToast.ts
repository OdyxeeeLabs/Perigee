"use client";

import { useEffect } from "react";
import { useTranslations } from "next-intl";
import { useToast } from "@/context/ToastContext";
import { useSettingsStore } from "@/lib/settingsStore";
import { PREFERENCE_UNDO_WINDOW_MS } from "@/lib/settings";
import type { LocalPreferenceSection } from "@/lib/settings";

export function usePreferenceUndoToast(section: LocalPreferenceSection): void {
  const t = useTranslations("settings");
  const toast = useToast();
  const pendingUndo = useSettingsStore((s) => s.pendingUndo);
  const undoPreferenceChange = useSettingsStore((s) => s.undoPreferenceChange);

  useEffect(() => {
    if (!pendingUndo || pendingUndo.section !== section) return;

    const remaining = Math.max(
      1000,
      pendingUndo.changedAt + PREFERENCE_UNDO_WINDOW_MS - Date.now(),
    );
    const id = toast.info({
      title: t("preferenceUpdated"),
      duration: remaining,
      action: { label: t("undo"), onClick: undoPreferenceChange },
    });

    return () => toast.dismiss(id);
  }, [pendingUndo, section, toast, t, undoPreferenceChange]);
}
