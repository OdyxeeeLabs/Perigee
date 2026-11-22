// site.mjs — renders the markdown produced by `render.mjs` into a standalone,
// static documentation site (plain HTML + two small assets, no runtime
// framework), so the docs can be served by the Next.js app from `public/`,
// published to GitHub Pages, or opened straight from disk.
//
// buildSite() is pure: it returns { path -> contents }. The CLI decides whether
// to write those files or compare them against the committed ones (`--check`).

import { escapeHtml } from './text.mjs';
import { markdownToHtml } from './markdown.mjs';

/** Repo-relative location of the published site; keep in sync with registry. */
const LIBRARY_SITE_DIR = 'public/component-docs';

// ─────────────────────────────────────────────────────────────────────────────
// Link handling
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Rewrites repo-relative markdown links to site-relative HTML links.
 * `prefix` is `''` for the root of the site and `'../'` for versioned snapshots.
 */
export function linkResolver(prefix = '') {
  return (href) => {
    if (!href || /^(https?:|mailto:|tel:|#)/.test(href)) return href;
    const [pathPart, hash] = href.split('#');
    // Markdown may link straight into the published site tree (the version
    // snapshots). Those paths are written repo-relative so they also work on
    // GitHub, and are rebased onto the site root here.
    const siteMarker = `/${LIBRARY_SITE_DIR}/`;
    if (pathPart.includes(siteMarker)) {
      const tail = pathPart.slice(pathPart.indexOf(siteMarker) + siteMarker.length);
      return `${prefix}${tail}${hash ? `#${hash}` : ''}`;
    }
    let target = pathPart;
    if (/(^|\/)(README|index)\.md$/.test(target)) target = 'index.html';
    else if (target.endsWith('.md')) target = `${target.replace(/^.*\//, '').replace(/\.md$/, '')}.html`;
    else if (target === '') target = hash ? '' : 'index.html';
    if (hash && target === '') return `#${hash}`;
    return `${prefix}${target}${hash ? `#${hash}` : ''}`;
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Assets
// ─────────────────────────────────────────────────────────────────────────────

const CSS = `:root {
  color-scheme: dark;
  --bg: #0d1117;
  --panel: #161b22;
  --border: #30363d;
  --text: #e6edf3;
  --muted: #9aa4b2;
  --accent: #33c5e0;
  --accent-contrast: #06272f;
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 16px/1.65 ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
a { color: var(--accent); }
a:focus-visible, button:focus-visible, input:focus-visible, select:focus-visible, pre:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  border-radius: 4px;
}
.skip-link {
  position: absolute;
  left: -9999px;
  top: 0;
  background: var(--accent);
  color: var(--accent-contrast);
  padding: 0.5rem 0.75rem;
  z-index: 10;
}
.skip-link:focus { left: 0.5rem; top: 0.5rem; }
.site-header {
  border-bottom: 1px solid var(--border);
  padding: 1rem 1.25rem;
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem 1.25rem;
  align-items: center;
  background: var(--panel);
}
.site-header .site-title { font-size: 1rem; font-weight: 600; margin: 0; }
.site-header .site-title a { color: var(--text); text-decoration: none; }
.site-header .meta { color: var(--muted); font-size: 0.8rem; margin: 0; }
.site-header .grow { margin-left: auto; }
.control { display: flex; align-items: center; gap: 0.4rem; font-size: 0.8rem; color: var(--muted); }
.control input, .control select {
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.35rem 0.5rem;
  font: inherit;
}
.layout { display: grid; grid-template-columns: 260px minmax(0, 1fr); gap: 2rem; padding: 1.5rem 1.25rem 4rem; max-width: 1200px; margin: 0 auto; }
nav.site-nav { position: sticky; top: 1rem; align-self: start; font-size: 0.9rem; }
nav.site-nav h2 { font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); margin: 1.25rem 0 0.4rem; }
nav.site-nav ul { list-style: none; margin: 0; padding: 0; }
nav.site-nav li { margin: 0.15rem 0; }
nav.site-nav a { text-decoration: none; display: block; padding: 0.2rem 0.4rem; border-radius: 6px; }
nav.site-nav a:hover { background: var(--panel); }
nav.site-nav a[aria-current="page"] { background: var(--panel); color: var(--text); font-weight: 600; }
main { min-width: 0; }
main h1 { font-size: 1.9rem; margin-top: 0.5rem; }
main h2 { font-size: 1.35rem; margin-top: 2.25rem; border-bottom: 1px solid var(--border); padding-bottom: 0.3rem; }
main h3 { font-size: 1.1rem; margin-top: 1.75rem; }
main h4 { font-size: 0.98rem; margin-top: 1.35rem; }
p, li { max-width: 78ch; }
code { background: var(--panel); border: 1px solid var(--border); border-radius: 4px; padding: 0.05rem 0.3rem; font-size: 0.88em; }
pre { background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 0.85rem 1rem; overflow-x: auto; }
pre code { background: none; border: 0; padding: 0; }
.table-wrap { overflow-x: auto; margin: 1rem 0; }
table { border-collapse: collapse; width: 100%; font-size: 0.9rem; }
th, td { border: 1px solid var(--border); padding: 0.5rem 0.65rem; text-align: left; vertical-align: top; }
th { background: var(--panel); }
blockquote { border-left: 3px solid var(--accent); margin: 1rem 0; padding: 0.25rem 1rem; background: var(--panel); border-radius: 0 6px 6px 0; }
blockquote > :first-child { margin-top: 0.4rem; }
blockquote > :last-child { margin-bottom: 0.4rem; }
.toc { background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 0.75rem 1rem; }
.toc h2 { font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); margin: 0 0 0.4rem; border: 0; }
.toc ul { margin: 0; padding-left: 1.1rem; }
.page-footer { margin-top: 3rem; color: var(--muted); font-size: 0.8rem; border-top: 1px solid var(--border); padding-top: 0.75rem; }
#search-results { border: 1px solid var(--border); border-radius: 8px; padding: 0.75rem 1rem; background: var(--panel); margin-bottom: 1.5rem; }
#search-results[hidden] { display: none; }
#search-results ul { margin: 0.4rem 0 0; padding-left: 1.1rem; }
.visually-hidden { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border: 0; }
@media (max-width: 820px) {
  .layout { grid-template-columns: minmax(0, 1fr); }
  nav.site-nav { position: static; }
}
@media (prefers-reduced-motion: reduce) {
  html { scroll-behavior: auto; }
}
`;

const JS = `// Progressive enhancement only: without JS the docs remain fully navigable.
(function () {
  'use strict';

  // ── Component search ───────────────────────────────────────────────────────
  var input = document.getElementById('docs-search');
  var panel = document.getElementById('search-results');
  var list = document.getElementById('search-results-list');
  var status = document.getElementById('search-status');
  if (input && panel && list && status) {
    var index = null;
    var base = panel.getAttribute('data-base') || '';
    var load = function () {
      if (index) return Promise.resolve(index);
      return fetch(base + 'search-index.json', { credentials: 'same-origin' })
        .then(function (response) { return response.json(); })
        .then(function (data) { index = data; return data; });
    };
    var render = function (query) {
      var needle = query.trim().toLowerCase();
      if (!needle) {
        panel.hidden = true;
        list.innerHTML = '';
        status.textContent = '';
        return;
      }
      load().then(function (data) {
        var matches = [];
        data.pages.forEach(function (page) {
          if (page.title.toLowerCase().indexOf(needle) !== -1) {
            matches.push({ href: base + page.href, label: page.title + ' (component)' });
          }
          page.headings.forEach(function (heading) {
            if (heading.text.toLowerCase().indexOf(needle) !== -1) {
              matches.push({ href: base + page.href + '#' + heading.id, label: page.title + ' › ' + heading.text });
            }
          });
        });
        var top = matches.slice(0, 12);
        list.innerHTML = top.map(function (match) {
          return '<li><a href="' + match.href + '">' + match.label.replace(/[<>&]/g, '') + '</a></li>';
        }).join('');
        panel.hidden = top.length === 0;
        status.textContent = top.length === 0
          ? 'No documentation matched "' + query.trim() + '".'
          : matches.length + ' result' + (matches.length === 1 ? '' : 's') + ' for "' + query.trim() + '".';
      });
    };
    var timer = null;
    input.addEventListener('input', function () {
      window.clearTimeout(timer);
      timer = window.setTimeout(function () { render(input.value); }, 120);
    });
    input.addEventListener('keydown', function (event) {
      if (event.key === 'Escape') { input.value = ''; render(''); }
    });
  }

  // ── Version selector ───────────────────────────────────────────────────────
  var versionSelect = document.getElementById('version-select');
  if (versionSelect) {
    versionSelect.addEventListener('change', function () {
      if (versionSelect.value) window.location.href = versionSelect.value;
    });
  }

  // ── Copy button for code blocks ────────────────────────────────────────────
  Array.prototype.forEach.call(document.querySelectorAll('main pre'), function (block, index) {
    var code = block.querySelector('code');
    if (!code || !navigator.clipboard) return;
    var id = 'code-' + index;
    block.setAttribute('id', id);
    var button = document.createElement('button');
    button.type = 'button';
    button.textContent = 'Copy';
    button.className = 'copy-button';
    button.addEventListener('click', function () {
      navigator.clipboard.writeText(code.textContent || '').then(function () {
        button.textContent = 'Copied';
        window.setTimeout(function () { button.textContent = 'Copy'; }, 1500);
      });
    });
    block.parentNode.insertBefore(button, block);
  });
})();
`;

// ─────────────────────────────────────────────────────────────────────────────
// Layout
// ─────────────────────────────────────────────────────────────────────────────

function navHtml(ctx, activeSlug, prefix) {
  const groups = ctx.groups
    .map((group) => {
      const pages = ctx.pages.filter((page) => page.group === group.id);
      if (!pages.length) return '';
      const links = pages
        .map((page) => {
          const current = page.slug === activeSlug ? ' aria-current="page"' : '';
          return `<li><a href="${prefix}${page.slug}.html"${current}>${escapeHtml(page.title)}</a></li>`;
        })
        .join('');
      return `<h2>${escapeHtml(group.title)}</h2><ul>${links}</ul>`;
    })
    .join('');

  return `<nav class="site-nav" aria-label="Component library">
<h2>Overview</h2>
<ul>
<li><a href="${prefix}index.html"${activeSlug === 'index' ? ' aria-current="page"' : ''}>All components</a></li>
<li><a href="${prefix}migrations.html"${activeSlug === 'migrations' ? ' aria-current="page"' : ''}>Migration guide</a></li>
<li><a href="${prefix}versions.html"${activeSlug === 'versions' ? ' aria-current="page"' : ''}>Versions</a></li>
</ul>
${groups}
</nav>`;
}

function versionControl(versions, prefix) {
  const entries = versions?.versions ?? [];
  const hasSnapshots = entries.some((entry) => entry.docsPath);
  if (!hasSnapshots && entries.length <= 1) {
    return `<p class="meta">v${escapeHtml(versions?.current ?? '0.1.0')}</p>`;
  }
  const options = entries
    .map((entry) => {
      const selected = entry.version === versions.current ? ' selected' : '';
      const isCurrent = entry.version === versions.current;
      // The current version always links to the live docs; older versions link
      // to their published snapshot, when one exists.
      if (isCurrent) {
        return `<option value="${escapeHtml(`${prefix}index.html`)}"${selected}>v${escapeHtml(entry.version)} (current)</option>`;
      }
      if (entry.docsPath) {
        return `<option value="${escapeHtml(`${prefix}${entry.docsPath}index.html`)}">v${escapeHtml(entry.version)}</option>`;
      }
      return `<option disabled>v${escapeHtml(entry.version)} (snapshot not published)</option>`;
    })
    .join('');
  return `<div class="control">
<label for="version-select">Version</label>
<select id="version-select" data-navigate>${options}</select>
</div>`;
}

function page({ title, description, content, headings, ctx, activeSlug, prefix, versions, searchBase }) {
  const toc = headings
    .filter((heading) => heading.level === 2 || heading.level === 3)
    .slice(0, 14);
  const tocHtml = toc.length
    ? `<nav class="toc" aria-label="On this page"><h2>On this page</h2><ul>${toc
        .map((heading) => `<li><a href="#${heading.id}">${escapeHtml(heading.text)}</a></li>`)
        .join('')}</ul></nav>`
    : '';

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${escapeHtml(title)} · ${escapeHtml(ctx.library.title)}</title>
<meta name="description" content="${escapeHtml(description)}" />
<meta name="generator" content="scripts/docs/generate.mjs" />
<link rel="stylesheet" href="${prefix}assets/docs.css" />
</head>
<body>
<a class="skip-link" href="#content">Skip to content</a>
<header class="site-header">
<p class="site-title"><a href="${prefix}index.html">${escapeHtml(ctx.library.title)}</a></p>
<p class="meta">Component library docs · generated from source</p>
<div class="grow"></div>
${versionControl(versions, prefix)}
<div class="control">
<label for="docs-search">Search docs</label>
<input id="docs-search" type="search" autocomplete="off" placeholder="Button, accessibility…" aria-describedby="search-status" />
</div>
</header>
<div class="layout">
${navHtml(ctx, activeSlug, prefix)}
<main id="content" tabindex="-1">
<div id="search-results" hidden data-base="${searchBase}">
<strong>Search results</strong>
<p id="search-status" role="status" aria-live="polite"></p>
<ul id="search-results-list"></ul>
</div>
${tocHtml}
${content}
<p class="page-footer">Generated from v${escapeHtml(ctx.version)} by <code>npm run docs:build</code>. Edit the component source or <code>scripts/docs/registry.mjs</code>, never this file.</p>
</main>
</div>
<script src="${prefix}assets/docs.js" defer></script>
</body>
</html>
`;
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Renders the whole site.
 *
 * @param {object} options
 * @param {Array<{slug:string,title:string,group:string,status:string,markdown:string}>} options.pages
 * @param {object} options.ctx registry + version context (must include `pages` for nav)
 * @param {{ current:string, versions:Array<object> }} options.versions
 * @param {string} [options.prefix] link prefix ('' at the root, '../' in snapshots)
 * @param {string} [options.searchBase] where this page's `search-index.json` lives
 * @returns {Record<string, string>} relative path → file contents
 */
export function buildSite({ pages, ctx, versions, prefix = '', searchBase = null }) {
  const files = {};
  const resolveLink = linkResolver(prefix);
  const headingsByPage = {};

  for (const page_ of pages) {
    const { html, headings } = markdownToHtml(page_.markdown, { resolveLink });
    headingsByPage[page_.slug] = { title: page_.title, href: `${page_.slug}.html`, headings };
    files[`${page_.slug}.html`] = render_({
      page: page_,
      html,
      headings,
      ctx,
      versions,
      prefix,
      searchBase: searchBase ?? prefix,
    });
  }

  files['assets/docs.css'] = CSS;
  files['assets/docs.js'] = JS;
  files['versions.json'] = JSON.stringify(
    {
      current: versions.current,
      versions: versions.versions.map((entry) => ({
        version: entry.version,
        status: entry.status,
        released: entry.released ?? null,
        docsPath: entry.docsPath ?? null,
      })),
    },
    null,
    2,
  ) + '\n';

  files['search-index.json'] = JSON.stringify(
    {
      version: ctx.version,
      pages: Object.entries(headingsByPage).map(([slug, data]) => ({
        slug,
        title: data.title,
        href: data.href,
        headings: data.headings.map(({ id, text, level }) => ({ id, text, level })),
      })),
    },
    null,
    2,
  ) + '\n';

  return files;

  function render_({ page: pageModel, html, headings, ctx: context, versions: versionsModel, prefix: prefixValue, searchBase: base }) {
    const title = pageModel.title ?? 'Documentation';
    const description = pageModel.description ?? context.library.tagline;
    return page({
      title,
      description,
      content: html,
      headings,
      ctx: context,
      activeSlug: pageModel.slug,
      prefix: prefixValue,
      versions: versionsModel,
      searchBase: base,
    });
  }
}
