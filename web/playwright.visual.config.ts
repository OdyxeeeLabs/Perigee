import { defineConfig, devices } from "@playwright/test";

/**
 * Visual regression configuration — issue #707.
 *
 * Kept as a *separate* config file rather than added to `playwright.config.ts`.
 * The two suites want different things and merging them makes both worse:
 *
 * - The functional E2E suite runs against a live backend and mainnet/testnet
 *   data, so its results are legitimately non-deterministic.
 * - The visual suite must be the opposite: no network, seeded data, frozen
 *   time, so a pixel difference means a real change and nothing else.
 *
 * A single config with one set of settings cannot do both, and a visual suite
 * that runs against live data produces baselines nobody can review.
 *
 * ## This cannot run yet
 *
 * `@playwright/test` is **not in `package.json` or `yarn.lock`** — the same
 * problem as `@axe-core/react` in #705, and `playwright.config.ts` already
 * imports it without declaring it. So does the existing e2e workflow; this
 * file adds nothing new to that problem and does not fix it, because fixing
 * it means regenerating the lockfile.
 *
 * After `yarn add -D @playwright/test` and `yarn exec playwright install
 * --with-deps chromium`:
 *
 * ```bash
 * yarn playwright.visual            # run against the committed baselines
 * yarn playwright.visual:update     # re-record baselines after a review
 * ```
 *
 * @see docs/accessibility/visual-regression.md
 */
export default defineConfig({
  testDir: "./e2e/visual",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 2 : undefined,

  reporter: process.env.CI
    ? [["github"], ["html", { open: "never", outputFolder: "playwright-report" }]]
    : [["list"], ["html", { open: "never", outputFolder: "playwright-report" }]],

  /**
   * Per-platform directories. Chromium on Linux does not render identically to
   * Chromium on macOS (font rasterisation, subpixel antialiasing), so a
   * single `snapshots/` directory produces failures on every machine that is
   * not the one the baseline was recorded on — which gets the whole thing
   * switched off within a week.
   */
  snapshotPathTemplate: "{testDir}/__screenshots__/{projectName}/{platform}/{arg}{ext}",

  expect: {
    toHaveScreenshot: {
      /**
       * A single global pixel ratio hides more than it catches. Text-heavy
       * views need a tighter budget than a chart; per-test overrides use
       * `maxDiffPixelRatio`, and this is the deliberately loose default.
       */
      maxDiffPixelRatio: 0.01,
      /**
       * Font rendering is the usual source of a 1–2 pixel edge difference
       * that is not a change. Hiding subpixel differences is what
       * `caret: "hide"` and `animations: "disabled"` buy here; the animations
       * setting also stops a CSS transition from being captured mid-flight
       * and recorded as the new baseline.
       */
      animations: "disabled",
      caret: "hide",
      scale: "css",
    },
    toMatchSnapshot: { maxDiffPixelRatio: 0.01 },
  },

  use: {
    baseURL: process.env.NEXT_PUBLIC_APP_URL ?? "http://127.0.0.1:3000",
    /**
     * Determinism over realism. `prefers-reduced-motion` is reduced because a
     * screenshot taken while a spinner is mid-rotation is a coin flip, and
     * this is exactly the failure mode that makes teams disable baselines.
     */
    reducedMotion: "reduce",
    /**
     * Deterministic fonts. Without a pinned font the text layout depends on
     * whatever the CI image happens to have, and the baseline is only valid
     * for the image it was recorded on.
     */
    timezoneId: "UTC",
    locale: "en-GB",
    colorScheme: "light",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "off",
  },

  projects: [
    {
      name: "chromium-desktop",
      use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } },
    },
    {
      name: "chromium-mobile",
      // NavDrawer and the command palette only exist below `md`, so the
      // mobile viewport is where the accessibility-critical surfaces are.
      use: { ...devices["Pixel 7"] },
    },
    {
      name: "dark",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1440, height: 900 },
        colorScheme: "dark",
      },
    },
  ],
});