import { strict as assert } from "node:assert";
import test from "node:test";
import { evaluateFeatureFlag, isValidFeatureFlagDefinition } from "./evaluator.ts";
import type { FeatureFlagDefinition } from "./types.ts";

const flag = (overrides: Partial<FeatureFlagDefinition> = {}): FeatureFlagDefinition => ({
  key: "dashboardV2",
  enabled: false,
  ...overrides,
});

test("preserves boolean defaults and kill-switch precedence", () => {
  assert.equal(evaluateFeatureFlag(flag({ enabled: true })).enabled, true);
  const disabled = evaluateFeatureFlag(flag({ enabled: true, killSwitch: true }));
  assert.equal(disabled.enabled, false);
  assert.equal(disabled.reason, "kill-switch");
});

test("selects variants and falls back safely", () => {
  const definition = flag({
    enabled: true,
    variants: { control: "control", experiment: "experiment" },
    defaultVariant: "control",
    targeting: [{ attribute: "betaGroup", values: ["early-access"], variant: "experiment" }],
  });
  assert.equal(evaluateFeatureFlag(definition, { betaGroups: ["early-access"] }).value, "experiment");
  assert.equal(evaluateFeatureFlag(definition, { betaGroups: ["general"] }).value, "control");
  assert.equal(evaluateFeatureFlag({ ...definition, defaultVariant: "missing" }).reason, "fallback");
});

test("percentage targeting is deterministic for a stable subject", () => {
  const definition = flag({ enabled: true, targeting: [{ attribute: "userId", percentage: 50 }] });
  const context = { userId: "user-42" };
  const first = evaluateFeatureFlag(definition, context);
  assert.deepEqual(evaluateFeatureFlag(definition, context), first);
});

test("supports wallet and country targeting and fails closed without context", () => {
  const definition = flag({ enabled: true, targeting: [{ attribute: "country", values: ["ng"] }] });
  assert.equal(evaluateFeatureFlag(definition, { country: "NG" }).enabled, true);
  assert.equal(evaluateFeatureFlag(definition, { walletAddress: "G..." }).enabled, false);
  const walletDefinition = flag({ enabled: true, targeting: [{ attribute: "walletAddress", values: ["G..."], variant: undefined }] });
  assert.equal(evaluateFeatureFlag(walletDefinition, { walletAddress: "G..." }).enabled, true);
});

test("rejects invalid rollout and variant configuration", () => {
  assert.equal(isValidFeatureFlagDefinition(flag({ targeting: [{ attribute: "userId", percentage: 101 }] })), false);
  assert.equal(isValidFeatureFlagDefinition(flag({ variants: { a: "A" }, defaultVariant: "b" })), false);
});
