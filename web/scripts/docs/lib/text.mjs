// text.mjs — string helpers shared by the component-docs generator.
//
// Kept dependency-free on purpose: the generator runs in CI (and inside the
// docs workflow) where only the repo's devDependencies are guaranteed.

/** `DataTable` -> `data-table`, `LiveStatus` -> `live-status`, `Button` -> `button`. */
export function slugify(name) {
  return String(name)
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1-$2')
    .replace(/[^a-zA-Z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .toLowerCase();
}

/** Escape a cell value for a GitHub-flavoured markdown table. */
export function mdCell(value) {
  return String(value ?? '')
    .replace(/\r?\n+/g, ' ')
    .replace(/\|/g, '\\|')
    .trim();
}

/** Inline code span, or an em dash placeholder when there is no value. */
export function mdCode(value) {
  if (value === undefined || value === null || value === '') return '—';
  const text = String(value).replace(/\r?\n+/g, ' ').trim();
  // Backtick-containing types (rare) fall back to a fenced-safe double backtick.
  const fence = text.includes('`') ? '`` ' : '`';
  const suffix = text.includes('`') ? ' ``' : '`';
  return `${fence}${text}${suffix}`;
}

/** Escape text for HTML output. */
export function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Remove the shared leading indentation from a JSDoc body so that prose and
 * code fences inside `@example` tags render cleanly in markdown.
 */
export function dedent(text) {
  const lines = String(text ?? '').replace(/\t/g, '  ').split('\n');
  while (lines.length && lines[0].trim() === '') lines.shift();
  while (lines.length && lines[lines.length - 1].trim() === '') lines.pop();
  const indents = lines
    .filter((line) => line.trim() !== '')
    .map((line) => line.match(/^ */)[0].length);
  const shared = indents.length ? Math.min(...indents) : 0;
  return lines.map((line) => line.slice(shared)).join('\n');
}

/** Collapse a JSDoc comment body to a single line for tables / metadata. */
export function oneLine(text) {
  return String(text ?? '')
    .replace(/\s+/g, ' ')
    .trim();
}
