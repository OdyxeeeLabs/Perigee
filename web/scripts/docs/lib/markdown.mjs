// markdown.mjs — minimal, dependency-free markdown → HTML for the docs site.
//
// The docs build deliberately avoids pulling a markdown engine into the repo:
// the generator emits a controlled markdown subset (headings, tables, fenced
// code, lists, blockquotes, raw HTML passthrough), which is all this needs to
// handle. It also lets the site render the hand-written `README.md` and the
// JSDoc prose that flows through unchanged.

import { escapeHtml, slugify } from './text.mjs';

const FENCE_RE = /^```\s*([\w-]*)\s*$/;
const HEADING_RE = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const LIST_RE = /^(\s*)([-*+]|\d+\.)\s+(.*)$/;
const TABLE_SEPARATOR_RE = /^\s*\|?[\s:|-]+\|[\s:|-]*$/;
const HR_RE = /^\s*(-{3,}|\*{3,}|_{3,})\s*$/;

/** Inline markdown: code spans, emphasis, links. HTML is escaped first. */
export function inline(text) {
  const codeSpans = [];
  let out = escapeHtml(text);

  out = out.replace(/``\s?([\s\S]*?)\s?``|`([^`]*?)`/g, (_match, double, single) => {
    codeSpans.push((double ?? single ?? '').replace(/\\\|/g, '|'));
    return `\u0000${codeSpans.length - 1}\u0000`;
  });

  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  out = out.replace(/(^|[^*\w])\*([^*\n]+)\*(?!\*)/g, '$1<em>$2</em>');
  // Underscore emphasis, guarded so `snake_case_names` are left alone.
  out = out.replace(/(^|[^\w])_([^_\n]+)_(?![\w])/g, '$1<em>$2</em>');
  out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_match, label, href) => {
    const external = /^https?:/.test(href);
    const attrs = external ? ' rel="noreferrer noopener"' : '';
    return `<a href="${href}"${attrs}>${label}</a>`;
  });

  return out.replace(/\u0000(\d+)\u0000/g, (_match, index) => `<code>${codeSpans[Number(index)]}</code>`);
}

function splitRow(row) {
  let text = row.trim();
  if (text.startsWith('|')) text = text.slice(1);
  if (text.endsWith('|')) text = text.slice(0, -1);
  return text
    .split(/(?<!\\)\|/)
    .map((cell) => cell.replace(/\\\|/g, '|').trim());
}

function renderTable(rows, renderInline) {
  const [head, ...body] = rows;
  const cells = splitRow(head);
  const headHtml = cells.map((cell) => `<th scope="col">${renderInline(cell)}</th>`).join('');
  const bodyHtml = body
    .map((row) => `<tr>${splitRow(row).map((cell) => `<td>${renderInline(cell)}</td>`).join('')}</tr>`)
    .join('');
  return `<div class="table-wrap"><table><thead><tr>${headHtml}</tr></thead><tbody>${bodyHtml}</tbody></table></div>`;
}

function renderList(lines, start, renderInline) {
  const first = LIST_RE.exec(lines[start]);
  const baseIndent = first[1].length;
  const ordered = /^\d+\.$/.test(first[2]);
  const items = [];
  let current = null;
  let index = start;

  while (index < lines.length) {
    const line = lines[index];
    const match = LIST_RE.exec(line);
    if (match) {
      const indent = match[1].length;
      if (indent < baseIndent) break;
      if (indent === baseIndent && /^\d+\.$/.test(match[2]) === ordered) {
        current = { text: [match[3]], nested: [] };
        items.push(current);
        index += 1;
        continue;
      }
      if (indent > baseIndent && current) {
        const [nestedHtml, next] = renderList(lines, index, renderInline);
        current.nested.push(nestedHtml);
        index = next;
        continue;
      }
      break;
    }
    if (line.trim() === '') {
      const next = lines[index + 1] ?? '';
      const nextMatch = LIST_RE.exec(next);
      if (nextMatch && nextMatch[1].length >= baseIndent) {
        index += 1;
        continue;
      }
      break;
    }
    if (current && /^\s+\S/.test(line)) {
      current.text.push(line.trim());
      index += 1;
      continue;
    }
    break;
  }

  const tag = ordered ? 'ol' : 'ul';
  const html = items
    .map((item) => `<li>${renderInline(item.text.join(' '))}${item.nested.join('')}</li>`)
    .join('');
  return [`<${tag}>${html}</${tag}>`, index];
}

/**
 * Converts markdown to HTML.
 *
 * @param {string} markdown
 * @param {{ resolveLink?: (href: string) => string }} [options]
 * @returns {{ html: string, headings: Array<{ level: number, id: string, text: string }> }}
 */
export function markdownToHtml(markdown, options = {}) {
  const resolveLink = options.resolveLink ?? ((href) => href);
  const lines = String(markdown).replace(/\r\n/g, '\n').split('\n');
  const html = [];
  const headings = [];
  // Ids must be unique per document; a page title and a section can slugify to
  // the same string (a compound page titled `Card` with a `## Card` section).
  const usedIds = new Map();
  let index = 0;

  const uniqueId = (text) => {
    const base = slugify(text.replace(/`/g, '')) || 'section';
    const seen = usedIds.get(base) ?? 0;
    usedIds.set(base, seen + 1);
    return seen === 0 ? base : `${base}-${seen + 1}`;
  };

  const renderInline = (text) =>
    inline(text).replace(/href="([^"]*)"/g, (_match, href) => `href="${resolveLink(href)}"`);

  while (index < lines.length) {
    const line = lines[index];

    if (line.trim() === '') {
      index += 1;
      continue;
    }

    if (/^\s*<!--/.test(line)) {
      while (index < lines.length && !/-->/.test(lines[index])) index += 1;
      index += 1;
      continue;
    }

    const fence = FENCE_RE.exec(line);
    if (fence) {
      const language = fence[1];
      const body = [];
      index += 1;
      while (index < lines.length && !FENCE_RE.test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      index += 1;
      const className = language ? ` class="language-${language}"` : '';
      html.push(`<pre tabindex="0"><code${className}>${escapeHtml(body.join('\n'))}</code></pre>`);
      continue;
    }

    const heading = HEADING_RE.exec(line);
    if (heading) {
      const level = heading[1].length;
      const text = heading[2];
      const id = uniqueId(text);
      headings.push({ level, id, text: text.replace(/`/g, '') });
      html.push(`<h${level} id="${id}">${renderInline(text)}</h${level}>`);
      index += 1;
      continue;
    }

    if (line.trimStart().startsWith('|')) {
      const rows = [];
      while (index < lines.length && lines[index].trimStart().startsWith('|')) {
        if (!TABLE_SEPARATOR_RE.test(lines[index])) rows.push(lines[index]);
        index += 1;
      }
      if (rows.length) html.push(renderTable(rows, renderInline));
      continue;
    }

    if (HR_RE.test(line)) {
      html.push('<hr />');
      index += 1;
      continue;
    }

    if (LIST_RE.test(line)) {
      const [listHtml, next] = renderList(lines, index, renderInline);
      html.push(listHtml);
      index = next;
      continue;
    }

    if (line.startsWith('>')) {
      const quote = [];
      while (index < lines.length && lines[index].startsWith('>')) {
        quote.push(lines[index].replace(/^>\s?/, ''));
        index += 1;
      }
      const inner = markdownToHtml(quote.join('\n'), options).html;
      html.push(`<blockquote>${inner}</blockquote>`);
      continue;
    }

    if (/^\s*</.test(line)) {
      const raw = [];
      while (index < lines.length && lines[index].trim() !== '') {
        raw.push(lines[index]);
        index += 1;
      }
      html.push(raw.join('\n'));
      continue;
    }

    const paragraph = [];
    while (
      index < lines.length &&
      lines[index].trim() !== '' &&
      !FENCE_RE.test(lines[index]) &&
      !HEADING_RE.test(lines[index]) &&
      !LIST_RE.test(lines[index]) &&
      !lines[index].trimStart().startsWith('|') &&
      !lines[index].trimStart().startsWith('<!') &&
      !lines[index].startsWith('>') &&
      !HR_RE.test(lines[index])
    ) {
      paragraph.push(lines[index]);
      index += 1;
    }
    if (paragraph.length) html.push(`<p>${renderInline(paragraph.join(' ').trim())}</p>`);
  }

  return { html: html.join('\n'), headings };
}
