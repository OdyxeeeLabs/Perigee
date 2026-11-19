#!/usr/bin/env node
// generate.mjs — the component-library documentation generator (issue #709).
//
//   node scripts/docs/generate.mjs                # write docs + docs site
//   node scripts/docs/generate.mjs --check        # fail if committed docs are stale
//   node scripts/docs/generate.mjs --release      # freeze + publish this version
//   node scripts/docs/generate.mjs --no-site      # markdown only
//
// Inputs:  component source + JSDoc, scripts/docs/registry.mjs,
//          scripts/docs/breaking-changes.json, package.json#version
// Outputs: docs/components/**            (markdown, committed)
//          public/component-docs/**      (static site, committed)

import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { extractFile, resolveComponent } from './lib/extract.mjs';
import { markdownToHtml } from './lib/markdown.mjs';
import { renderComponentPage, renderIndexPage, renderMigrationsPage } from './lib/render.mjs';
import { buildSite } from './lib/site.mjs';
import {
  A11Y_PRINCIPLES,
  COMPONENTS,
  GROUPS,
  LIBRARY,
  MIGRATION_INTRO,
  PATTERNS,
} from './registry.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WEB_ROOT = path.resolve(HERE, '..', '..');

// ─────────────────────────────────────────────────────────────────────────────
// CLI
// ─────────────────────────────────────────────────────────────────────────────

function parseArgs(argv) {
  const options = {
    check: false,
    release: false,
    site: true,
    quiet: false,
    version: null,
    date: null,
    webRoot: WEB_ROOT,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--check') options.check = true;
    else if (arg === '--release') options.release = true;
    else if (arg === '--no-site') options.site = false;
    else if (arg === '--quiet') options.quiet = true;
    else if (arg === '--version') options.version = argv[++i];
    else if (arg === '--date') options.date = argv[++i];
    else if (arg === '--web-root') options.webRoot = path.resolve(argv[++i]);
    else if (arg === '--help' || arg === '-h') {
      process.stdout.write(readFileSync(fileURLToPath(import.meta.url), 'utf8').split('\n').slice(1, 12).join('\n') + '\n');
      process.exit(0);
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }
  return options;
}

function today() {
  return new Date().toISOString().slice(0, 10);
}

// ─────────────────────────────────────────────────────────────────────────────
// Sources
// ─────────────────────────────────────────────────────────────────────────────

function readJsonFile(file) {
  if (!existsSync(file)) return null;
  try {
    return JSON.parse(readFileSync(file, 'utf8'));
  } catch (error) {
    throw new Error(`Could not parse ${path.relative(WEB_ROOT, file)}: ${error.message}`);
  }
}

function packageVersion(webRoot) {
  const pkg = readJsonFile(path.join(webRoot, 'package.json'));
  if (!pkg?.version) throw new Error('web/package.json is missing a "version" field.');
  return pkg.version;
}

function loadBreakingChanges(webRoot) {
  const file = path.join(webRoot, 'scripts', 'docs', 'breaking-changes.json');
  const data = readJsonFile(file);
  if (!data?.releases) throw new Error(`Missing releases in ${path.relative(webRoot, file)}.`);
  return data.releases;
}

// ─────────────────────────────────────────────────────────────────────────────
// Model
// ─────────────────────────────────────────────────────────────────────────────

function buildModel({ webRoot, version, releases, releaseDate }) {
  const byName = new Map();
  const bySlug = new Map(COMPONENTS.map((entry) => [entry.slug, entry]));
  const pages = [];

  for (const entry of COMPONENTS) {
    const absolute = path.join(webRoot, entry.source);
    if (!existsSync(absolute)) {
      throw new Error(`docs: registry entry "${entry.slug}" points at a missing file (${entry.source}).`);
    }
    const fileModel = extractFile(absolute, entry.source);
    const components = entry.exports.map((exportName) => {
      const resolved = resolveComponent(fileModel, exportName);
      if (!resolved) {
        throw new Error(
          `docs: export "${exportName}" from ${entry.source} could not be documented. ` +
            'Check the name in scripts/docs/registry.mjs.',
        );
      }
      byName.set(exportName, resolved);
      return resolved;
    });

    pages.push({
      ...entry,
      group: entry.group,
      groupTitle: GROUPS.find((group) => group.id === entry.group)?.title ?? entry.group,
      components,
      exports: entry.exports.slice(),
      exportCount: entry.exports.length,
    });
  }

  // Validate the hand-written breaking-change data against the real components
  // so the migration guide can never reference a component that does not exist.
  const changes = [];
  for (const release of releases) {
    for (const change of release.changes) {
      for (const side of ['from', 'to']) {
        const ref = change[side];
        if (ref && !byName.has(ref.component)) {
          throw new Error(
            `docs: breaking change "${change.id}" references unknown component "${ref.component}" (${side}).`,
          );
        }
      }
      if (change.component && !bySlug.has(change.component)) {
        throw new Error(
          `docs: breaking change "${change.id}" references unknown docs page "${change.component}".`,
        );
      }
      for (const slug of change.affects ?? []) {
        if (!bySlug.has(slug)) {
          throw new Error(`docs: breaking change "${change.id}" affects unknown page "${slug}".`);
        }
      }
      changes.push({ ...change, version: release.version, date: release.date });
    }
  }

  const exportCount = pages.reduce((total, page) => total + page.exportCount, 0);
  const ctx = {
    version,
    library: LIBRARY,
    groups: GROUPS,
    patterns: PATTERNS,
    a11yPrinciples: A11Y_PRINCIPLES,
    migrationIntro: MIGRATION_INTRO,
    registry: COMPONENTS,
    componentCount: pages.length,
    exportCount,
    breakingChanges: changes,
    releases,
    byName,
    pages: pages.map((page) => ({
      slug: page.slug,
      title: page.title,
      group: page.group,
      status: page.status,
    })),
  };

  return { pages, ctx, byName, releaseDate };
}

function toManifest(pages, version) {
  return {
    library: LIBRARY.title,
    version,
    groups: GROUPS.map((group) => ({ id: group.id, title: group.title })),
    components: pages.map((page) => ({
      slug: page.slug,
      title: page.title,
      group: page.group,
      status: page.status,
      issue: page.issue ?? null,
      source: page.source,
      exports: page.components.map((component) => ({
        name: component.name,
        kind: component.kind,
        element: component.element,
        propsType: component.propsTypeName,
        since: component.since,
        deprecated: component.deprecated,
        accessibilityNotes: component.accessibility.length,
        examples: component.examples.length,
        inheritedFrom: component.inheritedFrom,
        props: component.props.map((prop) => ({
          name: prop.name,
          type: prop.type,
          default: prop.defaultValue,
          required: !prop.optional,
          description: prop.description.replace(/\s+/g, ' ').trim() || null,
        })),
        types: component.types.map((type) => ({
          name: type.name,
          values: type.values ?? null,
        })),
      })),
    })),
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Link validation
// ─────────────────────────────────────────────────────────────────────────────

/** Every href in the generated site must resolve to a page, asset, or anchor. */
export function validateSite(siteFiles, isKnownSitePath = null) {
  const known = isKnownSitePath ?? ((file) => file in siteFiles);
  const problems = [];
  const pages = Object.keys(siteFiles).filter((file) => file.endsWith('.html'));
  const idsByPage = new Map(
    pages.map((page) => [
      page,
      new Set([...siteFiles[page].matchAll(/ id="([^"]+)"/g)].map((match) => match[1])),
    ]),
  );

  for (const page of pages) {
    const directory = path.posix.dirname(page);
    for (const match of siteFiles[page].matchAll(/href="([^"]+)"/g)) {
      const href = match[1];
      if (/^(https?:|mailto:|tel:)/.test(href)) continue;
      const [target, fragment] = href.split('#');
      if (!target) {
        if (fragment && !idsByPage.get(page).has(fragment)) {
          problems.push(`${page}: anchor #${fragment} has no target`);
        }
        continue;
      }
      const resolved = path.posix.normalize(
        directory === '.' ? target : path.posix.join(directory, target),
      );
      if (!known(resolved)) {
        problems.push(`${page}: broken link ${href}`);
        continue;
      }
      if (fragment && idsByPage.has(resolved) && !idsByPage.get(resolved).has(fragment)) {
        problems.push(`${page}: link ${href} points at a missing anchor`);
      }
    }
  }
  return problems;
}

/** Markdown links must resolve inside the docs directory or the published site. */
export function validateMarkdown(docsFiles, siteFiles, isKnownSitePath = null) {
  const known = isKnownSitePath ?? ((file) => file in siteFiles);
  const problems = [];
  const siteMarker = '/public/component-docs/';
  for (const [relative, contents] of Object.entries(docsFiles)) {
    if (!relative.endsWith('.md')) continue;
    for (const match of contents.matchAll(/\]\(([^)\s]+)\)/g)) {
      const href = match[1];
      if (/^(https?:|mailto:|#)/.test(href)) continue;
      const [target] = href.split('#');
      if (target.includes(siteMarker)) {
        const resolved = target.slice(target.indexOf(siteMarker) + siteMarker.length);
        if (!known(resolved)) problems.push(`${relative}: broken link ${href}`);
        continue;
      }
      const resolved = path.posix.normalize(path.posix.join(path.posix.dirname(relative), target));
      if (!(resolved in docsFiles)) problems.push(`${relative}: broken link ${href}`);
    }
  }
  return problems;
}

/**
 * @param {Set<string>} publishedPaths files that already exist on disk (frozen
 *   version snapshots are not rebuilt by an ordinary `--check` run).
 */
function assertLinksValid(docsFiles, siteFiles, publishedPaths) {
  const isKnownSitePath = (file) => file in siteFiles || publishedPaths.has(file);
  const problems = [
    ...validateMarkdown(docsFiles, siteFiles, isKnownSitePath),
    ...(Object.keys(siteFiles).length ? validateSite(siteFiles, isKnownSitePath) : []),
  ];
  if (problems.length) {
    throw new Error(
      `Generated docs contain ${problems.length} broken link(s):\n  ${problems.slice(0, 20).join('\n  ')}`,
    );
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Versions
// ─────────────────────────────────────────────────────────────────────────────

function compareVersions(a, b) {
  const parts = (value) => String(value).split('.').map((part) => Number.parseInt(part, 10) || 0);
  const [aMajor, aMinor, aPatch] = parts(a);
  const [bMajor, bMinor, bPatch] = parts(b);
  return aMajor - bMajor || aMinor - bMinor || aPatch - bPatch;
}

function readVersions(file) {
  const data = readJsonFile(file);
  if (!data || !Array.isArray(data.versions)) return { current: null, versions: [] };
  return { current: data.current ?? null, versions: data.versions };
}

function updateVersions(state, { version, componentCount, exportCount, releaseDate }) {
  const versions = state.versions.map((entry) => ({ ...entry }));
  let entry = versions.find((item) => item.version === version);
  if (!entry) {
    entry = { version, released: null, docsPath: null };
    versions.push(entry);
  }
  entry.components = componentCount;
  entry.exports = exportCount;
  if (releaseDate) {
    entry.released = releaseDate;
    entry.docsPath = `versions/v${version}/`;
  }
  versions.sort((a, b) => compareVersions(b.version, a.version));
  return {
    current: version,
    versions: versions.map((item) => ({
      version: item.version,
      status: item.version === version ? 'current' : item.released ? 'released' : 'unreleased',
      released: item.released ?? null,
      docsPath: item.docsPath ?? null,
      components: item.components ?? null,
      exports: item.exports ?? null,
    })),
  };
}

function renderVersionsPage(versions, ctx) {
  const lines = [
    '<!-- Generated by `npm run docs:build` from component source and JSDoc. Do not edit by hand. -->',
    '',
    '# Versions',
    '',
    'The component docs are versioned with `web/package.json`. `npm run docs:release` freezes the current docs and publishes a browsable snapshot of the docs site under `versions/`.',
    '',
    '| Version | Status | Released | Components | Docs |',
    '| --- | --- | --- | --- | --- |',
  ];
  for (const entry of versions.versions) {
    // Repo-relative on purpose: resolves on GitHub and is rebased onto the site
    // root by `linkResolver` when the site is generated.
    const docs = entry.docsPath
      ? `[snapshot](../../public/component-docs/${entry.docsPath}index.html)`
      : entry.status === 'current'
        ? 'this build'
        : '—';
    lines.push(
      `| v${entry.version} | ${entry.status} | ${entry.released ?? '—'} | ${entry.components ?? '—'} | ${docs} |`,
    );
  }
  lines.push('', '---', '', `[← All components](./README.md) · [Migration guide](./migrations.md)`);
  return lines.join('\n') + '\n';
}

// ─────────────────────────────────────────────────────────────────────────────
// File operations
// ─────────────────────────────────────────────────────────────────────────────

function writeFiles(dir, files) {
  for (const [relative, contents] of Object.entries(files)) {
    const absolute = path.join(dir, relative);
    mkdirSync(path.dirname(absolute), { recursive: true });
    writeFileSync(absolute, contents);
  }
}

function listFiles(dir, base = dir) {
  if (!existsSync(dir)) return [];
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith('.')) continue;
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) found.push(...listFiles(absolute, base));
    else found.push(path.relative(base, absolute));
  }
  return found;
}

/**
 * Published version snapshots (`versions/…`, from `--release`) are permanent:
 * an ordinary build must not rewrite, prune, or flag them as drift.
 */
function isVersionSnapshot(relative) {
  return relative.split(path.sep)[0] === 'versions';
}

function diffFiles(dir, files, label) {
  const problems = [];
  for (const [relative, contents] of Object.entries(files)) {
    const absolute = path.join(dir, relative);
    if (!existsSync(absolute)) problems.push(`missing  ${label}/${relative}`);
    else if (readFileSync(absolute, 'utf8') !== contents) problems.push(`stale    ${label}/${relative}`);
  }
  for (const relative of listFiles(dir)) {
    if (relative in files || isVersionSnapshot(relative)) continue;
    problems.push(`unexpected ${label}/${relative}`);
  }
  return problems;
}

/** Removes generated files that are no longer produced by the registry. */
function pruneGenerated(dir, files) {
  const removed = [];
  for (const relative of listFiles(dir)) {
    if (relative in files || isVersionSnapshot(relative)) continue;
    const absolute = path.join(dir, relative);
    const contents = readFileSync(absolute, 'utf8');
    const managed =
      relative.endsWith('manifest.json') ||
      relative.endsWith('versions.json') ||
      contents.includes('Generated by `npm run docs:build`') ||
      contents.includes('name="generator" content="scripts/docs/generate.mjs"');
    if (!managed) continue;
    rmSync(absolute);
    removed.push(path.join(path.basename(dir), relative));
  }
  return removed;
}

// ─────────────────────────────────────────────────────────────────────────────
// Build
// ─────────────────────────────────────────────────────────────────────────────

export function buildAll(options = {}) {
  const webRoot = options.webRoot ?? WEB_ROOT;
  const version = options.version ?? packageVersion(webRoot);
  const releaseDate = options.release ? options.date ?? today() : null;
  const releases = loadBreakingChanges(webRoot);
  const docsDir = path.join(webRoot, LIBRARY.docsDir);
  const siteDir = path.join(webRoot, LIBRARY.siteDir);

  const { pages, ctx } = buildModel({ webRoot, version, releases, releaseDate });

  // ── Markdown ───────────────────────────────────────────────────────────────
  const docsFiles = {};
  const sitePages = [];

  const indexMarkdown = renderIndexPage(pages, ctx);
  docsFiles['README.md'] = indexMarkdown;
  sitePages.push({
    slug: 'index',
    title: LIBRARY.title,
    markdown: indexMarkdown,
    description: LIBRARY.tagline,
  });

  for (const page of pages) {
    const group = GROUPS.find((item) => item.id === page.group);
    const migrationTarget = page.migration
      ? { slug: page.migration, title: COMPONENTS.find((item) => item.slug === page.migration)?.title ?? page.migration }
      : null;
    const markdown = renderComponentPage(page, page.components, {
      ...ctx,
      group,
      migrationTarget,
    });
    docsFiles[`${page.slug}.md`] = markdown;
    sitePages.push({
      slug: page.slug,
      title: page.title,
      group: page.group,
      status: page.status,
      markdown,
      description: page.usedFor ?? LIBRARY.tagline,
    });
  }

  const migrationsMarkdown = renderMigrationsPage(ctx);
  docsFiles['migrations.md'] = migrationsMarkdown;
  sitePages.push({
    slug: 'migrations',
    title: 'Migration guide',
    markdown: migrationsMarkdown,
    description: 'Breaking changes to the component library and how to migrate.',
  });

  // ── Versions ───────────────────────────────────────────────────────────────
  const versionsFile = path.join(docsDir, 'versions.json');
  const versions = updateVersions(readVersions(versionsFile), {
    version,
    componentCount: ctx.componentCount,
    exportCount: ctx.exportCount,
    releaseDate,
  });
  const versionsMarkdown = renderVersionsPage(versions, ctx);
  docsFiles['versions.md'] = versionsMarkdown;
  sitePages.push({
    slug: 'versions',
    title: 'Versions',
    markdown: versionsMarkdown,
    description: 'Versioned component-library documentation.',
  });

  docsFiles['versions.json'] = JSON.stringify(versions, null, 2) + '\n';
  docsFiles['manifest.json'] = JSON.stringify(toManifest(pages, version), null, 2) + '\n';

  if (releaseDate) {
    docsFiles[`versions/${version}.json`] = JSON.stringify(
      { ...toManifest(pages, version), released: releaseDate },
      null,
      2,
    ) + '\n';
  }

  // ── Static site ────────────────────────────────────────────────────────────
  let siteFiles = {};
  if (options.site !== false) {
    const navPages = pages.map((page) => ({
      slug: page.slug,
      title: page.title,
      group: page.group,
      status: page.status,
    }));
    const siteCtx = { ...ctx, pages: navPages };
    siteFiles = buildSite({ pages: sitePages, ctx: siteCtx, versions });

    if (releaseDate) {
      // Snapshots live at versions/v<version>/, so every root-relative link
      // needs two levels up to reach the site root.
      const snapshot = buildSite({
        pages: sitePages,
        ctx: siteCtx,
        versions,
        prefix: '../../',
        searchBase: '',
      });
      for (const [relative, contents] of Object.entries(snapshot)) {
        // Assets live once at the site root; snapshot pages link up to them.
        if (relative.startsWith('assets/')) continue;
        siteFiles[`versions/v${version}/${relative}`] = contents;
      }
    }
  }

  // A docs build that ships broken links is a failed build: #709 asks for a
  // navigable docs site, not merely for generated files. Published version
  // snapshots that already exist on disk count as valid link targets.
  const publishedPaths = new Set(
    listFiles(path.join(siteDir, 'versions')).map((file) =>
      path.posix.join('versions', file.split(path.sep).join('/')),
    ),
  );
  assertLinksValid(docsFiles, siteFiles, publishedPaths);

  return {
    version,
    docsDir,
    siteDir,
    docsFiles,
    siteFiles,
    ctx,
    versions,
    pages,
    releaseDate,
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Commands
// ─────────────────────────────────────────────────────────────────────────────

function run() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exit(2);
  }

  let result;
  try {
    result = buildAll(options);
  } catch (error) {
    process.stderr.write(`✖ ${error.message}\n`);
    process.exit(1);
  }

  const { docsFiles, siteFiles, docsDir, siteDir, ctx, version } = result;
  const log = (message) => {
    if (!options.quiet) process.stdout.write(`${message}\n`);
  };

  if (options.check) {
    const problems = [
      ...diffFiles(docsDir, docsFiles, LIBRARY.docsDir),
      ...diffFiles(siteDir, siteFiles, LIBRARY.siteDir),
    ];
    if (problems.length) {
      process.stderr.write(
        `✖ Component docs are out of date (${problems.length} file${problems.length === 1 ? '' : 's'}).\n` +
          '  Run `npm run docs:build` and commit the result.\n',
      );
      for (const problem of problems.slice(0, 40)) process.stderr.write(`  ${problem}\n`);
      if (problems.length > 40) process.stderr.write(`  …and ${problems.length - 40} more\n`);
      process.exit(1);
    }
    log(`✔ Component docs are up to date (${Object.keys(docsFiles).length} markdown, ${Object.keys(siteFiles).length} site files).`);
    return;
  }

  writeFiles(docsDir, docsFiles);
  const prunedDocs = pruneGenerated(docsDir, docsFiles);

  let prunedSite = [];
  if (options.site !== false) {
    writeFiles(siteDir, siteFiles);
    prunedSite = pruneGenerated(siteDir, siteFiles);
  }

  log(
    `✔ Documented ${ctx.componentCount} pages / ${ctx.exportCount} exports at v${version}\n` +
      `  ${LIBRARY.docsDir}: ${Object.keys(docsFiles).length} files` +
      (options.site === false ? '' : `\n  ${LIBRARY.siteDir}: ${Object.keys(siteFiles).length} files`),
  );
  for (const removed of [...prunedDocs, ...prunedSite]) log(`  removed ${removed}`);
  if (result.releaseDate) {
    log(`  released v${version} on ${result.releaseDate} (snapshot: versions/v${version}/)`);
  }
}

const isDirectRun = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isDirectRun) run();
