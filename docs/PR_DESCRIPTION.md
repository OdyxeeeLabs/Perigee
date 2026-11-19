# docs(web): generate a versioned component-library docs site from source

**Closes #709** · `web.md` § Developer Experience & Tooling (#34)

| | |
| --- | --- |
| **Type** | Feature — developer experience / documentation tooling |
| **Area** | `web/` (component library, docs tooling, CI) |
| **Base** | `OdyxeeeLabs/Perigee:main` @ `4834574` |
| **Head** | `Yinklekay:docs/component-library-docs-709` (fork) |
| **Size** | 60 files, **+9,293 / −9** — of which ~1,000 lines are generated markdown, ~2,600 lines generator + tests, 185 lines of JSDoc-only component edits, 26 generated site files |
| **New dependencies** | **None.** `package-lock.json` is untouched |
| **Behavioural risk** | **None** — no runtime code changed (component diff is comments) |
| **CI** | `Component Library Docs` → **pass** (run [38768582559](https://github.com/Yinklekay/Perigee/actions/runs/38768582559)), artifacts uploaded |

---

## TL;DR

The component library now documents itself. `npm run docs:build` reads the
component sources with the TypeScript compiler API and emits **committed
markdown** (props tables, usage examples, composition patterns, accessibility
notes) plus a **static docs site** published from `web/public/component-docs`. A
curated `breaking-changes.json` turns real API differences into a generated
migration guide with before/after props tables, and `npm run docs:release`
freezes a browsable snapshot per release, versioned with `web/package.json`.

Nothing is documented twice: 7 pages / 13 exports are derived from TypeScript +
JSDoc, and `npm run docs:check` fails CI the moment the committed docs drift
from the source.

```bash
cd web
npm run docs:build     # regenerate markdown + docs site (committed output)
npm run docs:check     # CI guard: fails if anything is stale
npm run docs:release   # freeze + publish this version's snapshot
npm run docs:serve     # local preview at http://localhost:4173
```

---

## How the issue's acceptance criteria are met

| Issue requirement | Where it lives | Proof |
| --- | --- | --- |
| *Auto-generate **props tables with types*** | `scripts/docs/lib/extract.mjs` → `scripts/docs/lib/render.mjs` | `docs/components/button.md`: `variant` renders the full union `"default" \| "destructive" \| …`; `data-table.md` renders `ColumnDef<T>[]`, `(row: T) => React.ReactNode`, and a nested `ColumnDef` table |
| *…with **descriptions*** | JSDoc on props-interface members (precedence: component → props type → file-level block) | `spinner.md` rows: "Controls the outer diameter of the spinner ring…"; `data-table.md`: "Message shown when the filtered result set is empty." |
| *…and **defaults** from TypeScript/JSDoc* | destructuring defaults, incl. aliased bindings and `@default` | `size` → `"default"`, `pageSize` → `0`, `filterPlaceholder` → `'Filter…'`, `asChild` → `false` (from `asChild: _asChild = false`) |
| *Usage examples* | `@example` tags (+ legacy indented `Usage:` blocks auto-fenced) | A `## Usage` section with runnable `tsx` on every page; asserted per page by the test suite |
| *Composition patterns* | `@composition` tags for compound components + registry `PATTERNS` for cross-component recipes | `card.md` documents all six parts and their spacing contract; index has 4 patterns (loading button, table-in-card, status pill in a table cell, announcing action results) |
| *Accessibility notes* | `@accessibility` per component + a shared baseline on the index | `button.md` (keyboard/focus/icon-only labelling), `spinner.md` (decorative vs `role="status"`), `data-table.md` (labelled filter, `aria-sort`, pagination labels), `live-region.md` (polite vs assertive), `badge.md` (contrast audit) |
| *Migration guides for breaking changes* | `scripts/docs/breaking-changes.json` + generator | `migrations.md`: two releases, before/after props tables injected from the **actual** components (`CommonButton` vs `Button` variant sets), 8 call-site rewrites, codemod, deprecation process |
| *Host on **dedicated docs site*** | `scripts/docs/lib/{markdown,site}.mjs`, `.github/workflows/component-docs.yml` | Static site: 12 HTML pages + 2 assets + search index; served by the app from `public/component-docs`, previewable with `docs:serve`, deployed to GitHub Pages on release |
| *Version with releases* | `docs/components/versions.json`, `versions/<v>.json`, `public/component-docs/versions/v<v>/` | `docs:release` freezes snapshots; the site shows a version selector (current → live docs, older → snapshot); `docs:check` treats snapshots as immutable |
| *Run CI locally, fix failures* | see **Verification** | 155/155 tests, `docs:check` green, no new lint/type errors; pre-existing failures reproduced on pristine `main` |
| *Draft a comprehensive PR description* | this document (`docs/PR_DESCRIPTION.md`) | — |
| *Open a quality PR referencing the issue* | this PR | **Closes #709** |

---

## Why this approach

`web.md § Developer Experience & Tooling` asks for documented props, examples,
accessibility guidance and migration paths. Today there is nothing: the
primitives in `web/components/ui/` have inconsistent JSDoc (`Badge` is the only
one with an `@example`), the legacy `CommonButton` is indistinguishable from the
newer `Button` without reading both files, and the API changes already shipped
(WEB-27 / #113 standardisation, WEB-38 / #492 contrast pass) are recorded
nowhere.

Hand-written docs would rot on the next refactor, so the deliverable is a
**generator plus its output**, with a drift check that turns staleness into a CI
failure instead of a code-review hope.

---

## What's in the PR

### 1. Generator — `web/scripts/docs/` (no new dependencies)

| File | Lines | Responsibility |
| --- | --- | --- |
| `lib/extract.mjs` | 566 | TypeScript AST + JSDoc → documentation model |
| `lib/render.mjs` | 410 | model → markdown |
| `lib/markdown.mjs` | 241 | markdown → accessible HTML (escaping, scoped tables, nested lists) |
| `lib/site.mjs` | 421 | markdown → static site (nav, TOC, search, version selector, CSS/JS) |
| `lib/text.mjs` | 64 | slug/id/escaping helpers |
| `registry.mjs` | 209 | hand-written: which exports to document, groups, patterns, a11y baseline |
| `generate.mjs` | 646 | CLI: build / check / release, versioning, link validation |
| `serve.mjs` | 71 | zero-dependency preview server |
| `breaking-changes.json` | — | hand-written: versioned breaking changes, mappings, codemods |

**Extraction** uses `ts.createSourceFile` — no `Program`, so a docs build never
type-checks the app. It handles every component shape in the library:

- `export function Foo(props: FooProps)` — `Spinner`, `Badge`, `DataTable`,
  `LiveStatus`, `LiveAlert`
- `export const Foo = React.forwardRef<Ref, FooProps>(…)` — `Button`, the `Card`
  family
- `export const Foo: React.FC<FooProps> = …` — `CommonButton`

…and derives: props (name, optionality, type text, JSDoc description);
defaults from destructuring (including aliased bindings) with `@default`
override; generic props types (`DataTableProps<T>` → declaration); inherited
native attributes plus the inferred host element (`button`, `span`, `div`, `h3`,
…); referenced unions (`SpinnerSize`, `SpinnerColor`, `BadgeVariant` — exported
or not) with allowed values; object-shaped types as their own props table
(`ColumnDef`); and the JSDoc tags `@example`, `@accessibility`, `@since`,
`@deprecated`, `@note`, `@default`, `@composition`, with `{@link X}` → `` `X` ``.

### 2. JSDoc on the library primitives (+185 lines, comments only)

`Button`, `Card` (all six parts), `Badge`, `Spinner`, `DataTable`, `LiveRegion`
and legacy `CommonButton` gained import-time intent, `@example`s,
`@accessibility` notes and composition guidance. `CommonButton` is now
`@deprecated` with a pointer to `Button`. **No runtime behaviour changed** — the
diff is JSDoc plus two docs-relative markdown links.

### 3. Generated artifacts (committed and reviewable)

- `web/docs/components/` — 13 files: index `README.md`, one page per component,
  `migrations.md`, `versions.md`, `manifest.json` (machine-readable model),
  `versions.json`, `versions/0.1.0.json`.
- `web/public/component-docs/` — 26 files: the static site (12 pages, CSS, JS,
  `search-index.json`, `versions.json`) plus the frozen `versions/v0.1.0/`
  snapshot.

### 4. Tests — `web/__tests__/component-docs.test.cjs` (610 lines, 62 subtests)

| Suite | Pins down |
| --- | --- |
| props tables | unions, destructuring defaults (incl. the alias regression), `@default`, required flags, generic resolution, referenced types, inherited attributes |
| page content | description/usage/composition/a11y per page, deprecation banner + migration target, index coverage |
| migration guide | release list, before/after tables generated from the real components, call-site mappings, codemods |
| docs site | every page published, `lang`/skip link/aria-labelled search/live status, exactly one `<h1>`, unique ids, scoped table headers, no `.md` links left, no dangling links, search index |
| versioning | current version tracks `package.json`, snapshots exist on disk, snapshot links resolve, release + selector exercised in an isolated root |
| drift | fails with nothing generated, passes after a build, fails on a hand-edited page (`stale`), fails on an orphaned page (`unexpected`), prunes orphans, **never deletes published snapshots** |
| units | extractor (props/defaults/tags, missing export), markdown renderer (escaping, scoping, nested lists, fences, link resolution), indented-code conversion |

### 5. CI — `.github/workflows/component-docs.yml`

- **`docs`** (PRs/pushes touching the library or docs): `npm ci` →
  `npm run docs:check` → `npm test` → upload the built site as an artifact. This
  is what makes "docs are current" a merge requirement.
- **`publish`** (GitHub release or manual dispatch): resolves the released tag,
  runs `docs:release --version … --date …`, commits the frozen snapshot back to
  the default branch and deploys the site to GitHub Pages.

Also adds `web/.eslintignore` so generated output is never linted (previously
the generated `assets/docs.js` showed up in lint output).

---

## Reviewer guide — where to look first

1. **Start with the output, not the generator** (5 min): read
   [`web/docs/components/button.md`](../web/docs/components/button.md),
   [`card.md`](../web/docs/components/card.md) (compound component),
   [`migrations.md`](../web/docs/components/migrations.md) (breaking changes) and
   the index [`README.md`](../web/docs/components/README.md). If the docs read
   well, the rest is implementation detail.
2. **Then the source of truth** (5 min): `scripts/docs/registry.mjs` (what gets
   documented) and `scripts/docs/breaking-changes.json`. These are the only files
   a contributor edits by hand.
3. **Then the two risky bits** (10 min):
   - `lib/extract.mjs` — does the precedence rule (component JSDoc → props-type
     JSDoc → file-level block) match your expectations, and are you comfortable
     with `createSourceFile` instead of a full `Program`?
   - `generate.mjs` — drift detection (`diffFiles`), snapshot immutability
     (`isVersionSnapshot`) and link validation (`validateSite`).
4. **Try to break the drift check** (2 min): edit any file under
   `web/docs/components/` and run `npm run docs:check` — it must exit 1 with
   `stale <path>`.
5. **Non-obvious things to sanity-check**: the components' JSDoc diff (comments
   only — please confirm nothing executable moved), and the new workflow's
   `permissions:` block for the Pages job.

---

## Preview the docs site locally

```bash
cd web && npm run docs:serve      # http://localhost:4173
# or, once the app builds on main again:
#   http://localhost:3000/component-docs/index.html
```

The site is plain static HTML: no framework, no client-side routing, and it
degrades gracefully — the component search, copy buttons and version selector
are progressive enhancements layered on top of fully navigable pages.

---

## Design decisions worth reviewing

1. **A generator, not Storybook.** The issue asks for docs auto-generated from
   TypeScript/JSDoc on a dedicated site. Storybook would add ~15 packages, a
   second build pipeline and a dev server to a project with a deliberately small
   dependency set; the generator is ~2.6k lines of plain ESM with **zero new
   dependencies** and produces artifacts reviewable in the diff.
2. **Markdown is the artifact; HTML is derived.** GitHub renders the committed
   markdown, CI publishes the HTML, and both come from one model — so reviewers
   read the docs in the PR instead of hunting generated HTML.
3. **Drift check, not a `prebuild` hook.** Wiring `docs:build` into
   `next build` would fail app builds on a docs typo and still wouldn't tell a
   reviewer that generated files were forgotten. `docs:check` runs in CI against
   the committed artifacts and reports missing / stale / unexpected files.
4. **Fail loudly on broken links.** Every build validates hrefs, anchors and
   markdown links before writing. This caught two real bugs during development:
   snapshot pages needed `../../` (not `../`) to reach the site root, and a
   hand-seeded `versions.json` that claimed a snapshot which didn't exist.
5. **Frozen snapshots are immutable.** `--release` writes `versions/v<version>/…`;
   ordinary builds and `docs:check` treat anything under `versions/` as
   permanent, so an old release can never be silently rewritten or pruned.
6. **Versioning source of truth.** `web/package.json#version` drives the docs
   version; `versions.json` records what has been published (date + path). An
   unreleased version appears in the selector as `(snapshot not published)` and
   is not selectable.

---

## Verification

All commands run in `web/` on the PR's head commit.

| Check | Command | Result |
| --- | --- | --- |
| **CI (GitHub Actions)** | `Component Library Docs` run `38768582559` | **pass** — `docs:check` + full test suite; `component-docs-site` artifact uploaded |
| Tests | `npm test` | **155 pass, 0 fail** (62 new subtests) |
| Docs in sync | `npm run docs:check` | exit 0 (`12 markdown, 14 site files`) |
| Docs build | `npm run docs:build` | 7 pages / 13 exports from source |
| Release flow | `npm run docs:release` | `versions/0.1.0.json` + `versions/v0.1.0/` snapshot published |
| Link validation | part of every build | 0 broken links, 0 broken anchors, 0 duplicate ids |
| Preview server | `npm run docs:serve` | `/`, `/button.html`, `/assets/docs.css` → 200; unknown path → 404 |
| Typecheck | `npx tsc --noEmit` | **no new errors** (pre-existing errors only, below) |
| Lint | `npm run lint` | **no new errors** (pre-existing errors only, below) |
| Production build | `npm run build` | fails on **pristine `main` too** (below) |

### Pre-existing failures — reproduced on an untouched `main`, not fixed here

Verified by building `main` @ `4834574` in a separate `git worktree` with the
same `node_modules`, before any of this PR's changes. These belong to the App
Router migration, the eslint config and the test script — not to the docs issue
— but they are why `main` is red today:

1. **`npm run build` fails with 3 Turbopack errors** (identical on pristine
   `main`): `App Router and Pages Router both match path: /`,
   `… both match path: /vault/[id]`, and `Can't resolve '../vault/[id]/page'`
   from `app/vaults/[id]/page.tsx`. This PR adds no routes, so it can only help
   once the coexistence hazards in `MIGRATION.md` are resolved; the docs site is
   static files under `public/component-docs/`.
2. **`npx tsc --noEmit` reports 33 errors in 11 untouched files**
   (`lib/i18n.ts`, `lib/vault-metadata.ts`, `pages/index.tsx`, `pages/_app.tsx`,
   `app/vaults/[id]/page.tsx`, `playwright.config.ts`, …). None of the files this
   PR edits appear in that output.
3. **`npm test` aborts on Node ≤ 20.** The `test` script hands `**` patterns
   straight to `node --test`, and only Node ≥ 22 expands them itself: on Node 20
   the run dies with `Could not find '…/web/lib/**/*.test.cjs'` before executing
   anything (only `__tests__/migration/appRouter.test.cjs` ever matched). The
   new CI job therefore pins `node-version: '24'`, the version the app is
   developed against. Fixing the shared script is a separate cross-cutting
   change.
4. **`npm run lint` reports 296 problems (281 errors, 15 warnings).** 273 are
   `Definition for rule '@typescript-eslint/…' was not found` on every file,
   because `.eslintrc.json` enables TypeScript rules without
   `@typescript-eslint/eslint-plugin` installed. The remaining 8 are real
   `react/no-danger` / `react-hooks/exhaustive-deps` findings in untouched files.
   The `.eslintignore` added here only excludes generated artifacts, returning
   the count to its previous level.

Nothing was suppressed or skipped to make this PR look green: the docs job runs
the real suite, and `docs:check` compares byte-for-byte against committed files.

---

## Usage after merge

```bash
cd web

# Where to write what when adding a component:
#   prop description      → JSDoc above the prop in the props interface
#   default value         → inline destructuring default, or @default
#   usage example         → @example
#   accessibility notes   → @accessibility
#   composition guidance  → @composition
#   deprecation/replacement → @deprecated (raises the banner + links the page)
#   breaking change       → scripts/docs/breaking-changes.json
#   page grouping/order   → scripts/docs/registry.mjs

npm run docs:build    # regenerate and commit
npm run docs:check    # exactly what CI runs
npm run docs:release  # when cutting a release
npm run docs:serve    # read it locally
```

To enable the published site: repository **Settings → Pages → Source: GitHub
Actions**, then run the `publish` job (or publish a release). The docs site is
also reachable inside the app at `/component-docs/index.html`.

---

## Risk, rollout and revert

- **Runtime risk: none.** No application component behaviour changed; the
  component edits are comments, and the existing suite still passes.
- **Bundle risk: none.** The site is static HTML in `public/`; nothing is added
  to the Next build graph or to any bundle.
- **CI risk: low and additive.** One new workflow with path filters; the
  `publish` job only runs on releases or manual dispatch.
- **Revert:** delete the added files/dirs, the four `docs:*` scripts and the
  JSDoc blocks. Nothing else depends on them.
- **Reviewing generated files:** if you change wording in the generator or in
  component JSDoc, run `npm run docs:build` before pushing — otherwise
  `docs:check` will (correctly) fail the PR.

## Follow-up issues worth filing separately

1. The three pre-existing blockers above (`next build` route conflicts, the
   `tsc` backlog, the missing `@typescript-eslint` plugin, and the Node ≤ 20
   test-script glob).
2. Extend the registry to the remaining app components (`VaultRow`,
   `NetworkStatusBanner`, `upload-zone`, …) — intentionally out of scope here so
   the first PR stays reviewable.
3. Optional: generate a docs page for the lib helpers (`lib/utils.ts`,
   `lib/errorHandling.ts`) with the same extractor.
