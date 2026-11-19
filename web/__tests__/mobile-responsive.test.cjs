const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

test('mobile responsiveness layout audit (< 640px)', (t) => {
  const indexPage = fs.readFileSync(path.join(__dirname, '../pages/index.tsx'), 'utf8');
  assert.ok(
    indexPage.includes('flex flex-col sm:flex-row'),
    'Header on index.tsx adapts to vertical on mobile (< 640px)'
  );

  const skeleton = fs.readFileSync(path.join(__dirname, '../components/ResultViewerSkeleton.tsx'), 'utf8');
  assert.ok(
    skeleton.includes('grid-cols-4 sm:grid-cols-6') || skeleton.includes('grid-cols-1 sm:grid-cols-2 md:grid-cols-4'),
    'Skeleton grids adapt on mobile screens'
  );

  const heatmap = fs.readFileSync(path.join(__dirname, '../components/ResourceHeatmap.tsx'), 'utf8');
  assert.ok(
    heatmap.includes('grid-cols-2 sm:grid-cols-4'),
    'Heatmap hotspot grid uses 2 columns on mobile'
  );
  assert.ok(
    heatmap.includes('grid-cols-1 sm:grid-cols-2 md:grid-cols-4'),
    'Heatmap footer summary bar wraps to 1 column on extra small screens'
  );
});

test('data tables scroll intentionally on small screens with actions reachable', () => {
  const table = fs.readFileSync(path.join(__dirname, '../components/ui/DataTable.tsx'), 'utf8');
  assert.ok(
    table.includes('responsive-scroll-region'),
    'Table uses the shared, focusable scroll-region utility'
  );
  assert.ok(table.includes('role="region"'), 'Scroll container has an accessible region role');
  assert.ok(table.includes('tabIndex={0}'), 'Scroll container is keyboard focusable');
  assert.ok(table.includes('sticky left-0 z-10'), 'First (key) column stays visible while scrolling');
  assert.ok(table.includes('min-w-[36rem]'), 'Columns keep a readable minimum width');
  assert.ok(
    table.includes('flex flex-wrap items-center justify-between gap-2'),
    'Pagination controls wrap instead of overflowing'
  );
  assert.ok(table.includes('min-h-[32px] min-w-[32px]'), 'Pagination targets meet touch-size guidance');

  const css = fs.readFileSync(path.join(__dirname, '../styles/globals.css'), 'utf8');
  assert.ok(
    css.includes('.responsive-scroll-region {'),
    'Shared scroll-region utility is defined globally'
  );
});

test('chart values are text-labelled, not colour-only', () => {
  const history = fs.readFileSync(
    path.join(__dirname, '../components/vault/HistoricalPerformanceChart.tsx'),
    'utf8'
  );
  assert.ok(history.includes('formatNumber('), 'Chart values use the shared locale helpers');
  assert.ok(history.includes('First:'), 'Starting value is written out');
  assert.ok(history.includes('Latest:'), 'Ending value is written out');
  assert.ok(history.includes('Trend:'), 'Trend direction is written out, not just coloured');

  const gas = fs.readFileSync(path.join(__dirname, '../components/GasUsageChart.tsx'), 'utf8');
  assert.ok(gas.includes('flex flex-wrap'), 'Gas chart rows wrap on narrow screens');
  assert.ok(gas.includes('role="img"'), 'Bars expose a text alternative');
  assert.ok(gas.includes('${metric.label} simulated:'), 'Simulated bar is labelled with its value');

  const diagram = fs.readFileSync(
    path.join(__dirname, '../components/strategy/StrategyPhaseDiagram.tsx'),
    'utf8'
  );
  assert.ok(
    diagram.includes('responsive-scroll-region'),
    'Mermaid diagram has an intentional, keyboard-reachable scroll region'
  );
});

test('visual suite captures narrow phone and tablet data views', () => {
  const visual = fs.readFileSync(path.join(__dirname, '../e2e/visual/visual.spec.ts'), 'utf8');
  assert.ok(visual.includes('width: 360'), 'Narrow phone width is covered');
  assert.ok(visual.includes('width: 768'), 'Tablet width is covered');
  assert.ok(visual.includes('small-screen-analytics-'), 'Table view has a baseline');
  assert.ok(visual.includes('small-screen-portfolio-'), 'Chart view has a baseline');
});
