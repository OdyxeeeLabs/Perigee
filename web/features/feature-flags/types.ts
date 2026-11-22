import { FEATURE_FLAGS } from "./constants";

export type FeatureFlag =
  (typeof FEATURE_FLAGS)[keyof typeof FEATURE_FLAGS];

export type FeatureFlagValue = boolean | number | string;

export type FeatureFlagTargetAttribute =
  | "userId"
  | "walletAddress"
  | "country"
  | "betaGroup";

export interface FeatureFlagEvaluationContext {
  userId?: string;
  walletAddress?: string;
  country?: string;
  betaGroups?: string[];
}

export interface FeatureFlagTargetingRule {
  attribute: FeatureFlagTargetAttribute;
  values?: string[];
  percentage?: number;
  variant?: string;
}

export interface FeatureFlagEvaluation {
  key: FeatureFlag;
  enabled: boolean;
  value: FeatureFlagValue;
  variant: string | null;
  reason: "default" | "targeting" | "kill-switch" | "fallback";
}

export interface FeatureFlagDefinition {
  key: FeatureFlag;

  enabled: boolean;

  description?: string;

  variants?: Record<string, FeatureFlagValue>;
  defaultVariant?: string;
  targeting?: FeatureFlagTargetingRule[];
  killSwitch?: boolean;
}

export type FeatureFlagMap = Record<
  FeatureFlag,
  boolean
>;

export interface FeatureFlagApiSource {
  url: string;
  headers?: Record<string, string>;
  /** Interval in ms to re-fetch flags. Defaults to 60 000. */
  pollingInterval?: number;
}

export interface FeatureFlagProviderConfig {
  /** API endpoint to fetch remote flags from. */
  apiSource?: FeatureFlagApiSource;
  /** Additional env-var prefix to look for (default: "NEXT_PUBLIC_FEATURE_FLAG_"). */
  envPrefix?: string;
}
