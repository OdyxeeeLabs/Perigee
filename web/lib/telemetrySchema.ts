/**
 * Event schema definitions and validation for Perigee product telemetry.
 *
 * Defines explicit event categories, event names, allowed payloads,
 * and funnel steps (Connect Wallet -> View Vault -> Deposit).
 */

import { sanitizeTelemetryProperties } from "./telemetryPii";

export type EventCategory = "funnel" | "pageview" | "feature" | "error" | "performance";

/** Supported funnel event names */
export type FunnelEventName =
  | "wallet_connect_initiated"
  | "wallet_connect"
  | "wallet_connected"
  | "wallet_disconnect"
  | "wallet_error"
  | "vault_list_viewed"
  | "vault_viewed"
  | "vault_filter_applied"
  | "deposit_initiated"
  | "deposit_amount_entered"
  | "deposit_submitted"
  | "deposit_success"
  | "deposit_failed"
  | "vault_create";

/** Supported pageview event names */
export type PageViewEventName = "page_view";

/** Supported feature usage event names */
export type FeatureUsageEventName =
  | "contract_analyze"
  | "palette_open"
  | "palette_copy_address"
  | "palette_navigate"
  | "theme_toggle"
  | "feature_flag_evaluated"
  | "feature_used";

/** Supported error event names */
export type ErrorEventName = "error_occurred" | "error_reported";

/** Supported performance event names */
export type PerformanceEventName =
  | "performance_vital"
  | "hydration_time"
  | "slow_render";

export type KnownEventName =
  | FunnelEventName
  | PageViewEventName
  | FeatureUsageEventName
  | ErrorEventName
  | PerformanceEventName
  | string;

export interface TelemetryEvent {
  name: KnownEventName;
  properties?: Record<string, string | number | boolean>;
  category?: EventCategory;
  timestamp?: number;
}

export interface FunnelStepDefinition {
  step: 1 | 2 | 3;
  name: string;
  stageName: string;
}

const FUNNEL_EVENTS: Record<string, FunnelStepDefinition> = {
  // Step 1: Connect Wallet
  wallet_connect_initiated: { step: 1, name: "wallet_connect_initiated", stageName: "Connect Wallet" },
  wallet_connect: { step: 1, name: "wallet_connect", stageName: "Connect Wallet" },
  wallet_connected: { step: 1, name: "wallet_connected", stageName: "Connect Wallet" },

  // Step 2: View Vault
  vault_list_viewed: { step: 2, name: "vault_list_viewed", stageName: "View Vault" },
  vault_viewed: { step: 2, name: "vault_viewed", stageName: "View Vault" },
  vault_filter_applied: { step: 2, name: "vault_filter_applied", stageName: "View Vault" },

  // Step 3: Deposit
  deposit_initiated: { step: 3, name: "deposit_initiated", stageName: "Deposit" },
  deposit_amount_entered: { step: 3, name: "deposit_amount_entered", stageName: "Deposit" },
  deposit_submitted: { step: 3, name: "deposit_submitted", stageName: "Deposit" },
  deposit_success: { step: 3, name: "deposit_success", stageName: "Deposit" },
};

/**
 * Returns the funnel step number (1 = Connect Wallet, 2 = View Vault, 3 = Deposit)
 * or null if the event is not a primary funnel step.
 */
export function getFunnelStep(eventName: string): 1 | 2 | 3 | null {
  return FUNNEL_EVENTS[eventName]?.step ?? null;
}

/**
 * Resolves the event category from the event name.
 */
export function getEventCategory(eventName: string): EventCategory {
  if (eventName in FUNNEL_EVENTS || eventName === "wallet_disconnect" || eventName === "wallet_error" || eventName === "vault_create" || eventName === "deposit_failed") {
    return "funnel";
  }
  if (eventName === "page_view") {
    return "pageview";
  }
  if (
    eventName === "contract_analyze" ||
    eventName.startsWith("palette_") ||
    eventName === "theme_toggle" ||
    eventName === "feature_used" ||
    eventName === "feature_flag_evaluated"
  ) {
    return "feature";
  }
  if (eventName.includes("error") || eventName === "error_occurred" || eventName === "error_reported") {
    return "error";
  }
  if (
    eventName === "performance_vital" ||
    eventName === "hydration_time" ||
    eventName === "slow_render" ||
    eventName.includes("vital")
  ) {
    return "performance";
  }
  return "feature";
}

/**
 * Validates and normalizes an event against the Telemetry schema.
 * Rejects invalid event names or payload structures, scrubs PII,
 * and attaches standard metadata (timestamp, category).
 */
export function validateTelemetryEvent(raw: unknown): {
  valid: boolean;
  event?: TelemetryEvent;
  error?: string;
} {
  if (!raw || typeof raw !== "object") {
    return { valid: false, error: "Event must be a non-null object" };
  }

  const candidate = raw as Record<string, unknown>;

  if (typeof candidate.name !== "string" || candidate.name.trim().length === 0) {
    return { valid: false, error: "Event name must be a non-empty string" };
  }

  const name = candidate.name.trim();
  if (name.length > 64) {
    return { valid: false, error: "Event name exceeds maximum length of 64 characters" };
  }

  // Scrub and validate properties
  const rawProps =
    candidate.properties && typeof candidate.properties === "object"
      ? (candidate.properties as Record<string, unknown>)
      : {};

  const cleanProps = sanitizeTelemetryProperties(rawProps);

  const category = (candidate.category as EventCategory) || getEventCategory(name);
  const timestamp =
    typeof candidate.timestamp === "number" && Number.isFinite(candidate.timestamp)
      ? candidate.timestamp
      : Date.now();

  return {
    valid: true,
    event: {
      name,
      properties: cleanProps,
      category,
      timestamp,
    },
  };
}
