import {
  CURRENCIES,
  GAS_PRICE_STRATEGIES,
  MAX_SLIPPAGE,
  MIN_SLIPPAGE,
  NUMBER_FORMATS,
  SESSION_TIMEOUT_OPTIONS,
  type AppSettings,
} from "./settings";
import { SUPPORTED_LOCALES } from "./locale/locales";

export interface SettingsFieldError {
  id: string;
  message: string;
  label?: string;
}

export type SettingsTabId = "network" | "trading" | "display" | "security" | "advanced";

interface SettingsFieldDefinition {
  label: string;
  tab: SettingsTabId;
  validate: (value: string, settings: AppSettings) => string | null;
}

function finiteNumber(value: string): number | null {
  if (!value.trim()) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function integerInRange(min: number, max: number): (value: string) => string | null {
  return (value) => {
    if (!value.trim()) return "Enter a value.";
    const parsed = Number(value);
    if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) return "Enter a whole number.";
    if (parsed < min || parsed > max) return `Enter a value between ${min} and ${max}.`;
    return null;
  };
}

export const SETTINGS_FIELDS: Record<string, SettingsFieldDefinition> = {
  "preferred-network": {
    label: "Preferred network",
    tab: "network",
    validate: (value) => (value === "mainnet" || value === "testnet" ? null : "Choose a supported network."),
  },
  "custom-rpc": {
    label: "Custom RPC endpoint",
    tab: "network",
    validate: (value) => {
      if (!value.trim()) return null;
      try {
        const url = new URL(value);
        if (url.protocol !== "http:" && url.protocol !== "https:") {
          return "Use an http:// or https:// URL.";
        }
        return null;
      } catch {
        return "Enter a valid URL.";
      }
    },
  },
  "chain-id": {
    label: "Chain ID",
    tab: "network",
    validate: (value) => (value.length > 200 ? "Chain ID is too long." : null),
  },
  slippage: {
    label: "Slippage tolerance",
    tab: "trading",
    validate: (value) => {
      const parsed = finiteNumber(value);
      if (parsed === null) return "Enter a number.";
      if (parsed < MIN_SLIPPAGE || parsed > MAX_SLIPPAGE) {
        return `Enter a value between ${MIN_SLIPPAGE}% and ${MAX_SLIPPAGE}%.`;
      }
      return null;
    },
  },
  "custom-gas": {
    label: "Custom gas price",
    tab: "trading",
    validate: (value) => {
      if (!value.trim()) return "Enter a value.";
      const parsed = Number(value);
      if (!Number.isFinite(parsed) || !Number.isInteger(parsed) || parsed < 0) {
        return "Enter a non-negative whole number.";
      }
      return null;
    },
  },
  "tx-deadline": {
    label: "Transaction deadline",
    tab: "trading",
    validate: integerInRange(1, 180),
  },
  "gas-strategy": {
    label: "Gas price strategy",
    tab: "trading",
    validate: (value) =>
      (GAS_PRICE_STRATEGIES as readonly string[]).includes(value)
        ? null
        : "Choose a supported gas price strategy.",
  },
  currency: {
    label: "Currency",
    tab: "display",
    validate: (value) => (CURRENCIES.includes(value as (typeof CURRENCIES)[number]) ? null : "Choose a supported currency."),
  },
  "number-format": {
    label: "Number format",
    tab: "display",
    validate: (value) =>
      (NUMBER_FORMATS as readonly string[]).includes(value) ? null : "Choose a supported number format.",
  },
  "preferred-language": {
    label: "Preferred language",
    tab: "display",
    validate: (value) =>
      (SUPPORTED_LOCALES as readonly string[]).includes(value)
        ? null
        : "Choose a supported language.",
  },
  "session-timeout": {
    label: "Session timeout",
    tab: "security",
    validate: (value) => {
      const parsed = finiteNumber(value);
      if (parsed === null || !SESSION_TIMEOUT_OPTIONS.includes(parsed as (typeof SESSION_TIMEOUT_OPTIONS)[number])) {
        return "Choose a supported session timeout.";
      }
      return null;
    },
  },
};

export function settingsFieldValues(settings: AppSettings): Record<string, string> {
  return {
    "preferred-network": settings.network.preferredNetwork,
    "custom-rpc": settings.network.customRpcEndpoint,
    "chain-id": settings.network.chainId,
    slippage: String(settings.trading.slippageTolerance),
    "custom-gas": String(settings.trading.customGasPrice),
    "tx-deadline": String(settings.trading.transactionDeadlineMinutes),
    "gas-strategy": settings.trading.gasPriceStrategy,
    currency: settings.display.currency,
    "number-format": settings.display.numberFormat,
    "preferred-language": settings.display.preferredLanguage,
    "session-timeout": String(settings.security.sessionTimeoutMinutes),
  };
}

export function validateSettingsValues(
  values: Record<string, string>,
  settings: AppSettings,
): SettingsFieldError[] {
  const errors: SettingsFieldError[] = [];
  const defaults = settingsFieldValues(settings);
  for (const [id, definition] of Object.entries(SETTINGS_FIELDS)) {
    const value = values[id] ?? defaults[id] ?? "";
    const message = definition.validate(value, settings);
    if (message) errors.push({ id, message, label: definition.label });
  }
  return errors;
}

export function fieldErrorsToMap(errors: readonly SettingsFieldError[]): Record<string, string> {
  const map: Record<string, string> = {};
  for (const error of errors) map[error.id] = error.message;
  return map;
}

export function errorsFromMap(errors: Record<string, string>): SettingsFieldError[] {
  return Object.entries(errors)
    .filter(([, message]) => Boolean(message))
    .map(([id, message]) => ({
      id,
      message,
      label: SETTINGS_FIELDS[id]?.label,
    }));
}

interface ServerErrorEntry {
  field?: string;
  id?: string;
  message?: string;
}

export function normalizeServerErrors(payload: unknown): SettingsFieldError[] {
  if (!payload || typeof payload !== "object") return [];
  const body = payload as {
    fieldErrors?: Record<string, string>;
    errors?: ServerErrorEntry[];
    message?: string;
  };
  const errors: SettingsFieldError[] = [];

  for (const [id, message] of Object.entries(body.fieldErrors ?? {})) {
    if (typeof message === "string" && message) {
      errors.push({ id, message, label: SETTINGS_FIELDS[id]?.label });
    }
  }

  for (const entry of body.errors ?? []) {
    const id = entry.field ?? entry.id;
    if (id && entry.message) {
      errors.push({ id, message: entry.message, label: SETTINGS_FIELDS[id]?.label });
    }
  }

  if (errors.length === 0 && typeof body.message === "string" && body.message) {
    const fallbackId = "custom-rpc";
    errors.push({
      id: fallbackId,
      message: body.message,
      label: SETTINGS_FIELDS[fallbackId]?.label,
    });
  }

  return errors;
}
