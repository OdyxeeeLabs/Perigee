# Storybook for accessibility review — issue #706

## Status: configuration and stories only

`@storybook/nextjs`, `@storybook/react`, `@storybook/addon-a11y`,
`@storybook/addon-essentials` and `@storybook/addon-themes` are **not in
`package.json` or `yarn.lock`**. Adding them is a dependency change, and the
lockfile is immutable in CI, so it is deliberately not part of this PR.

To make it run:

```bash
yarn add -D @storybook/nextjs @storybook/react @storybook/addon-a11y \
  @storybook/addon-essentials @storybook/addon-themes
yarn storybook          # http://localhost:6006
yarn build-storybook
```

Everything here is written as real configuration rather than as a placeholder,
so installing the dependencies is the only remaining step. Until then it is
inert. The `declared-tooling` job added in #705 is what stops this from
becoming a silent gap again.

## Why Storybook, for an accessibility issue

Because the defects that matter here are invisible in a diff:

- A live region's markup looks the same whether or not it stays mounted when
  the message arrives, and that is the whole bug (see #704).
- A spinner's reduced-motion and high-contrast behaviour only exists when the
  OS setting is on.
- An icon-only button's accessible name comes from an `aria-label` on a line
  someone will read without evaluating.
- Focus styling is invisible until something receives focus.

Axe running in the Storybook panel puts those next to each other, with
`prefers-reduced-motion`, contrast and text direction as toolbar toggles
rather than separate stories.

## What is configured

`web/.storybook/main.ts`

- `addon-a11y` first in the addon list — it is the reason for the setup.
- `builder-nextjs` pointed at the app's own `next.config.js`, so path aliases
  and `next-intl` resolve in stories. Without it every story using
  `useTranslations()` throws, which excludes exactly the components whose a11y
  is hardest to review (NavDrawer, the command palette, error summaries).
- `reactDocgen: "react-docgen-typescript"` so the a11y panel keeps the prop
  descriptions that explain *why* a story passes a label.

`web/.storybook/preview.tsx`

- **Global `next-intl` messages**, for the reason above.
- **`a11y.test: "todo"` rather than `"error"`.** The repo has known
  violations; a panel pinned red on first run gets ignored rather than fixed.
  `button-name` is escalated to a hard error because there is no acceptable
  exception for an icon-only control.
- **Backgrounds for light, dark and high contrast.** Contrast is a
  per-background property: a token that passes on the dark surface can fail on
  the light one.
- **Global toolbar toggles** for contrast (#701), reduced motion (#702) and
  direction (#699), implemented as decorators so every story gets every
  variant. As separate stories they would double the count and invite one
  variant to be checked and the other skipped.
- Reduced motion is applied via a CSS custom property rather than the media
  query, because a Storybook toolbar cannot toggle `prefers-reduced-motion`.
  That is a real limitation: a story that depends on the media query directly
  will not react to the toggle.

## Stories

| Story | What it is for |
|---|---|
| `Accessibility/LiveRegion` → `Idle` | The state a region must be in *before* its first message |
| → `AnalysisRunning` / `AnalysisComplete` | Polite, non-urgent updates |
| → `AnalysisFailed` | Assertive; kept separate so a "Saved!" cannot end up interrupting a screen reader mid-sentence |
| → `Paired` | Two regions on one page, as `_app.tsx` mounts them |
| `Components/Spinner` → `Decorative` | `aria-hidden`; correct only inside a named control |
| → `WithLabel` | The correct default: `role="status"` plus text |
| → `ReducedMotionHighContrast` | Where the ring's low-opacity track disappears |
| → `StatusColors` | danger/success must not converge at low lightness |
| → `UnlabelledButtonAntiPattern` | Deliberately failing `button-name`, so it cannot be copied |

## CI

The a11y story tests are not wired into CI here, because the runner is the
addon and the addon is not installed. Once it is, add to
`.github/workflows/a11y-audit.yml`:

```yaml
  storybook-a11y:
    name: Storybook a11y violations
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: yarn install --immutable
      - run: yarn build-storybook
      - uses: actions/upload-artifact@v4
        with:
          name: storybook-static
          path: web/storybook-static
      - name: Run the a11y test-runner
        working-directory: web
        run: yarn test-storybook --maxWorkers=2
```

`addon-a11y`'s `test-runner` fails on violations and, with
`requireSemantics: true`, on a11y *semantic* errors — an image without alt text,
a form control without a label. That is a stronger gate than the panel alone,
so it is worth gating on rather than only displaying.

## Still manual

Storybook renders components in isolation. It cannot tell you whether the page
around them is navigable, whether focus order survives a route change, or
whether a live region is announced at the right moment in a real flow. Those
need the end-to-end pass — see `docs/accessibility/audit-suite.md` for the
keyboard and screen-reader checklists. Visual baselines are
`docs/accessibility/visual-regression.md` (#707).