'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('path');

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');

test('historical performance chart exposes its data and time range', () => {
  const src = read('components/vault/HistoricalPerformanceChart.tsx');

  assert.match(src, /role="img"/);
  assert.match(src, /aria-describedby=\{descriptionId\}/);
  assert.match(src, /data-testid="historical-performance-chart-description"/);
  assert.match(
    src,
    /<table className="sr-only" data-testid="historical-performance-chart-data">/,
  );
  assert.match(src, /<caption>/);
  assert.match(src, /<th scope="col">Date<\/th>/);
  assert.match(
    src,
    /<th scope="row">\{formatDate\(point\.timestamp\)\}<\/th>/,
  );
  assert.match(src, /history\.map\(\(point, index\)/);
  assert.match(src, /startTime/);
  assert.match(src, /endTime/);
});

test('strategy diagram keeps its table alternative visible to assistive tech', () => {
  const diagram = read('components/strategy/StrategyPhaseDiagram.tsx');
  const table = read('components/strategy/StrategyPhaseTable.tsx');

  assert.match(diagram, /<ErrorBoundary/);
  assert.match(diagram, /<Mermaid \/>/);
  assert.match(diagram, /<StrategyPhaseTable \/>/);
  assert.match(diagram, /phaseSummary/);
  assert.match(
    diagram,
    /data-testid="strategy-phase-state-summary"/,
  );
  assert.match(diagram, /Time range:/);
  assert.match(table, /caption="Strategy phases:/);
});

test('gas usage chart exposes comparable values', () => {
  const src = read('components/GasUsageChart.tsx');

  assert.match(src, /data-testid="gas-usage-chart-description"/);
  assert.match(src, /role="img"/);
  assert.match(src, /aria-describedby=\{summaryId\}/);
  assert.match(src, /<table className="sr-only" data-testid="gas-usage-chart-data">/);
  assert.match(src, /<caption>/);
  assert.match(src, /<th scope="row">\{metric\.label\}<\/th>/);
  assert.match(src, /\{formatCompact\(metric\.simulated\)\}/);
  assert.match(src, /\{formatCompact\(metric\.average\)\}/);
  assert.match(src, /cost_stroops/);
  assert.match(src, /aria-hidden="true"/);
});

test('form and transaction state announce through one shared provider', () => {
  const form = read('components/DynamicForm.tsx');
  const interaction = read('components/ContractInteraction.tsx');
  const settings = read('components/settings/SettingsPage.tsx');

  assert.match(form, /useAnnouncer/);
  assert.match(form, /announceStatus\(t\("dynamicForm\.simulating"\)/);
  assert.match(form, /announceAlert\(/);
  assert.match(form, /noValidate/);
  assert.match(form, /aria-describedby=\{describedBy\(/);
  assert.match(form, /aria-invalid=/);
  assert.doesNotMatch(form, /role="alert"/);
  assert.doesNotMatch(form, /aria-live=/);

  assert.match(interaction, /useAnnouncer/);
  assert.match(interaction, /useState/);
  assert.match(interaction, /outcome/);
  assert.match(interaction, /announceStatus\(/);
  assert.match(interaction, /announceAlert\(/);
  assert.match(interaction, /await submit\(inputs\)/);

  assert.match(settings, /useAnnouncer/);
  assert.match(settings, /announceStatus\("Settings reset to defaults\."\)/);
});

test('overlays restore focus to a stable target when the opener is gone', () => {
  const helper = read('lib/a11y/focus.ts');
  const hook = read('hooks/useFocusTrap.ts');
  const docs = read('docs/accessibility/focus-management.md');

  for (const file of [
    'components/WalletModal.tsx',
    'components/NavDrawer.tsx',
    'components/vault/VaultDetailModal.tsx',
  ]) {
    assert.match(read(file), /useFocusTrap/, file);
  }

  assert.match(helper, /FocusRestoreCandidate/);
  assert.match(helper, /pickFocusRestoreTarget/);
  assert.match(hook, /pickFocusRestoreTarget/);
  assert.match(hook, /getElementById\("main-content"\)/);
  assert.match(hook, /document\.addEventListener\("keydown"/);
  assert.match(docs, /#main-content/);
});

test('data table exposes sort, filter, pagination and result changes', () => {
  const src = read('components/ui/DataTable.tsx');

  assert.match(src, /caption\?: React\.ReactNode/);
  assert.match(src, /<caption className="sr-only">/);
  assert.match(src, /aria-sort=/);
  assert.match(src, /type="button"/);
  assert.match(src, /focus-visible:ring-2/);
  assert.match(src, /data-testid="data-table-result-count"/);
  assert.match(src, /<LiveStatus/);
  assert.match(src, /resultsRef/);
  assert.match(src, /aria-label="Next page"/);
});

test('vault rows support roving arrow navigation and visible focus', () => {
  const row = read('components/VaultRow.tsx');
  const list = read('components/vault/VaultList.tsx');
  const css = read('styles/focus.css');

  assert.match(row, /rowRef\?: \(node: HTMLDivElement \| null\) => void/);
  assert.match(row, /onKeyDown\?:/);
  assert.match(row, /tabIndex/);
  assert.match(row, /event\.key === "Enter" \|\| event\.key === " "/);
  assert.match(row, /focus-visible:ring-2/);

  assert.match(list, /useRovingTabIndex/);
  assert.match(list, /getItemProps/);
  assert.match(list, /nextRovingIndex/);
  assert.match(list, /rowVirtualizer\.scrollToIndex/);
  assert.match(
    list,
    /tabIndex=\{activeIndex === index \? 0 : -1\}/,
  );
  assert.match(list, /event\.key === "ArrowDown"/);
  assert.match(list, /event\.key === "Home"/);

  assert.match(css, /:focus-visible/);
});
