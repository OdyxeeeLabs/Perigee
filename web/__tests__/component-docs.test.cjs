// component-docs.test.cjs — component library documentation (#709)
//
// Covers the whole delivery:
//   1. props tables auto-generated from TypeScript + JSDoc (types, defaults,
//      descriptions, required flags)
//   2. usage examples, composition patterns and accessibility notes per page
//   3. the generated migration guide for breaking changes
//   4. the dedicated static docs site (accessibility, links, search index)
//   5. versioning with releases (version selector + frozen snapshots)
//   6. drift detection (`docs:check`) so the docs cannot silently go stale
//
// Runs with: node --test ./__tests__/component-docs.test.cjs

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
  appendFileSync,
  cpSync,
} = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');

const WEB_ROOT = join(__dirname, '..');
const DOCS_DIR = join(WEB_ROOT, 'docs', 'components');
const SITE_DIR = join(WEB_ROOT, 'public', 'component-docs');
const GENERATOR = join(WEB_ROOT, 'scripts', 'docs', 'generate.mjs');

const read = (relative) => readFileSync(join(WEB_ROOT, relative), 'utf8');
const readJson = (relative) => JSON.parse(read(relative));

function runGenerator(args, options = {}) {
  return spawnSync(process.execPath, [GENERATOR, ...args], {
    cwd: options.cwd ?? WEB_ROOT,
    encoding: 'utf8',
  });
}

/** Builds a throwaway `web/` root so write/drift/release paths can be tested. */
function makeTempWebRoot(version = '9.9.9') {
  const dir = mkdtempSync(join(tmpdir(), 'perigee-docs-'));
  cpSync(join(WEB_ROOT, 'components'), join(dir, 'components'), { recursive: true });
  cpSync(join(WEB_ROOT, 'scripts'), join(dir, 'scripts'), { recursive: true });
  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify({ name: 'Perigee-web', version }, null, 2) + '\n',
  );
  return dir;
}

/** Every docs page in the shipped registry. */
const PAGES = [
  'button',
  'badge',
  'data-table',
  'spinner',
  'card',
  'live-region',
  'common-button',
];

// ─────────────────────────────────────────────────────────────────────────────
// 1. Generated props tables (TypeScript + JSDoc)
// ─────────────────────────────────────────────────────────────────────────────

test('props tables are extracted from TypeScript and JSDoc', async (t) => {
  const button = read('docs/components/button.md');

  await t.test('props table has the expected columns', () => {
    assert.match(
      button,
      /\| Prop \| Type \| Default \| Required \| Description \|/,
      'props table header missing',
    );
  });

  await t.test('types come from the TypeScript union declaration', () => {
    assert.match(
      button,
      /\| `variant` \| `"default" \\\| "destructive" \\\| "outline" \\\| "secondary" \\\| "ghost" \\\| "link"` \|/,
      'variant type union not rendered',
    );
  });

  await t.test('defaults come from destructuring, not prose', () => {
    assert.match(
      button,
      /\| `size` \| `"default" \\?\| "sm" \\?\| "lg" \\?\| "icon"` \| `"default"` \| No \|/,
    );
  });

  await t.test('aliased destructuring defaults are attributed to the public prop', () => {
    // `asChild: _asChild = false` — the row must be `asChild`, with default false.
    assert.match(button, /\| `asChild` \| `boolean` \| `false` \| No \|/);
    assert.doesNotMatch(button, /_asChild/, 'internal alias leaked into the docs');
  });

  await t.test('descriptions come from JSDoc on the props interface', () => {
    assert.match(button, /Visual treatment\./);
    assert.match(button, /Control height and padding\./);
  });

  await t.test('required props are marked as required', () => {
    const table = read('docs/components/data-table.md');
    assert.match(table, /\| `columns` \| `ColumnDef<T>\[\]` \| — \| Yes \|/);
    assert.match(table, /\| `data` \| `T\[\]` \| — \| Yes \|/);
  });

  await t.test('defaults are read even when quoted or non-primitive', () => {
    const table = read('docs/components/data-table.md');
    assert.match(table, /`pageSize` \| `number` \| `0` \| No \|/);
    assert.match(table, /`filterPlaceholder` \| `string` \| `'Filter…'` \| No \|/);
  });

  await t.test('referenced union types are expanded into a Types section', () => {
    const spinner = read('docs/components/spinner.md');
    assert.match(spinner, /## Types/);
    assert.match(spinner, /### `SpinnerColor`/);
    assert.match(spinner, /- `primary`/);
    assert.match(spinner, /### `SpinnerSize`/);
    assert.match(spinner, /- `xs`/);
  });

  await t.test('object-shaped types get their own props table', () => {
    const table = read('docs/components/data-table.md');
    assert.match(table, /### `ColumnDef`/);
    assert.match(table, /\| `cell` \| `\(row: T\) => React\.ReactNode` \| — \| No \|/);
  });

  await t.test('inherited native attributes are documented separately', () => {
    assert.match(button, /### Inherited attributes/);
    assert.match(button, /`React\.ButtonHTMLAttributes<HTMLButtonElement>`/);
  });

  await t.test('generic props types resolve to their declarations', () => {
    // `DataTableProps<T>` and `ColumnDef<T>` must not be treated as unknown.
    const manifest = readJson('docs/components/manifest.json');
    const dataTable = manifest.components.find((entry) => entry.slug === 'data-table');
    assert.equal(dataTable.exports[0].propsType, 'DataTableProps<T>');
    assert.equal(dataTable.exports[0].props.length, 11);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 2. Usage examples, composition patterns, accessibility
// ─────────────────────────────────────────────────────────────────────────────

test('pages include usage examples, composition patterns and accessibility notes', async (t) => {
  const manifest = readJson('docs/components/manifest.json');

  await t.test('every documented export has a description', () => {
    for (const entry of manifest.components) {
      for (const exported of entry.exports) {
        assert.ok(
          exported.props.length > 0 || exported.element,
          `${exported.name} documented without props or element`,
        );
      }
    }
  });

  await t.test('every page ships a usage example as runnable code', () => {
    const missing = [];
    for (const slug of PAGES) {
      const markdown = read(`docs/components/${slug}.md`);
      const hasUsageSection = /^#+ Usage$/m.test(markdown);
      const hasTsx = markdown.includes('```tsx');
      if (!hasUsageSection || !hasTsx) missing.push(slug);
    }
    assert.deepEqual(missing, [], `pages without a usage example: ${missing.join(', ')}`);
  });

  await t.test('every page carries accessibility notes', () => {
    const missing = [];
    for (const entry of manifest.components) {
      const notes = entry.exports.reduce((total, exported) => total + exported.accessibilityNotes, 0);
      if (notes === 0) missing.push(entry.slug);
    }
    assert.deepEqual(missing, [], `pages without @accessibility notes: ${missing.join(', ')}`);
  });

  await t.test('examples render as fenced tsx code blocks', () => {
    const spinner = read('docs/components/spinner.md');
    assert.match(spinner, /## Usage[\s\S]*```tsx[\s\S]*<Spinner size="sm" color="primary" \/>/);
  });

  await t.test('compound components document a composition pattern', () => {
    const card = read('docs/components/card.md');
    assert.match(card, /## Composition/);
    assert.match(card, /compound component/);
    for (const part of ['CardHeader', 'CardTitle', 'CardDescription', 'CardContent', 'CardFooter']) {
      assert.match(card, new RegExp(`## \`${part}\``), `${part} is not documented`);
    }
  });

  await t.test('DataTable shows its usage example as code, not prose', () => {
    const dataTable = read('docs/components/data-table.md');
    assert.match(dataTable, /## Usage[\s\S]{0,80}```tsx/);
    assert.match(dataTable, /<DataTable columns=\{columns\} data=\{rows\} pageSize=\{10\} \/>/);
  });

  await t.test('deprecated exports get a banner and a migration target', () => {
    const legacy = read('docs/components/common-button.md');
    assert.match(legacy, /⚠️ \*\*Deprecated\.\*\*/);
    assert.match(legacy, /Migrate to \[`Button`\]\(\.\/button\.md\)/);
    assert.match(legacy, /## Migration notes/);
  });

  await t.test('the index lists every page, the a11y baseline and the patterns', () => {
    const index = read('docs/components/README.md');
    for (const slug of PAGES) {
      assert.match(index, new RegExp(`\\(./${slug}\\.md\\)`), `${slug} missing from the index`);
    }
    assert.match(index, /## Composition patterns/);
    assert.match(index, /## Accessibility baseline/);
    assert.match(index, /## Maintaining these docs/);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 3. Migration guide
// ─────────────────────────────────────────────────────────────────────────────

test('migration guide documents breaking changes with before/after props', async (t) => {
  const migrations = read('docs/components/migrations.md');

  await t.test('lists the releases that shipped breaking changes', () => {
    assert.match(migrations, /## v0\.1\.0/);
    assert.match(migrations, /## v0\.0\.1/);
    assert.match(migrations, /\| \[v0\.1\.0\]\(#release-0-1-0\) \|/);
  });

  await t.test('before/after tables are generated from the real components', () => {
    assert.match(migrations, /#### Before — CommonButton/);
    assert.match(migrations, /#### After — Button/);
    // CommonButton's variant set differs from Button's — proof the tables are
    // extracted rather than hand-written.
    assert.match(migrations, /`"primary" \\\| "secondary" \\\| "outline"`/);
    assert.match(migrations, /`"default" \\\| "destructive" \\\| "outline"/);
  });

  await t.test('each breaking change ships a call-site migration', () => {
    assert.match(migrations, /#### Call-site migration/);
    assert.match(migrations, /<CommonButton variant="primary">Save<\/CommonButton>/);
    assert.match(migrations, /<Button variant="default">Save<\/Button>/);
  });

  await t.test('codemods are included for mechanical migrations', () => {
    assert.match(migrations, /#### Codemod/);
    assert.match(migrations, /```bash/);
  });

  await t.test('deprecations are explained as a process', () => {
    assert.match(migrations, /## Handling a deprecation/);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 4. The docs site
// ─────────────────────────────────────────────────────────────────────────────

test('static docs site is generated and accessible', async (t) => {
  await t.test('every page is published', () => {
    for (const slug of PAGES) {
      assert.ok(existsSync(join(SITE_DIR, `${slug}.html`)), `${slug}.html missing`);
    }
    for (const slug of ['index', 'migrations', 'versions']) {
      assert.ok(existsSync(join(SITE_DIR, `${slug}.html`)), `${slug}.html missing`);
    }
  });

  await t.test('pages are valid, self-describing documents', () => {
    const html = read('public/component-docs/button.html');
    assert.match(html, /^<!doctype html>/i);
    assert.match(html, /<html lang="en">/);
    assert.match(html, /<meta name="viewport"/);
    assert.match(html, /name="generator" content="scripts\/docs\/generate\.mjs"/);
    assert.match(html, /<link rel="stylesheet" href="assets\/docs\.css"/);
  });

  await t.test('navigation and search are labelled for assistive tech', () => {
    const html = read('public/component-docs/index.html');
    assert.match(html, /<a class="skip-link" href="#content">Skip to content<\/a>/);
    assert.match(html, /<nav class="site-nav" aria-label="Component library">/);
    assert.match(html, /<label for="docs-search">Search docs<\/label>/);
    assert.match(html, /id="search-status" role="status" aria-live="polite"/);
    assert.match(html, /<main id="content" tabindex="-1">/);
  });

  await t.test('each page has exactly one h1 and unique element ids', () => {
    for (const slug of PAGES) {
      const html = read(`public/component-docs/${slug}.html`);
      const h1s = html.match(/<h1[ >]/g) ?? [];
      assert.equal(h1s.length, 1, `${slug}.html has ${h1s.length} h1 elements`);
      const ids = [...html.matchAll(/ id="([^"]+)"/g)].map((match) => match[1]);
      const duplicates = ids.filter((id, index) => ids.indexOf(id) !== index);
      assert.deepEqual(duplicates, [], `${slug}.html has duplicate ids: ${duplicates.join(', ')}`);
    }
  });

  await t.test('data tables use real table markup with scoped headers', () => {
    const html = read('public/component-docs/button.html');
    assert.match(html, /<table><thead><tr><th scope="col">Prop<\/th>/);
    assert.match(html, /<td><code>variant<\/code><\/td>/);
  });

  await t.test('markdown links are rewritten to site links', () => {
    for (const slug of [...PAGES, 'index', 'migrations']) {
      const html = read(`public/component-docs/${slug}.html`);
      const markdownLinks = html.match(/href="[^"]*\.md[^"]*"/g) ?? [];
      assert.deepEqual(markdownLinks, [], `${slug}.html still links to markdown: ${markdownLinks.join(', ')}`);
    }
  });

  await t.test('internal links point at published pages', () => {
    const html = read('public/component-docs/button.html');
    for (const match of html.matchAll(/href="([a-z0-9-]+\.html)(#[^"]*)?"/g)) {
      assert.ok(existsSync(join(SITE_DIR, match[1])), `dangling link to ${match[1]}`);
    }
  });

  await t.test('compound pages anchor each export uniquely', () => {
    const card = read('public/component-docs/card.html');
    for (const name of ['Card', 'CardHeader', 'CardTitle', 'CardDescription', 'CardContent', 'CardFooter']) {
      assert.match(card, new RegExp(`id="component-${name.replace(/([a-z])([A-Z])/g, '$1-$2').toLowerCase()}"`));
    }
  });

  await t.test('a search index is published for the site', () => {
    const index = readJson('public/component-docs/search-index.json');
    assert.equal(index.pages.length, PAGES.length + 3);
    const button = index.pages.find((page) => page.slug === 'button');
    assert.ok(button.headings.some((heading) => heading.id === 'props' && heading.text === 'Props'));
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 5. Versioning with releases
// ─────────────────────────────────────────────────────────────────────────────

test('docs are versioned with releases', async (t) => {
  const versions = readJson('docs/components/versions.json');
  const pkg = readJson('package.json');

  await t.test('the current version tracks package.json', () => {
    assert.equal(versions.current, pkg.version);
    assert.ok(versions.versions.some((entry) => entry.version === pkg.version && entry.status === 'current'));
  });

  await t.test('released versions publish a frozen snapshot', () => {
    const released = versions.versions.filter((entry) => entry.released);
    assert.ok(released.length > 0, 'no released version recorded');
    for (const entry of released) {
      assert.ok(entry.docsPath, `v${entry.version} has no docsPath`);
      assert.ok(
        existsSync(join(WEB_ROOT, 'docs', 'components', 'versions', `${entry.version}.json`)),
        `docs snapshot for v${entry.version} missing`,
      );
      assert.ok(
        existsSync(join(SITE_DIR, entry.docsPath, 'index.html')),
        `site snapshot for v${entry.version} missing`,
      );
    }
  });

  await t.test('snapshot pages link back to the root assets', () => {
    const html = read(`public/component-docs/${versions.versions[0].docsPath}index.html`);
    // versions/v<version>/index.html needs two levels up to reach the root.
    assert.match(html, /href="\.\.\/\.\.\/assets\/docs\.css"/);
    assert.match(html, /href="\.\.\/\.\.\/index\.html"/);
  });

  await t.test('release and build run in an isolated root (version + selector)', () => {
    const dir = makeTempWebRoot('9.9.9');
    try {
      // Seed previous versions so the selector has released and unpublished
      // entries alongside the current one.
      const docsDir = join(dir, 'docs', 'components');
      mkdirSync(docsDir, { recursive: true });
      writeFileSync(
        join(docsDir, 'versions.json'),
        JSON.stringify(
          {
            current: '1.0.0',
            versions: [
              { version: '1.0.0', status: 'current', released: null, docsPath: null, components: 6, exports: 11 },
              { version: '0.9.0', status: 'released', released: '2026-09-01', docsPath: 'versions/v0.9.0/', components: 6, exports: 11 },
            ],
          },
          null,
          2,
        ),
      );
      // A version recorded as published must actually have a snapshot on disk,
      // otherwise the docs contain a dangling link (the generator fails loudly).
      mkdirSync(join(dir, 'public/component-docs/versions/v0.9.0'), { recursive: true });
      writeFileSync(
        join(dir, 'public/component-docs/versions/v0.9.0/index.html'),
        '<!doctype html><html lang="en"><head><title>v0.9.0</title></head><body></body></html>',
      );

      const built = runGenerator(['--web-root', dir, '--version', '9.9.9', '--release', '--date', '2026-10-15']);
      assert.equal(built.status, 0, built.stderr);

      const released = JSON.parse(readFileSync(join(docsDir, 'versions.json'), 'utf8'));
      assert.equal(released.current, '9.9.9');
      const entry = released.versions.find((item) => item.version === '9.9.9');
      assert.equal(entry.released, '2026-10-15');
      assert.equal(entry.docsPath, 'versions/v9.9.9/');
      assert.ok(
        existsSync(join(dir, 'public/component-docs/versions/v9.9.9/index.html')),
        'versioned snapshot not published',
      );

      const siteIndex = readFileSync(join(dir, 'public/component-docs/index.html'), 'utf8');
      assert.match(siteIndex, /<label for="version-select">Version<\/label>/);
      // The current version always points at the live docs…
      assert.match(siteIndex, /<option value="index\.html" selected>v9\.9\.9 \(current\)<\/option>/);
      // …while published versions link to their frozen snapshot…
      assert.match(siteIndex, /<option value="versions\/v0\.9\.0\/index\.html">v0\.9\.0<\/option>/);
      // …and versions with no snapshot are shown but not selectable.
      assert.match(siteIndex, /<option disabled>v1\.0\.0 \(snapshot not published\)<\/option>/);

      const snapshotIndex = readFileSync(
        join(dir, 'public/component-docs/versions/v9.9.9/index.html'),
        'utf8',
      );
      assert.match(snapshotIndex, /href="\.\.\/\.\.\/assets\/docs\.css"/);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 6. Drift detection
// ─────────────────────────────────────────────────────────────────────────────

test('docs:check detects stale and unexpected files', async (t) => {
  const dir = makeTempWebRoot('2.0.0');
  const docsDir = join(dir, 'docs', 'components');
  try {
    await t.test('fails while nothing has been generated yet', () => {
      const result = runGenerator(['--web-root', dir, '--check']);
      assert.equal(result.status, 1);
      assert.match(result.stderr, /out of date/);
      assert.match(result.stderr, /missing +docs\/components\/README\.md/);
    });

    await t.test('passes once the docs are generated', () => {
      const build = runGenerator(['--web-root', dir]);
      assert.equal(build.status, 0, build.stderr);
      assert.equal(readFileSync(join(docsDir, 'button.md'), 'utf8').includes('# Button'), true);

      const check = runGenerator(['--web-root', dir, '--check']);
      assert.equal(check.status, 0, check.stderr);
      assert.match(check.stdout, /up to date/);
    });

    await t.test('fails when a generated file is edited by hand', () => {
      appendFileSync(join(docsDir, 'button.md'), '\nhand-written note\n');
      const check = runGenerator(['--web-root', dir, '--check']);
      assert.equal(check.status, 1);
      assert.match(check.stderr, /stale {4}docs\/components\/button\.md/);
    });

    await t.test('recovers when the file is regenerated', () => {
      assert.equal(runGenerator(['--web-root', dir]).status, 0);
      assert.equal(runGenerator(['--web-root', dir, '--check']).status, 0);
    });

    await t.test('fails when a page for a removed component lingers', () => {
      writeFileSync(join(docsDir, 'removed-component.md'), 'Generated by `npm run docs:build`\n');
      const check = runGenerator(['--web-root', dir, '--check']);
      assert.equal(check.status, 1);
      assert.match(check.stderr, /unexpected docs\/components\/removed-component\.md/);
    });

    await t.test('an ordinary build prunes pages that are no longer generated', () => {
      const build = runGenerator(['--web-root', dir]);
      assert.equal(build.status, 0, build.stderr);
      assert.equal(existsSync(join(docsDir, 'removed-component.md')), false);
      assert.equal(runGenerator(['--web-root', dir, '--check']).status, 0);
    });

    await t.test('published version snapshots are left alone', () => {
      const release = runGenerator(['--web-root', dir, '--release', '--date', '2026-10-16']);
      assert.equal(release.status, 0, release.stderr);
      const snapshotSite = join(dir, 'public/component-docs/versions/v2.0.0/index.html');
      assert.ok(existsSync(snapshotSite));

      const rebuild = runGenerator(['--web-root', dir]);
      assert.equal(rebuild.status, 0, rebuild.stderr);
      assert.ok(existsSync(snapshotSite), 'rebuild deleted the published snapshot');
      assert.ok(existsSync(join(docsDir, 'versions', '2.0.0.json')));
      assert.equal(runGenerator(['--web-root', dir, '--check']).status, 0);
    });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ─────────────────────────────────────────────────────────────────────────────
// 7. Committed artifacts are in sync with source
// ─────────────────────────────────────────────────────────────────────────────

test('committed docs match the component sources', async (t) => {
  await t.test('manifest lists every documented export', () => {
    const manifest = readJson('docs/components/manifest.json');
    const names = manifest.components.flatMap((entry) => entry.exports.map((exported) => exported.name));
    for (const name of ['Button', 'Badge', 'DataTable', 'Spinner', 'Card', 'LiveStatus', 'LiveAlert', 'CommonButton']) {
      assert.ok(names.includes(name), `${name} missing from the manifest`);
    }
    assert.equal(manifest.components.length, 7);
  });

  await t.test('every prop row carries a type', () => {
    const manifest = readJson('docs/components/manifest.json');
    for (const entry of manifest.components) {
      for (const exported of entry.exports) {
        for (const prop of exported.props) {
          assert.ok(prop.type && prop.type.length > 0, `${exported.name}.${prop.name} has no type`);
        }
      }
    }
  });

  await t.test('docs:check passes for the committed artifacts', () => {
    const result = runGenerator(['--check']);
    assert.equal(result.status, 0, result.stderr);
  });

  await t.test('the registry covers every Markdown page on disk', () => {
    const pages = readdirSync(DOCS_DIR)
      .filter((file) => file.endsWith('.md'))
      .filter((file) => !['README.md', 'migrations.md', 'versions.md'].includes(file))
      .map((file) => file.replace(/\.md$/, ''))
      .sort();
    assert.deepEqual(pages, [...PAGES].sort());
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 8. Unit coverage for the pieces the CLI relies on
// ─────────────────────────────────────────────────────────────────────────────

test('extractor and markdown renderer behave in isolation', async (t) => {
  const { extractFile, resolveComponent } = await import('../scripts/docs/lib/extract.mjs');
  const { markdownToHtml } = await import('../scripts/docs/lib/markdown.mjs');
  const { linkResolver } = await import('../scripts/docs/lib/site.mjs');

  await t.test('extractor reads props, defaults and tags straight from a file', () => {
    const file = extractFile(join(WEB_ROOT, 'components/ui/Spinner.tsx'), 'components/ui/Spinner.tsx');
    const spinner = resolveComponent(file, 'Spinner');
    assert.equal(spinner.props.length, 4);
    assert.equal(spinner.props[0].defaultValue, '"md"');
    assert.equal(spinner.props[0].description.startsWith('Controls the outer diameter'), true);
    assert.equal(spinner.element, 'span');
    assert.equal(spinner.since, '0.1.0');
    assert.equal(spinner.accessibility.length, 1);
  });

  await t.test('extractor reports a missing export as null instead of throwing', () => {
    const file = extractFile(join(WEB_ROOT, 'components/ui/Button.tsx'), 'components/ui/Button.tsx');
    assert.equal(resolveComponent(file, 'NotAComponent'), null);
  });

  await t.test('markdown tables are escaped and scoped', () => {
    const { html } = markdownToHtml('| a | b |\n| --- | --- |\n| `x \\| y` | <script> |');
    assert.match(html, /<th scope="col">a<\/th>/);
    assert.match(html, /<code>x \| y<\/code>/);
    assert.match(html, /&lt;script&gt;/);
    assert.doesNotMatch(html, /<script>/);
  });

  await t.test('markdown links resolve to site pages', () => {
    const resolveLink = linkResolver();
    assert.equal(resolveLink('./button.md'), 'button.html');
    assert.equal(resolveLink('./README.md'), 'index.html');
    assert.equal(resolveLink('./migrations.md#release-0-1-0'), 'migrations.html#release-0-1-0');
    assert.equal(resolveLink('https://example.com/x.md'), 'https://example.com/x.md');
    assert.equal(linkResolver('../')('./button.md'), '../button.html');
  });

  await t.test('indented JSDoc code samples are converted to fenced blocks', async () => {
    const { normaliseProse } = await import('../scripts/docs/lib/render.mjs');
    const markdown = normaliseProse('Overview line.\n\nUsage:\n  const a = 1;\n  <X a={a} />');
    assert.match(markdown, /```tsx\nconst a = 1;\n<X a=\{a\} \/>\n```/);
  });

  await t.test('lists, code fences and blockquotes render', () => {
    const { html, headings } = markdownToHtml(
      '# Title\n\n> warn\n\n- one\n  - nested\n\n```ts\nconst x = 1;\n```\n',
    );
    assert.match(html, /<h1 id="title">Title<\/h1>/);
    assert.match(html, /<blockquote><p>warn<\/p><\/blockquote>/);
    assert.match(html, /<ul><li>one<ul><li>nested<\/li><\/ul><\/li><\/ul>/);
    assert.match(html, /<pre tabindex="0"><code class="language-ts">/);
    assert.deepEqual(headings, [{ level: 1, id: 'title', text: 'Title' }]);
  });
});
