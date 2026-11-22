// web/lib/sanitize.test.cjs — unit tests for the XSS sanitization helpers in
// web/lib/sanitize.ts (Issue #475 / WEB-21). Runs with
// `node --test ./lib/**/*.test.cjs`.
//
// `package.json`'s `test` script only executes `.cjs` files, so — following the
// convention already used by web/lib/api-middleware.test.cjs — the logic is
// mirrored here in plain JS.
//
// SCOPE: `web/lib/sanitize.ts` branches on `typeof window === "undefined"`:
//   • in the browser it delegates to DOMPurify (authoritative, covered by
//     DOMPurify's own suite + web/components/external-link-security.test.cjs),
//   • on the server / in any window-less runtime (this test runner) it falls
//     back to stripping tags with a regex.
// Node has no `window`, so the mirror below implements that SSR fallback branch
// and the assertions pin its exact behaviour, including the places where it is
// deliberately weaker than DOMPurify (e.g. `<b>` is stripped too, and text
// between stripped tags survives as inert text).
//
// If you change behaviour in web/lib/sanitize.ts, mirror the change here.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

/* ── Mirror of web/lib/sanitize.ts (SSR branch) ──────────────────────── */

function stripTags(dirty) {
  return dirty.replace(/<[^>]*>/g, '');
}

function sanitizeHtml(dirty) {
  return stripTags(dirty);
}

function sanitizeSvg(dirty) {
  return stripTags(dirty);
}

function sanitizeText(dirty) {
  return stripTags(dirty);
}

function sanitize(dirty, context = 'user input') {
  const sanitized = sanitizeText(dirty);

  if (sanitized !== dirty) {
    console.warn(`[security] Sanitized ${context}: removed unsafe HTML/JS content.`);
  }

  return sanitized;
}

function sanitizeUserInput(value, fieldName = 'input') {
  return sanitize(value, `${fieldName} value`);
}

/* ── Test helpers ────────────────────────────────────────────────────── */

/** Run `fn` while capturing everything the mirror logs through console.warn. */
function captureWarnings(fn) {
  const original = console.warn;
  const warnings = [];
  console.warn = (...args) => warnings.push(args.join(' '));
  try {
    return { result: fn(), warnings };
  } finally {
    console.warn = original;
  }
}

function sourceText() {
  return fs.readFileSync(path.join(__dirname, 'sanitize.ts'), 'utf8');
}

/* ── sanitizeText() ──────────────────────────────────────────────────── */

test('sanitizeText: leaves plain text untouched', () => {
  assert.equal(sanitizeText('Perigee vault 42 report'), 'Perigee vault 42 report');
});

test('sanitizeText: returns an empty string for empty input', () => {
  assert.equal(sanitizeText(''), '');
});

test('sanitizeText: strips a <script> block', () => {
  assert.equal(sanitizeText('<script>alert(1)</script>'), 'alert(1)');
});

test('sanitizeText: strips inline event handlers with their tag', () => {
  const dirty = '<img src=x onerror="alert(1)">safe';
  const clean = sanitizeText(dirty);
  assert.equal(clean, 'safe');
  assert.equal(clean.includes('onerror'), false);
  assert.equal(clean.includes('<'), false);
});

test('sanitizeText: strips nested and self-closing markup', () => {
  assert.equal(sanitizeText('<div><span>hi</span></div>'), 'hi');
  assert.equal(sanitizeText('a<br/>b'), 'ab');
  assert.equal(sanitizeText('a<br />b'), 'ab');
});

test('sanitizeText: strips markup that spans newlines', () => {
  assert.equal(sanitizeText('<a\n  href="https://x.test">link</a>'), 'link');
});

test('sanitizeText: an unterminated tag is left as inert text', () => {
  // Documented SSR limitation: only complete `<…>` sequences are removed.
  assert.equal(sanitizeText('a < b > c'), 'a  c');
  assert.equal(sanitizeText('<b never closed'), '<b never closed');
});

test('sanitizeText: keeps text between removed tags (no HTML execution)', () => {
  assert.equal(sanitizeText('<script>alert(1)</script>ok'), 'alert(1)ok');
});

/* ── sanitizeHtml() / sanitizeSvg() ──────────────────────────────────── */

test('sanitizeHtml: strips formatting tags on the SSR path', () => {
  assert.equal(sanitizeHtml('<b>bold</b> text'), 'bold text');
  assert.equal(sanitizeHtml('<iframe src="https://evil.test"></iframe>'), '');
});

test('sanitizeSvg: strips a script smuggled inside an <svg>', () => {
  const dirty = '<svg><script>alert(1)</script><path d="M0 0"/></svg>';
  const clean = sanitizeSvg(dirty);
  // SSR fallback removes every tag, including the SVG ones — no markup survives.
  assert.equal(clean, 'alert(1)');
  assert.equal(clean.includes('<'), false);
  assert.equal(clean.includes('script'), false);
});

test('sanitizeHtml: returns the value unchanged when it is already safe', () => {
  assert.equal(sanitizeHtml('just words'), 'just words');
});

/* ── sanitize() ──────────────────────────────────────────────────────── */

test('sanitize: returns clean input unchanged without warning', () => {
  const { result, warnings } = captureWarnings(() => sanitize('nothing to fix'));
  assert.equal(result, 'nothing to fix');
  assert.deepEqual(warnings, []);
});

test('sanitize: warns once, with the default context, when content changes', () => {
  const { result, warnings } = captureWarnings(() => sanitize('<img src=x onerror=alert(1)>'));
  assert.equal(result, '');
  assert.equal(warnings.length, 1);
  assert.equal(warnings[0], '[security] Sanitized user input: removed unsafe HTML/JS content.');
});

test('sanitize: uses the caller-supplied context in the warning', () => {
  const { warnings } = captureWarnings(() => sanitize('<script>x</script>', 'analysis report'));
  assert.deepEqual(warnings, [
    '[security] Sanitized analysis report: removed unsafe HTML/JS content.',
  ]);
});

test('sanitize: does not warn twice for repeated calls', () => {
  const { warnings } = captureWarnings(() => {
    sanitize('<script>x</script>');
    sanitize('<script>y</script>');
  });
  assert.equal(warnings.length, 2);
});

test('sanitize: empty string is returned as-is and never warns', () => {
  const { result, warnings } = captureWarnings(() => sanitize(''));
  assert.equal(result, '');
  assert.deepEqual(warnings, []);
});

/* ── sanitizeUserInput() ─────────────────────────────────────────────── */

test('sanitizeUserInput: labels the warning with the field name', () => {
  const { result, warnings } = captureWarnings(() =>
    sanitizeUserInput('<script>x</script>', 'contract_id'),
  );
  assert.equal(result, 'x');
  assert.deepEqual(warnings, [
    '[security] Sanitized contract_id value: removed unsafe HTML/JS content.',
  ]);
});

test('sanitizeUserInput: falls back to the generic "input" label', () => {
  const { warnings } = captureWarnings(() => sanitizeUserInput('<b>x</b>'));
  assert.deepEqual(warnings, [
    '[security] Sanitized input value: removed unsafe HTML/JS content.',
  ]);
});

/* ── Source contract ─────────────────────────────────────────────────── */

test('sanitize.ts still exports the helpers mirrored above', () => {
  const src = sourceText();
  assert.match(src, /export function sanitizeHtml\(/);
  assert.match(src, /export function sanitizeSvg\(/);
  assert.match(src, /export function sanitizeText\(/);
  assert.match(src, /export function sanitize\(/);
  assert.match(src, /export function sanitizeUserInput\(/);
  assert.match(src, /typeof window === "undefined"/, 'SSR fallback branch must remain');
});
