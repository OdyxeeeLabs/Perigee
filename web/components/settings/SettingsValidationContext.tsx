"use client";

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { AppSettings } from "@/lib/settings";
import { settingsStore } from "@/lib/settingsStore";
import { settingsFieldValues } from "@/lib/settingsValidation";

interface SettingsValidationValue {
  values: Record<string, string>;
  errors: Record<string, string>;
  setValue: (id: string, value: string) => void;
  setError: (id: string, message: string | null) => void;
  setErrors: (errors: Record<string, string>) => void;
  clearErrors: () => void;
  syncFromSettings: () => void;
  fieldProps: (
    id: string,
  ) => { id: string; "aria-invalid": boolean; "aria-describedby"?: string };
}

const SettingsValidationContext = createContext<SettingsValidationValue | null>(null);

export function SettingsValidationProvider({
  settings,
  children,
}: {
  settings: AppSettings;
  children: ReactNode;
}) {
  const [values, setValues] = useState<Record<string, string>>(() =>
    settingsFieldValues(settings),
  );
  const [errors, setErrorsState] = useState<Record<string, string>>({});

  const setValue = useCallback((id: string, value: string) => {
    setValues((prev) => ({ ...prev, [id]: value }));
  }, []);

  const setError = useCallback((id: string, message: string | null) => {
    setErrorsState((prev) => {
      const next = { ...prev };
      if (message) next[id] = message;
      else delete next[id];
      return next;
    });
  }, []);

  const setErrors = useCallback((next: Record<string, string>) => {
    setErrorsState(next);
  }, []);

  const clearErrors = useCallback(() => setErrorsState({}), []);

  const syncFromSettings = useCallback(() => {
    setValues(settingsFieldValues(settingsStore.getState().settings));
  }, []);

  const fieldProps = useCallback(
    (id: string) => ({
      id,
      "aria-invalid": Boolean(errors[id]),
      ...(errors[id] ? { "aria-describedby": `${id}-error` } : {}),
    }),
    [errors],
  );

  const value = useMemo<SettingsValidationValue>(
    () => ({
      values,
      errors,
      setValue,
      setError,
      setErrors,
      clearErrors,
      syncFromSettings,
      fieldProps,
    }),
    [values, errors, setValue, setError, setErrors, clearErrors, syncFromSettings, fieldProps],
  );

  return (
    <SettingsValidationContext.Provider value={value}>
      {children}
    </SettingsValidationContext.Provider>
  );
}

export function useSettingsValidation(): SettingsValidationValue {
  const context = useContext(SettingsValidationContext);
  if (!context) {
    throw new Error("useSettingsValidation must be used inside SettingsValidationProvider");
  }
  return context;
}
