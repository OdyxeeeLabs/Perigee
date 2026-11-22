# Visual regression for accessibility states — issue #707

## Status: configuration and spec only

`@playwright/test` is **not in `package.json` or `yarn.lock`**. This is the
same class of gap #705 found for `@axe-core/react`: `playwright.config.ts` has
imported it since WEB-20 (#106) without declaring it, so the existing e2e
workflow cannot resolve a runner either. Regenerating the lockfile is out of
scope for this PR — CI installs with `--immutable`, so a partial update breaks
every other job.

To make it run:

```bash
yarn add -D @playwright/test
yarn exec playwright install --with-deps chromium
yarn playwright.visual:update          # record the baselines
yarn playwright.visual                 # compare against them
```

Then set `VISUAL_REGRESSION_REQUIRED=true` in the repository variables to make
the workflow blocking. Until then the `gate` job reports it and skips.

**No new package is required for the screenshot comparison itself.**
`toHaveScreenshot` ships with the Playwright runner, so this suite needs only
the one dependency that is already missing.

## What this protects, and why it is not about looks

The accessibility regressions that survive code review are the ones you cannot
see in a diff:

| Regression | What it looks like |
|---|---|
| A focus ring removed by a Tailwind refactor (`focus:outline-none` with no replacement) | Identical until something is focused |
| A contrast fix reverted, or a token re-tuned for one theme and not the other | Identical in the other theme |
| A `forced-colors` or `data-contrast` override dropped | Only under emulation you do not run locally |
| An `aria-hidden` wrapper swallowing a focusable control | Invisible, and focus lands nowhere |
| RTL mirroring removed from directional icons | Only in `dir="rtl"` |

So the suite is organised around **states** — default, focus-visible,
forced-colors, manual high contrast, RTL, navigation layers, degraded network —
rather than around pages. A screenshot of a page in its default state catches
none of the above.

## Separate config, deliberately

`playwright.visual.config.ts` rather than extending `playwright.config.ts`,
because the two suites want opposite things:

- Functional E2E runs against a live backend and real chain data, so its
  results are legitimately non-deterministic.
- Visual must be the opposite: network stubbed, clock frozen, motion disabled.
  A visual suite running against live data produces baselines nobody can
  review.

One config with one set of settings cannot do both, and sharing settings is how
a baseline ends up recording a spinner mid-rotation.

## Determinism

Every step here exists because a non-deterministic screenshot gets adopted as
the new baseline and then fails the next run:

| Setting | Without it |
|---|---|
| `page.clock.install()` | "3 minutes ago" differs between runs |
| `animations: "disabled"` + injected CSS | A CSS transition is captured mid-flight |
| `caret: "hide"` | Text caret blinks between baseline and comparison |
| `reducedMotion: "reduce"` | The loading spinner is a coin flip |
| `locale`/`timezoneId` pinned | Date and number formatting shifts |
| `**/api/**` stubbed | Real data, real timestamps, real async states |

`scale: "css"` keeps a `deviceScaleFactor` change from resampling every
baseline.

## Platform-specific baselines

```ts
snapshotPathTemplate: "{testDir}/__screenshots__/{projectName}/{platform}/{arg}{ext}"
```

Chromium on Linux does not render identically to Chromium on macOS — font
rasterisation and subpixel antialiasing differ. With a single `snapshots/`
directory, every machine that did not record the baseline fails, and the suite
gets disabled within a week. Splitting by platform means a Linux CI run
compares only against Linux baselines.

## Thresholds

A single global `maxDiffPixelRatio` hides more than it catches, so the default
is deliberately loose (1%) and the exceptions are tightened at the call site:

- `api-down.png` uses 2%: it shows a cached timestamp that legitimately varies.
- Everything else stays at the default, and any test needing more than that
  should be investigating a real nondeterminism source, not widening the
  threshold.

If a diff cannot be explained, the answer is to find the nondeterminism, not to
raise the number. A threshold raised until it stops firing is a disabled check.

## Projects

| Project | Why |
|---|---|
| `chromium-desktop` | 1440×900 baseline |
| `chromium-mobile` | NavDrawer and the command palette only exist below `md` |
| `dark` | Contrast is a per-background property; a token can pass on one and fail on the other |

## Onboarding and small-screen baselines

`e2e/visual/visual.spec.ts` records three manager-onboarding states — initial,
validation-error and completed — and data views at a 360px phone and a 768px
tablet width. The onboarding fixture injects a fake `window.freighter`, seeds
`perigee_wallet_*` storage and stubs `/managers/status/*` and
`/managers/register` with fixed records. The validation-error state fills the
name with whitespace so the browser's native `required` check does not pre-empt
the app's own error. Portfolio fixtures pin `Math.random` and `Date.now` to the
same instant as `page.clock`, so the demo data is identical on every run. No
baseline contains a live timestamp or a network-loaded response.

Small-screen diagrams and tables use a shared, keyboard-focusable
`.responsive-scroll-region` so a narrow viewport produces an intentional scroll
rather than clipped labels; key columns stay sticky and chart values are also
written out as text.

## What still needs a human

A screenshot proves pixels are stable, not that the UI is usable. Focus *order*,
announcement timing and operability need the keyboard and screen-reader passes
in `docs/accessibility/audit-suite.md`. A stable screenshot of a keyboard trap
is still a keyboard trap.