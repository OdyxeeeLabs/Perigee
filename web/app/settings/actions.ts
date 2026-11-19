"use server";

import type { AppSettings } from "@/lib/settings";
import {
  validateSettingsValues,
  type SettingsFieldError,
} from "@/lib/settingsValidation";

export interface SettingsValidationResult {
  ok: boolean;
  errors: SettingsFieldError[];
}

export async function validateSettingsUpdate(
  settings: AppSettings,
  values: Record<string, string>,
): Promise<SettingsValidationResult> {
  const errors = validateSettingsValues(values, settings);
  const rpcEndpoint = values["custom-rpc"] ?? "";

  if (
    process.env.NODE_ENV === "production" &&
    /(localhost|127\.0\.0\.1|\[::1\])/i.test(rpcEndpoint) &&
    !errors.some((error) => error.id === "custom-rpc")
  ) {
    errors.push({
      id: "custom-rpc",
      message: "Use a public RPC endpoint in production.",
      label: "Custom RPC endpoint",
    });
  }

  return { ok: errors.length === 0, errors };
}
