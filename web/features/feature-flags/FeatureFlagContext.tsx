"use client";

import React, {
  createContext,
  useContext,
  useEffect,
  useState,
  useCallback,
} from "react";
import { featureFlagService } from "./featureFlagService";
import type {
  FeatureFlag,
  FeatureFlagEvaluation,
  FeatureFlagEvaluationContext,
  FeatureFlagMap,
  FeatureFlagProviderConfig,
} from "./types";

interface FeatureFlagContextValue {
  flags: FeatureFlagMap;
  isEnabled: (flag: FeatureFlag) => boolean;
  evaluate: (
    flag: FeatureFlag,
    context?: FeatureFlagEvaluationContext,
  ) => FeatureFlagEvaluation;
  refresh: () => Promise<void>;
}

const FeatureFlagContext =
  createContext<FeatureFlagContextValue | null>(null);

export function FeatureFlagProvider({
  children,
  config,
}: {
  children: React.ReactNode;
  config?: FeatureFlagProviderConfig;
}) {
  const [flags, setFlags] = useState<FeatureFlagMap>(
    featureFlagService.getAll(),
  );

  const sync = useCallback(() => {
    setFlags(featureFlagService.getAll());
  }, []);

  useEffect(() => {
    let cancelled = false;

    void featureFlagService.initialize(config).then(() => {
      if (!cancelled) sync();
    });

    return () => {
      cancelled = true;
      featureFlagService.stopPolling();
    };
  }, [config, sync]);

  const isEnabled = useCallback(
    (flag: FeatureFlag) =>
      featureFlagService.isEnabled(flag),
    [],
  );

  const evaluate = useCallback(
    (flag: FeatureFlag, context?: FeatureFlagEvaluationContext) =>
      featureFlagService.evaluate(flag, context),
    [],
  );

  const refresh = useCallback(async () => {
    if (config?.apiSource) {
      await featureFlagService.fetchFromApi(
        config.apiSource,
      );
      sync();
    }
  }, [config, sync]);

  return (
    <FeatureFlagContext.Provider
      value={{ flags, isEnabled, evaluate, refresh }}
    >
      {children}
    </FeatureFlagContext.Provider>
  );
}

export function useFeatureFlagContext(): FeatureFlagContextValue {
  const ctx = useContext(FeatureFlagContext);
  if (!ctx) {
    throw new Error(
      "useFeatureFlagContext must be used within a <FeatureFlagProvider>",
    );
  }
  return ctx;
}
