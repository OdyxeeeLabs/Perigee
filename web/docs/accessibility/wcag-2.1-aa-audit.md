# WCAG 2.1 AA audit

Issue #696. Scope: the Next.js front end in `web/`. Automated checks are in
`web/lib/a11y/contrast.test.cjs`; this document records the manual review, the
fixes that came out of it, and the exceptions with a remediation plan.

Audit method: source review of every interactive component under `web/components`
and `web/pages`, keyboard walk-through of the primary flows, plus the token
ratios computed by `web/lib/a11y/contrast.ts`.

## Findings and fixes

### 2.4.7 Focus Visible (A) — was failing, fixed

The app shipped **no focus styling at all**. `styles/globals.css` and
`styles/variables.css` contained no `:focus` or `:focus-visible` rule, so a
keyboard user had no indicator of position anywhere in the product.

- `web/styles/focus.css` defines one global `:focus-visible` ring: 2px solid,
  2px offset, drawn with `outline` so it follows border radius and is not
  clipped by `overflow: hidden`. Links additionally get an underline, so focus
  is never signalled by colour alone (1.4.1).
- Ring colours are tokens (`--focus-ring-color`, `--focus-ring-color-dark`,
  `--focus-ring-width`, `--focus-ring-offset`) rather than literals, so the
  forced-colors work in #701 can override them in one place.
- `contrast.test.cjs` asserts both ring colours clear 3:1 against the light
  and dark backgrounds, so the indicator cannot silently regress below the
  non-text minimum.

### 2.4.1 Bypass Blocks (A) — was failing, fixed

`pages/_app.tsx` renders the command launcher, two network banners, an error
boundary and the analytics layer **above** the page content, so reaching the
content took a variable number of Tab presses.

- `web/components/common/SkipLink/SkipLink.tsx` renders a visually hidden
  "Skip to main content" link as the first focusable element, shown on focus.
- The click handler moves **focus**, not just scroll, to
  `<main id="main-content" tabIndex={-1}>`; `tabIndex={-1}` is what allows a
  container to receive programmatic focus.
- Wired in both routers (`pages/_app.tsx` and the App Router foundation in
  `app/layout.tsx`).

### 1.4.3 Contrast (Minimum) / 1.4.11 Non-text Contrast (AA) — pass, now enforced

`styles/variables.css` already carried contrast-corrected tokens with the
ratios in comments (#492). Those comments are claims, so:

- `web/lib/a11y/contrast.ts` implements the WCAG relative-luminance formula
  and the contrast ratio, and `contrast.test.cjs` **recomputes** the shipped
  tokens: 4.5:1 minimum for `--color-text`, `--color-text-muted`,
  `--color-primary`, `--color-secondary`, `--color-danger` against
  `--color-background`, and 3:1 for `--color-border`,
  `--color-border-subtle` and the focus ring.

### 1.3.1 Info and Relationships (A) — pass, with one gap closed here

- `components/DynamicForm.tsx` already pairs every control with a `<label
  htmlFor>`, marks invalid fields with `aria-invalid` and points them at the
  error node with `aria-describedby`.
- Landmarks: the page content is now wrapped in `<main>` (2.4.1 above), and
  both network banners already expose `role="alert"`/`status`.
- **Remaining gap, filed for the next pass:** `components/WasmUpload.tsx`
  has no `<label>`/`aria-describedby` wiring for its file input. It is tracked
  in the remediation plan below rather than fixed here, because the file
  control is rendered by `react-dropzone` and the fix needs the dropzone's
  render-prop path verified in a browser.

### 4.1.2 Name, Role, Value (A) — pass

`CommandPalette` exposes `aria-haspopup`/`aria-expanded` on its trigger,
`NavDrawer` is `role="dialog" aria-modal="true" aria-labelledby`, and the
shared `LiveRegion` primitives are used for dynamic status.

### 2.1.1 Keyboard / 2.4.3 Focus Order / 2.4.7 — see #698

Keyboard operability, roving tabindex and the focus-trap/Escape work are
covered by issue #698 in the same branch.

### 4.1.3 Status Messages (AA) — see #697

Live-region announcements and the verbosity preference are covered by #697.

### 1.4.10 Reflow / 1.4.12 Text Spacing — pass

Layout uses fluid units and `rem`-based spacing; no fixed-height text
containers were found in the reviewed components.

## Exceptions and remediation plan

| # | Exception | Impact | Plan | Owner |
|---|---|---|---|---|
| 1 | `WasmUpload` file input has no programmatic label or description | Screen-reader users cannot identify the upload control | Add `aria-label`/`aria-describedby` through the dropzone render prop; verify in the browser | next a11y pass |
| 2 | Mermaid diagrams render as SVG without a text alternative | Diagram content is unavailable non-visually | Emit a visually hidden text summary of node/edge counts and labels alongside the chart | with #706 (component docs) |
| 3 | Third-party `cmdk` and wallet-kit widgets are not audited | Unknown conformance | Record the versions in `docs/accessibility/vendor.md` and re-audit on upgrade | next a11y pass |
| 4 | Colour is still the primary signal in the status pills | Users who cannot distinguish hues | Add icon + text to each status pill; contrast is already AA | next a11y pass |

## Re-running the checks

```bash
# token ratios, verbosity, roving tabindex, RTL
yarn workspace Perigee-web test

# the whole suite
yarn workspace Perigee-web run typecheck
yarn workspace Perigee-web lint
```

`contrast.test.cjs` reads `styles/variables.css` directly, so changing a
token without checking its ratio fails the test rather than silently
regressing.
