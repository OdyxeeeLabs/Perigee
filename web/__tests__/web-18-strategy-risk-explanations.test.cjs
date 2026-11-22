// web/__tests__/web-18-strategy-risk-explanations.test.cjs — tests for WEB-18
// (#818): strategy selection must be accompanied by risk explanations that are
// descriptive, bound-aware, and never colour-only.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('path');

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');

test('every published phase declares a risk level, summary and bound', () => {
  const phases = read('components/strategy/strategyPhases.ts');

  assert.ok(phases.includes('export type StrategyRiskLevel'), 'risk level type is exported');
  assert.ok(phases.includes('risk: StrategyPhaseRisk'), 'phase data requires risk info');
  assert.ok(phases.includes('level: "medium"'), 'accumulation risk declared');
  assert.ok(phases.includes('level: "high"'), 'distribution risk declared');
  assert.ok(phases.includes('level: "low"'), 'markdown risk declared');

  const levelCount = (phases.match(/\blevel: "/g) || []).length;
  const summaryCount = (phases.match(/\bsummary: "/g) || []).length;
  const mitigationCount = (phases.match(/\bmitigation: "/g) || []).length;
  assert.equal(levelCount, 4, 'four phases declare a level');
  assert.equal(summaryCount, 4, 'four phases declare a risk summary');
  assert.equal(mitigationCount, 4, 'four phases declare a mitigation bound');
});

test('the explanations component renders text labels, not colour-only status', () => {
  const component = read('components/strategy/StrategyRiskExplanations.tsx');

  assert.ok(component.includes('data-testid="strategy-risk-explanations"'), 'component is testable');
  assert.ok(component.includes('aria-labelledby="strategy-risk-heading"'), 'has an accessible heading');
  assert.ok(component.includes('Lower risk'), 'low risk has a text label');
  assert.ok(component.includes('Moderate risk'), 'medium risk has a text label');
  assert.ok(component.includes('Higher risk'), 'high risk has a text label');
  assert.ok(component.includes('phase.risk.summary'), 'risk summary is displayed');
  assert.ok(component.includes('phase.risk.mitigation'), 'rule bound is displayed');
});

test('explanations are rendered alongside the strategy phase table', () => {
  const diagram = read('components/strategy/StrategyPhaseDiagram.tsx');
  assert.ok(diagram.includes('import { StrategyRiskExplanations }'), 'component is imported');
  assert.ok(diagram.includes('<StrategyRiskExplanations />'), 'component is rendered next to the table');
  assert.ok(diagram.includes('<StrategyPhaseTable />'), 'phase table remains available');
});

test('risk copy does not invent client-calculated values', () => {
  const data = read('components/strategy/strategyPhases.ts');
  assert.ok(!/Math\.|calculate|compute|performance_fee/.test(data), 'copy is descriptive, not computed');
  assert.ok(/maximum basket weight/.test(data), 'explanations cite published bounds');
  assert.ok(/drawdown thresholds/.test(data), 'explanations cite published triggers');
});
