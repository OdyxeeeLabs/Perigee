'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const WEB_ROOT = path.join(__dirname, '..');
const read = (...parts) => fs.readFileSync(path.join(WEB_ROOT, ...parts), 'utf8');

test('presentational components are memoised (#692)', () => {
  for (const file of [
    ['components', 'ui', 'Badge.tsx'],
    ['components', 'ui', 'Spinner.tsx'],
    ['components', 'NutritionLabel.tsx'],
    ['components', 'GasUsageChart.tsx'],
    ['components', 'GasGolfingSuggestionsTable.tsx'],
    ['components', 'ResourceHeatmap.tsx'],
  ]) {
    assert.match(read(...file), /React\.memo\(/, `${file.join('/')} must use React.memo`);
  }
  assert.match(read('components', 'strategy', 'StrategyPhaseTable.tsx'), /memo\(StrategyPhaseTableImpl\)/);
});

test('exhaustive-deps stays an error (#692)', () => {
  assert.match(read('.eslintrc.json'), /"react-hooks\/exhaustive-deps": "error"/);
});

test('web vitals are reported from _app and thresholds follow Google guidance (#693)', () => {
  assert.match(read('pages', '_app.tsx'), /export \{ reportWebVitals \}/);
  const vitals = read('lib', 'webVitals.ts');
  assert.match(vitals, /LCP: \[2500, 4000\]/);
  assert.match(vitals, /CLS: \[0\.1, 0\.25\]/);
  assert.match(vitals, /getTelemetryConsent\(\) !== true/);
});

test('vitals stats use nearest-rank percentiles (#693)', () => {
  const src = read('lib', 'vitalsStats.ts');
  assert.match(src, /Math\.ceil\(\(p \/ 100\) \* sorted\.length\)/);
});

test('analytics hydrates progressively and hydration is measured (#694)', () => {
  const src = read('components', 'Analytics.tsx');
  assert.match(src, /dynamic\(/);
  assert.match(src, /ssr: false/);
  assert.match(src, /<Suspense/);
  assert.match(src, /measureHydration\(\)/);
});

test('compression is enabled and documented (#695)', () => {
  assert.match(read('next.config.js'), /compress: true/);
  const doc = read('docs', 'compression.md');
  assert.match(doc, /brotli_comp_level 11/);
  assert.match(doc, /gzip_comp_level 6/);
});
