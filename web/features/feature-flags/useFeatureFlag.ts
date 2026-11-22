import { useCallback, useMemo } from "react";
import { useFeatureFlagContext } from "./FeatureFlagContext";
import type {
  FeatureFlag,
  FeatureFlagEvaluation,
  FeatureFlagEvaluationContext,
} from "./types";

export function useFeatureFlag(flag: FeatureFlag): boolean {
  const { isEnabled } = useFeatureFlagContext();
  return isEnabled(flag);
}

export function useFeatureEvaluation(
  flag: FeatureFlag,
  context?: FeatureFlagEvaluationContext,
): FeatureFlagEvaluation {
  const { evaluate } = useFeatureFlagContext();
  return evaluate(flag, context);
}

export function useFeatureFlags(): {
  flags: Record<string, boolean>;
  isEnabled: (flag: FeatureFlag) => boolean;
  evaluate: (
    flag: FeatureFlag,
    context?: FeatureFlagEvaluationContext,
  ) => FeatureFlagEvaluation;
  refresh: () => Promise<void>;
} {
  const { flags, isEnabled, evaluate, refresh } =
    useFeatureFlagContext();

  const stableIsEnabled = useCallback(
    (f: FeatureFlag) => isEnabled(f),
    [isEnabled],
  );

  const memoisedFlags = useMemo(
    () => ({ ...flags }),
    [flags],
  );

  return {
    flags: memoisedFlags,
    isEnabled: stableIsEnabled,
    evaluate,
    refresh,
  };
}
