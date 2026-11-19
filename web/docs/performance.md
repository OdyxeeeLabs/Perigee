# Performance notes

## Re-renders (#692)

- Purely presentational components that receive stable props are wrapped in
  `React.memo`: `Badge`, `Spinner`, `NutritionLabel`, `GasUsageChart`,
  `GasGolfingSuggestionsTable`, `ResourceHeatmap` and `StrategyPhaseTable`.
- `react-hooks/exhaustive-deps` is enforced as an **error** in
  `.eslintrc.json` (via `next/core-web-vitals`), so memoised callbacks and
  values cannot silently go stale.
- Wrap suspicious subtrees in `<RenderProfiler id="...">` to report renders
  slower than the 16ms budget (console in development, anonymous
  `slow_render` telemetry in production). Use the React DevTools Profiler to
  find the cause.
- When passing handlers or derived objects to a memoised child, stabilise them
  with `useCallback` / `useMemo`, otherwise `React.memo` is defeated.

## Web vitals (#693)

See `lib/webVitals.ts` and `pages/api/vitals.ts`. Metrics are sent only when
the user consented to performance telemetry. `GET /api/vitals` returns
p50/p75/p95 per metric, per page, device class and connection type, and the
alert state against the thresholds in `VITAL_THRESHOLDS`. The dashboard lives
at `/admin/vitals`.

## Hydration (#694)

Header/navigation, command palette, wallet connection and primary CTAs stay in
the main bundle and hydrate first. The analytics consent banner (`components/Analytics.tsx`) and other
non-critical widgets are loaded with `next/dynamic` and mounted inside
`Suspense` after the first paint. `lib/hydrationMetrics.ts` records
`perigee:hydration` (navigation start to hydration complete) as a User Timing
measure and a `hydration_time` telemetry event.

## Route JavaScript budgets (#842 / WEB-41)

`route-budgets.json` records the maximum raw, uncompressed JavaScript size for
the home (`/`), vault (`/vault/[id]`) and analytics (`/analytics`) route chunks.
Budgets are measured from Next's own build output (`build-manifest.json` and
`app-build-manifest.json`); no timing measurements are involved, so a breach
is reproducible. The home budget accounts for the result viewer being loaded
with `next/dynamic` from `pages/index.tsx` — importing it statically would
breach it again.

Run `yarn budget:routes` after `yarn build` (CI does the same in
`.github/workflows/web-route-budget.yml`). A failure prints the route and each
of its chunks with their byte sizes. To raise a budget, change
`route-budgets.json` deliberately in the same PR that justifies the increase.

## Read-only refresh cadence (#843)

The Display settings offer 15 s, 60 s and 5 min for background reads of
analytics, portfolio and network status; the default is one minute (60 s).
Transaction confirmations, security notices and explicit Retry buttons are not
delayed by it. The preference lives in `lib/settings.ts` and is persisted with
the existing settings store.

## Compression (#695)

See `docs/compression.md`.
