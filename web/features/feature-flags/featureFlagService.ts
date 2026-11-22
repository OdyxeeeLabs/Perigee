import {
  DEFAULT_FLAG_VALUE,
  DEFAULT_POLLING_INTERVAL_MS,
  ENV_PREFIX,
} from "./constants";

import { featureFlags } from "./feature-flags";
import { evaluateFeatureFlag, isValidFeatureFlagDefinition } from "./evaluator";

import type {
  FeatureFlag,
  FeatureFlagApiSource,
  FeatureFlagDefinition,
  FeatureFlagEvaluation,
  FeatureFlagEvaluationContext,
  FeatureFlagMap,
  FeatureFlagProviderConfig,
} from "./types";

function readEnvFlags(
  prefix: string,
): Partial<FeatureFlagMap> {
  if (typeof process === "undefined") return {};

  const result: Partial<FeatureFlagMap> = {};
  const allFlags = Object.values(
    featureFlags.reduce(
      (acc, f) => {
        acc[f.key] = f.key;
        return acc;
      },
      {} as Record<string, string>,
    ),
  );

  for (const key of allFlags) {
    const envKey = `${prefix}${key}`;
    const raw = process.env[envKey];
    if (raw !== undefined) {
      result[key as FeatureFlag] = raw === "true";
    }
  }

  return result;
}

class FeatureFlagService {
  private flags: FeatureFlagMap;
  private definitions: Record<FeatureFlag, FeatureFlagDefinition>;
  private pollTimer: ReturnType<typeof setInterval> | null = null;

  constructor() {
    this.flags = {} as FeatureFlagMap;
    this.definitions = {} as Record<FeatureFlag, FeatureFlagDefinition>;
    this.reset();
  }

  getAll(): FeatureFlagMap {
    return { ...this.flags };
  }

  isEnabled(flag: FeatureFlag, context?: FeatureFlagEvaluationContext): boolean {
    return this.evaluate(flag, context).enabled;
  }

  evaluate(
    flag: FeatureFlag,
    context: FeatureFlagEvaluationContext = {},
  ): FeatureFlagEvaluation {
    const definition = this.definitions[flag];
    if (!definition) {
      return {
        key: flag,
        enabled: DEFAULT_FLAG_VALUE,
        value: DEFAULT_FLAG_VALUE,
        variant: null,
        reason: "fallback",
      };
    }
    return evaluateFeatureFlag(definition, context);
  }

  getDefinition(flag: FeatureFlag): FeatureFlagDefinition | undefined {
    const definition = this.definitions[flag];
    return definition
      ? {
          ...definition,
          variants: definition.variants
            ? { ...definition.variants }
            : undefined,
          targeting: definition.targeting?.map((rule) => ({
            ...rule,
            values: rule.values ? [...rule.values] : undefined,
          })),
        }
      : undefined;
  }

  enable(flag: FeatureFlag): void {
    this.flags[flag] = true;
    if (this.definitions[flag]) this.definitions[flag].enabled = true;
  }

  disable(flag: FeatureFlag): void {
    this.flags[flag] = false;
    if (this.definitions[flag]) this.definitions[flag].enabled = false;
  }

  toggle(flag: FeatureFlag): void {
    this.set(flag, !this.flags[flag]);
  }

  set(flag: FeatureFlag, enabled: boolean): void {
    this.flags[flag] = enabled;
    if (this.definitions[flag]) this.definitions[flag].enabled = enabled;
  }

  reset(): void {
    this.flags = featureFlags.reduce(
      (acc, flag) => {
        acc[flag.key] = flag.enabled;
        return acc;
      },
      {} as FeatureFlagMap,
    );
    this.definitions = featureFlags.reduce(
      (acc, flag) => {
        acc[flag.key] = {
          ...flag,
          variants: flag.variants ? { ...flag.variants } : undefined,
          targeting: flag.targeting?.map((rule) => ({ ...rule })),
        };
        return acc;
      },
      {} as Record<FeatureFlag, FeatureFlagDefinition>,
    );
  }

  applyEnvOverrides(prefix: string = ENV_PREFIX): void {
    const envFlags = readEnvFlags(prefix);
    for (const [key, value] of Object.entries(envFlags)) {
      this.set(key as FeatureFlag, value as boolean);
    }
  }

  async fetchFromApi(
    source: FeatureFlagApiSource,
  ): Promise<void> {
    const controller = new AbortController();
    const timeout = setTimeout(
      () => controller.abort(),
      5_000,
    );

    try {
      const res = await fetch(source.url, {
        headers: source.headers ?? {},
        signal: controller.signal,
      });

      if (!res.ok) return;

      const data: unknown = await res.json();

      if (
        data &&
        typeof data === "object" &&
        !Array.isArray(data)
      ) {
        for (const [key, value] of Object.entries(
          data as Record<string, unknown>,
        )) {
          if (!(key in this.flags)) continue;
          if (typeof value === "boolean") {
            this.set(key as FeatureFlag, value);
            continue;
          }
          if (!value || typeof value !== "object" || Array.isArray(value)) continue;
          const candidate = {
            ...this.definitions[key as FeatureFlag],
            ...(value as Partial<FeatureFlagDefinition>),
            key: key as FeatureFlag,
          };
          if (isValidFeatureFlagDefinition(candidate)) {
            this.definitions[key as FeatureFlag] = candidate;
            this.flags[key as FeatureFlag] = candidate.enabled;
          }
        }
      }
    } catch {
      // Silently ignore — flags retain their previous state
    } finally {
      clearTimeout(timeout);
    }
  }

  startPolling(source: FeatureFlagApiSource): void {
    this.stopPolling();
    const interval =
      source.pollingInterval ??
      DEFAULT_POLLING_INTERVAL_MS;

    void this.fetchFromApi(source);
    this.pollTimer = setInterval(() => {
      void this.fetchFromApi(source);
    }, interval);
  }

  stopPolling(): void {
    if (this.pollTimer !== null) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  async initialize(
    config?: FeatureFlagProviderConfig,
  ): Promise<void> {
    this.reset();

    this.applyEnvOverrides(config?.envPrefix);

    if (config?.apiSource) {
      await this.fetchFromApi(config.apiSource);
      this.startPolling(config.apiSource);
    }
  }
}

export const featureFlagService =
  new FeatureFlagService();
