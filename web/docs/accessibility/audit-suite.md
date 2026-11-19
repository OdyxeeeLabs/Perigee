# Automated accessibility audit — issue #705

## The finding that motivated this

`pages/_app.tsx` has imported `@axe-core/react` since WEB-24 (#110):

```ts
const axe = await import("@axe-core/react");
```

**`@axe-core/react` is in neither `package.json` nor `yarn.lock`.** The
`import()` rejects on every dev boot, and the rejection was swallowed:

```ts
initAxe().catch(() => {
  // axe-core is a dev-only optional dependency; swallow import errors
});
```

So the browser-console audit has never run — while the code, the comment and
the PR all said it had. `axe-core` *is* in the lockfile, but only as a
dependency of `eslint-plugin-jsx-a11y`, which is a different package that
cannot satisfy this import.

`@playwright/test` is missing the same way, and `web/playwright.config.ts`
exists with no runner to load it.

Silent failure is worse than absence here: a missing audit is a gap, while a
missing audit that looks present is a false assurance. Two things follow from
it, and both are in this change:

1. `initAxe` now reports the failure once per session, with the exact command
   to fix it, instead of swallowing it.
2. The `declared-tooling` CI job reports when an imported accessibility
   package is not declared in `package.json`, so the two cannot drift apart
   again. It is **advisory** for the same reason the static audit is: the gap
   exists on `main` and closing it means regenerating the lockfile, so a
   blocking check would sit red on every PR until someone did that. The
   finding appears as an annotation on the PR; set `A11Y_TOOLING_STRICT=true`
   to make it blocking once the dependencies are added.

Adding the dependencies themselves is a `package.json` + `yarn.lock` change
and is deliberately **not** part of this PR (the lockfile is immutable in CI,
and regenerating it here would break every other job).

## The scanner

`scripts/a11y/static-audit.mjs` — no dependencies, Node's standard library
only, so it runs anywhere Node runs:

```bash
node scripts/a11y/static-audit.mjs            # advisory: report, exit 0
node scripts/a11y/static-audit.mjs --json     # machine-readable
node scripts/a11y/static-audit.mjs --strict   # exit 1 on any finding
node scripts/a11y/static-audit.mjs components/  # limit the scope
```

### Rules

| Rule | Severity | Checks |
|---|---|---|
| `img-missing-alt` | error | 1.1.1 — `<img>` with no `alt` |
| `iframe-missing-title` | error | 4.1.2 — `<iframe>` with no `title` |
| `label-without-associated-control` | error | 1.3.1 / 3.3.2 — label names nothing |
| `empty-link-name` | error | 2.4.4 / 4.1.2 — icon-only link with no name |
| `click-non-interactive` | error | 2.1.1 — `onClick` with no role and no keyboard path |
| `aria-hidden-focusable` | error | 4.1.2 — focusable but hidden from AT |
| `positive-tabindex` | warning | 2.4.3 — valid only for roving tabindex |
| `outline-none-without-replacement` | warning | 2.4.7 — focus styling removed, nothing replaced it |
| `autofocus` | warning | 3.2.1 — steals focus on load |

Three details that took a false positive each to get right:

- **Comments are stripped, with line numbers preserved.** `DataTable.tsx` has
  an `@accessibility` JSDoc block that *describes* a visually hidden
  `<label>`; before stripping, the scanner reported an unassociated label in a
  file that has none. A tool that cries wolf over documentation gets switched
  off. Line comments are deliberately left alone: `//` occurs inside strings
  and URLs (`https://…`) and a regex cannot tell them apart without a
  tokenizer.
- **Opening tags are matched across lines.** Attributes wrap constantly in
  this codebase, and a per-line regex misses `<img` when `src` is on the next
  line. Attributes inside quoted strings are skipped, so a `>` in a string
  does not end the tag early.
- **`<label>` gets nesting detection.** A label that *wraps* its control is
  correctly associated and extremely common; only a control that is a
  *sibling* is the bug. Without looking at the content up to `</label>`,
  `AnalyticsBanner.tsx`'s three checkbox rows were all false positives.

Backdrops are excluded from `click-non-interactive` when they carry
`aria-hidden="true"`: a decorative overlay is hidden from assistive technology
on purpose, and its keyboard path is Escape or the close button.

## Baselined findings (6 errors, 1 warning)

Not fixed here — they are pre-existing and outside this issue's scope, but
they are recorded so `--strict` can be adopted deliberately:

| Location | Rule |
|---|---|
| `pages/managers/onboarding.tsx:180,190,202,214` | label with a sibling input, no `htmlFor` |
| `pages/vault/[id].tsx:244` | label with no associated control |
| `components/WalletModal.tsx:61` | backdrop `onClick` without `aria-hidden` |
| one `focus:outline-none` | no replacement focus style |

`WalletModal` is inconsistent with `NavDrawer`, whose equivalent backdrop
*is* marked `aria-hidden`. Fixing these five is a small, self-contained PR, and
clearing them is what makes `--strict` a reasonable gate.

## CI

`.github/workflows/a11y-audit.yml`, three jobs:

- **static-audit** — runs the scanner, publishes a Markdown summary to the
  job summary, and fails only if `STATIC_AUDIT_STRICT` is set. Advisory by
  default on purpose: a check that fails immediately on day one gets ignored
  rather than fixed.
- **a11y-unit-tests** — runs every `lib/a11y/*.test.cjs` and
  `scripts/a11y/*.test.cjs` via glob, with **no install step**. They are
  dependency-free `node:test`, so they cannot be broken by a lockfile change.
- **declared-tooling** — fails if `@axe-core/react` or `@playwright/test` is
  imported but not declared.

## What this still cannot do

A static scanner reads one line of markup, not a rendered DOM. It cannot
compute accessible names, resolve contrast, verify focus order, or see
anything rendered dynamically. That work needs `axe-core` in a real browser
(the missing dependency) plus a human. The checklist below is not optional
padding; it is the larger half of the issue.

### Manual checklist

Keyboard only, no mouse:

- [ ] Every interactive element is reachable and operable with Tab/Shift+Tab.
- [ ] Focus is always visible, including on custom controls.
- [ ] No positive `tabindex` outside a composite widget.
- [ ] Dialogs trap focus, close on Escape, and restore focus to the trigger.
- [ ] Nothing steals focus on load (no `autoFocus`).

Screen reader (VoiceOver + Safari, or NVDA + Firefox):

- [ ] Every image has a meaningful `alt`; decorative images have `alt=""`.
- [ ] Form errors are announced and associated with their fields.
- [ ] Dynamic updates (analysis, uploads, transactions) are announced once,
      not per tick.
- [ ] Landmarks and headings are navigable.

Visual:

- [ ] 200% zoom and 400% reflow without loss of content or function.
- [ ] Text contrast ≥ 4.5:1; UI component contrast ≥ 3:1.
- [ ] Windows High Contrast Mode (`forced-colors: active`).
- [ ] `prefers-reduced-motion: reduce` and `prefers-contrast: more`.
- [ ] Light and dark themes.