// registry.mjs — hand-written source of truth for *what* gets documented.
//
// The generator derives every prop, type, default, and JSDoc note from the
// component files themselves; this file only declares which exports make up the
// public component library and how the docs site is organised. Adding a new
// component to the docs means adding one entry here and writing JSDoc on the
// component — nothing else.

export const LIBRARY = {
  title: 'Perigee Web Component Library',
  tagline: 'Shared UI primitives for the Perigee web app — documented from source.',
  /** Repo-relative directory the component imports are rooted at. */
  importRoot: 'components',
  /** Root of the deployed docs site inside the Next.js app. */
  siteBasePath: '/component-docs',
  /** Where the generated markdown lives, relative to `web/`. */
  docsDir: 'docs/components',
  /** Where the generated static site lives, relative to `web/`. */
  siteDir: 'public/component-docs',
  /** Canonical repo location of the generated markdown, linked from the index. */
  docsUrl: 'https://github.com/OdyxeeeLabs/Perigee/tree/main/web/docs/components',
};

/** Grouping used by the site nav and the generated index. Order matters. */
export const GROUPS = [
  { id: 'actions', title: 'Actions', blurb: 'Things the user can press.' },
  { id: 'forms', title: 'Forms & input', blurb: 'Data entry and validation surfaces.' },
  { id: 'data-display', title: 'Data display', blurb: 'Read-only information surfaces.' },
  { id: 'feedback', title: 'Feedback', blurb: 'Progress, loading, and status signals.' },
  { id: 'layout', title: 'Layout', blurb: 'Structural containers and spacing.' },
  { id: 'accessibility', title: 'Accessibility', blurb: 'Primitives that exist purely for assistive technology.' },
  { id: 'legacy', title: 'Legacy', blurb: 'Superseded components awaiting removal.' },
];

/**
 * Public components, one entry per docs page. `exports` lists the named exports
 * rendered on that page (in order) — a page can document a compound component
 * such as `Card` + its five parts.
 */
export const COMPONENTS = [
  {
    slug: 'button',
    group: 'actions',
    title: 'Button',
    exports: ['Button'],
    source: 'components/ui/Button.tsx',
    status: 'stable',
    issue: 'WEB-27 (#113)',
    usedFor: 'Primary and secondary actions, form submits, toolbar controls.',
    related: ['common-button', 'spinner'],
  },
  {
    slug: 'badge',
    group: 'data-display',
    title: 'Badge',
    exports: ['Badge'],
    source: 'components/ui/Badge.tsx',
    status: 'stable',
    issue: 'WEB-27 (#113), WEB-38 (#492)',
    usedFor: 'Status pills, severity tags, resource budget indicators.',
    related: ['data-table'],
  },
  {
    slug: 'data-table',
    group: 'data-display',
    title: 'DataTable',
    exports: ['DataTable'],
    source: 'components/ui/DataTable.tsx',
    status: 'stable',
    issue: 'WEB-33, WEB-34',
    usedFor: 'Sortable, filterable, paginated tables of vault and invocation data.',
    related: ['badge', 'card'],
  },
  {
    slug: 'spinner',
    group: 'feedback',
    title: 'Spinner',
    exports: ['Spinner'],
    source: 'components/ui/Spinner.tsx',
    status: 'stable',
    usedFor: 'Inline and block loading states.',
    related: ['button', 'live-region'],
  },
  {
    slug: 'card',
    group: 'layout',
    title: 'Card',
    exports: ['Card', 'CardHeader', 'CardTitle', 'CardDescription', 'CardContent', 'CardFooter'],
    source: 'components/ui/card.tsx',
    status: 'stable',
    usedFor: 'Panels that group related content and actions.',
    related: ['button', 'data-table'],
  },
  {
    slug: 'live-region',
    group: 'accessibility',
    title: 'Live regions',
    exports: ['LiveStatus', 'LiveAlert'],
    source: 'components/LiveRegion.tsx',
    status: 'stable',
    issue: 'WEB-16 (#470)',
    usedFor: 'Announcing async progress and failures to screen readers.',
    related: ['spinner'],
  },
  {
    slug: 'common-button',
    group: 'legacy',
    title: 'CommonButton',
    exports: ['CommonButton'],
    source: 'components/common/Button/Button.tsx',
    status: 'deprecated',
    usedFor: 'Nothing new — migrate to Button.',
    migration: 'button',
    related: ['button'],
  },
];

/**
 * Cross-component composition patterns. These are intentionally not attached to
 * a single component: they show how library pieces fit together in the app.
 */
export const PATTERNS = [
  {
    title: 'Loading button',
    summary:
      'Buttons that trigger async work stay focusable and announce progress: set `aria-busy`, keep the label, and put a `Spinner` in the leading slot.',
    code: `<Button aria-busy={saving} disabled={saving}>
  {saving ? <Spinner size="sm" color="white" /> : null}
  {saving ? 'Saving…' : 'Save'}
</Button>`,
  },
  {
    title: 'Table inside a card',
    summary:
      '`DataTable` owns its own border, so render it inside `CardContent` with the card chrome disabled by passing `className="border-0"` — this keeps one border per surface.',
    code: `<Card>
  <CardHeader>
    <CardTitle>Vaults</CardTitle>
  </CardHeader>
  <CardContent>
    <DataTable columns={columns} data={vaults} pageSize={10} className="border-0" />
  </CardContent>
</Card>`,
  },
  {
    title: 'Status pill in a table cell',
    summary:
      'Use a `ColumnDef.cell` renderer to map a row value to a `Badge` variant instead of styling the cell directly.',
    code: `const columns: ColumnDef<Vault>[] = [
  { key: 'name', header: 'Name', sortable: true },
  {
    key: 'status',
    header: 'Status',
    cell: (vault) => (
      <Badge variant={vault.status === 'healthy' ? 'success' : 'warning'}>
        {vault.status}
      </Badge>
    ),
  },
];`,
  },
  {
    title: 'Announce the result of an action',
    summary:
      'Keep a live region mounted for the lifetime of the screen and change only its message — a region that mounts with its text is frequently not announced.',
    code: `<LiveStatus message={status} />
<Button onClick={runAnalysis} aria-busy={running} disabled={running}>
  {running ? 'Analysing…' : 'Analyse'}
</Button>`,
  },
];

/**
 * Accessibility principles applied across the library. Shown on the docs index
 * so the per-component notes have a shared frame of reference.
 */
export const A11Y_PRINCIPLES = [
  {
    title: 'Native elements first',
    detail:
      'Interactive primitives render real `<button>`, `<input>`, `<a>`, and `<table>` elements so keyboard support, focus handling, and browser semantics come for free.',
  },
  {
    title: 'Visible focus is never removed',
    detail:
      'Every focusable component paints a `focus-visible` ring in the brand cyan. Do not override `outline` when composing these primitives.',
  },
  {
    title: 'Announce dynamic changes',
    detail:
      'Async results are announced through `LiveStatus` (polite) and `LiveAlert` (assertive) rather than relying on colour or layout changes.',
  },
  {
    title: 'Colour is never the only signal',
    detail:
      'Status colour pairings are contrast-audited (≥ 4.5:1) and always paired with a text label.',
  },
  {
    title: 'Reduced motion is respected',
    detail:
      'A global `prefers-reduced-motion` rule disables decorative animation, so animated components must also expose a non-motion cue.',
  },
];

/** Headings used by the generated migration guide. */
export const MIGRATION_INTRO = `Breaking changes to the component library are recorded here — in
\`scripts/docs/breaking-changes.json\` — with the before/after props,
generated straight from source, and a copy-pasteable migration for each
affected call site. Every entry links back to the release it shipped in.`;
