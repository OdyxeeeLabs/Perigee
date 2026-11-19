/**
 * Settings domain model + persistence — issue #682.
 *
 * There is no backend settings service yet, so this is a deliberate "first
 * cut": every section is a plain, validated object persisted to
 * `localStorage` under `SETTINGS_STORAGE_KEY`. Kept dependency-free and pure
 * so it can be unit tested without a DOM (see `settings.test.cjs`) and so the
 * React binding (`settingsStore.ts`) stays a thin wrapper.
 *
 * Judgment calls (documented further in the PR description):
 *  - The Network tab's "preferred network" is a *display* preference only.
 *    The actual Soroban network the app talks to is resolved at build time
 *    from `NEXT_PUBLIC_STELLAR_NETWORK` (see `lib/contracts.config.ts`
 *    `getStage()`), which `NetworkStatusBanner`/`RpcFallbackBanner` already
 *    read. This file does not duplicate that — it stores a custom RPC
 *    endpoint + chain ID override for forward compatibility, plus the
 *    preferred-network label shown in the UI.
 *  - Theme is intentionally NOT duplicated here: `components/ThemeToggle.tsx`
 *    already owns the `theme` localStorage key and <html class="dark">
 *    toggling. The Display tab renders `<ThemeToggle />` directly.
 */

export type PreferredNetwork = "mainnet" | "testnet";
export type GasPriceStrategy = "slow" | "standard" | "fast" | "custom";
export type Currency = "USD" | "EUR" | "GBP" | "XLM" | "USDC";
export type NumberFormat = "1,234.56" | "1.234,56" | "1 234.56";
export type SessionTimeoutMinutes = 15 | 30 | 60 | 120 | 0; // 0 = never

export interface NetworkSettings {
  /** UI-only preference; does not change the deployed contract network. */
  preferredNetwork: PreferredNetwork;
  /** Empty string = use the app default RPC endpoint. */
  customRpcEndpoint: string;
  /** Empty string = use the network's default chain id. */
  chainId: string;
}

export interface TradingSettings {
  /** Percent, clamped to [0.1, 10]. */
  slippageTolerance: number;
  gasPriceStrategy: GasPriceStrategy;
  /** Only meaningful when gasPriceStrategy === "custom". Stroops. */
  customGasPrice: number;
  /** Minutes before an unconfirmed transaction is considered expired. */
  transactionDeadlineMinutes: number;
}

export interface DisplaySettings {
  currency: Currency;
  numberFormat: NumberFormat;
  /** Mirrors the BCP-47 locale codes in lib/locale/locales.ts. */
  preferredLanguage: string;
}

export interface SecuritySettings {
  twoFactorEnabled: boolean;
  sessionTimeoutMinutes: SessionTimeoutMinutes;
}

export interface AdvancedSettings {
  debugMode: boolean;
  /** Local-only overrides layered on top of features/feature-flags defaults. */
  featureFlagOverrides: Record<string, boolean>;
}

export interface AppSettings {
  network: NetworkSettings;
  trading: TradingSettings;
  display: DisplaySettings;
  security: SecuritySettings;
  advanced: AdvancedSettings;
}

export const SETTINGS_STORAGE_KEY = "perigee_settings";

export const DEFAULT_SETTINGS: AppSettings = {
  network: {
    preferredNetwork: "testnet",
    customRpcEndpoint: "",
    chainId: "",
  },
  trading: {
    slippageTolerance: 0.5,
    gasPriceStrategy: "standard",
    customGasPrice: 100,
    transactionDeadlineMinutes: 20,
  },
  display: {
    currency: "USD",
    numberFormat: "1,234.56",
    preferredLanguage: "en",
  },
  security: {
    twoFactorEnabled: false,
    sessionTimeoutMinutes: 30,
  },
  advanced: {
    debugMode: false,
    featureFlagOverrides: {},
  },
};

export const CURRENCIES: Currency[] = ["USD", "EUR", "GBP", "XLM", "USDC"];
export const NUMBER_FORMATS: NumberFormat[] = ["1,234.56", "1.234,56", "1 234.56"];
export const GAS_PRICE_STRATEGIES: GasPriceStrategy[] = ["slow", "standard", "fast", "custom"];
export const SESSION_TIMEOUT_OPTIONS: SessionTimeoutMinutes[] = [15, 30, 60, 120, 0];

export const MIN_SLIPPAGE = 0.1;
export const MAX_SLIPPAGE = 10;

/** Clamp slippage tolerance to the supported [0.1, 10] percent range. */
export function clampSlippage(value: number): number {
  if (Number.isNaN(value)) return DEFAULT_SETTINGS.trading.slippageTolerance;
  return Math.min(MAX_SLIPPAGE, Math.max(MIN_SLIPPAGE, value));
}

/** Clamp a transaction deadline to a sane [1, 180] minute range. */
export function clampDeadlineMinutes(value: number): number {
  if (Number.isNaN(value)) return DEFAULT_SETTINGS.trading.transactionDeadlineMinutes;
  return Math.min(180, Math.max(1, Math.round(value)));
}

/** Clamp a custom gas price (stroops) to a non-negative integer. */
export function clampGasPrice(value: number): number {
  if (Number.isNaN(value) || value < 0) return DEFAULT_SETTINGS.trading.customGasPrice;
  return Math.round(value);
}

export const PREFERENCE_UNDO_WINDOW_MS = 7000;

export type LocalPreferenceSection = "display" | "trading";

export interface PendingPreferenceUndo {
  section: LocalPreferenceSection;
  field: string;
  previousValue: unknown;
  changedAt: number;
  sequence: number;
}

export function canUndoPreference(
  pending: PendingPreferenceUndo | null,
  now: number = Date.now(),
): boolean {
  return pending !== null && now - pending.changedAt <= PREFERENCE_UNDO_WINDOW_MS;
}

export function recordPreferenceUndo(
  pending: PendingPreferenceUndo | null,
  section: LocalPreferenceSection,
  field: string,
  previousValue: unknown,
  now: number = Date.now(),
  sequence: number = now,
): PendingPreferenceUndo {
  if (
    pending &&
    pending.section === section &&
    pending.field === field &&
    canUndoPreference(pending, now)
  ) {
    return pending;
  }
  return { section, field, previousValue, changedAt: now, sequence };
}

/**
 * Deep-merges a partial/untrusted object onto the defaults so that:
 *  - a corrupted or partially-written localStorage blob never crashes the UI
 *  - new fields added in a later release get sane defaults for existing users
 */
export function mergeWithDefaults(partial: unknown): AppSettings {
  const p = (partial && typeof partial === "object" ? partial : {}) as Partial<AppSettings>;

  return {
    network: { ...DEFAULT_SETTINGS.network, ...(p.network ?? {}) },
    trading: {
      ...DEFAULT_SETTINGS.trading,
      ...(p.trading ?? {}),
      slippageTolerance: clampSlippage(
        Number(p.trading?.slippageTolerance ?? DEFAULT_SETTINGS.trading.slippageTolerance),
      ),
      transactionDeadlineMinutes: clampDeadlineMinutes(
        Number(
          p.trading?.transactionDeadlineMinutes ??
            DEFAULT_SETTINGS.trading.transactionDeadlineMinutes,
        ),
      ),
      customGasPrice: clampGasPrice(
        Number(p.trading?.customGasPrice ?? DEFAULT_SETTINGS.trading.customGasPrice),
      ),
    },
    display: { ...DEFAULT_SETTINGS.display, ...(p.display ?? {}) },
    security: { ...DEFAULT_SETTINGS.security, ...(p.security ?? {}) },
    advanced: {
      ...DEFAULT_SETTINGS.advanced,
      ...(p.advanced ?? {}),
      featureFlagOverrides: {
        ...DEFAULT_SETTINGS.advanced.featureFlagOverrides,
        ...(p.advanced?.featureFlagOverrides ?? {}),
      },
    },
  };
}

/** Parse a raw localStorage string into validated AppSettings. Never throws. */
export function parseStoredSettings(raw: string | null): AppSettings {
  if (!raw) return DEFAULT_SETTINGS;
  try {
    const parsed = JSON.parse(raw);
    return mergeWithDefaults(parsed);
  } catch {
    return DEFAULT_SETTINGS;
  }
}

/** Read settings from localStorage. Safe to call during SSR (returns defaults). */
export function loadSettings(): AppSettings {
  if (typeof window === "undefined") return DEFAULT_SETTINGS;
  try {
    return parseStoredSettings(window.localStorage.getItem(SETTINGS_STORAGE_KEY));
  } catch {
    // localStorage can throw in private browsing / sandboxed frames.
    return DEFAULT_SETTINGS;
  }
}

/** Persist settings to localStorage. No-ops during SSR or when storage throws. */
export function persistSettings(settings: AppSettings): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // Ignore quota / privacy-mode failures — settings simply won't persist.
  }
}

/** Serializes settings for the Advanced tab's "Export data" action. */
export function exportSettingsAsJson(settings: AppSettings): string {
  return JSON.stringify(settings, null, 2);
}
