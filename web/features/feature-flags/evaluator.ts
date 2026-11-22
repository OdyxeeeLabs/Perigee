import type {
  FeatureFlag,
  FeatureFlagDefinition,
  FeatureFlagEvaluation,
  FeatureFlagEvaluationContext,
  FeatureFlagTargetAttribute,
  FeatureFlagTargetingRule,
  FeatureFlagValue,
} from "./types";

const TARGET_ATTRIBUTES: FeatureFlagTargetAttribute[] = [
  "userId",
  "walletAddress",
  "country",
  "betaGroup",
];

function isValue(value: unknown): value is FeatureFlagValue {
  return typeof value === "boolean" || typeof value === "number" || typeof value === "string";
}

function subjectFor(attribute: FeatureFlagTargetAttribute, context: FeatureFlagEvaluationContext): string | undefined {
  if (attribute === "betaGroup") return context.betaGroups?.slice().sort().join(",");
  return context[attribute];
}

function stableBucket(flag: FeatureFlag, subject: string): number {
  let hash = 2166136261;
  for (const byte of `${flag}:${subject}`) {
    hash ^= byte.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) % 10000;
}

function ruleMatches(flag: FeatureFlag, rule: FeatureFlagTargetingRule, context: FeatureFlagEvaluationContext): boolean {
  if (!TARGET_ATTRIBUTES.includes(rule.attribute)) return false;
  const subject = subjectFor(rule.attribute, context);
  if (!subject) return false;
  if (rule.values?.length) {
    const values = rule.values.map((value) => rule.attribute === "country" ? value.toLowerCase() : value);
    if (rule.attribute === "betaGroup") {
      if (!(context.betaGroups ?? []).some((group) => values.includes(group))) return false;
    } else {
      const candidate = rule.attribute === "country" ? subject.toLowerCase() : subject;
      if (!values.includes(candidate)) return false;
    }
  }
  if (rule.percentage !== undefined) {
    if (!Number.isFinite(rule.percentage) || rule.percentage < 0 || rule.percentage > 100) return false;
    if (stableBucket(flag, subject) >= rule.percentage * 100) return false;
  }
  return true;
}

function validVariant(definition: FeatureFlagDefinition, variant: string | undefined): variant is string {
  return variant !== undefined && definition.variants !== undefined &&
    Object.prototype.hasOwnProperty.call(definition.variants, variant) && isValue(definition.variants[variant]);
}

export function evaluateFeatureFlag(definition: FeatureFlagDefinition, context: FeatureFlagEvaluationContext = {}): FeatureFlagEvaluation {
  if (definition.killSwitch) return { key: definition.key, enabled: false, value: false, variant: null, reason: "kill-switch" };
  const matchedRule = (definition.targeting ?? []).find((rule) => ruleMatches(definition.key, rule, context));
  const selectedVariant = matchedRule?.variant ?? definition.defaultVariant;
  if (definition.variants && validVariant(definition, selectedVariant)) {
    return { key: definition.key, enabled: definition.enabled, value: definition.variants[selectedVariant], variant: selectedVariant, reason: matchedRule ? "targeting" : "default" };
  }
  if (definition.variants && selectedVariant !== undefined) return { key: definition.key, enabled: false, value: false, variant: null, reason: "fallback" };
  if (definition.targeting?.length && !matchedRule) return { key: definition.key, enabled: false, value: false, variant: null, reason: "targeting" };
  return { key: definition.key, enabled: definition.enabled, value: definition.enabled, variant: null, reason: matchedRule ? "targeting" : "default" };
}

export function isValidFeatureFlagDefinition(definition: FeatureFlagDefinition): boolean {
  if (!definition.key || typeof definition.enabled !== "boolean") return false;
  if (definition.variants) {
    if (!Object.keys(definition.variants).length || Object.values(definition.variants).some((value) => !isValue(value))) return false;
    if (definition.defaultVariant !== undefined && !validVariant(definition, definition.defaultVariant)) return false;
  }
  if (definition.killSwitch !== undefined && typeof definition.killSwitch !== "boolean") return false;
  return (definition.targeting ?? []).every((rule) => {
    if (!TARGET_ATTRIBUTES.includes(rule.attribute)) return false;
    if (rule.values?.some((value) => typeof value !== "string")) return false;
    return rule.percentage === undefined || (Number.isFinite(rule.percentage) && rule.percentage >= 0 && rule.percentage <= 100);
  });
}
