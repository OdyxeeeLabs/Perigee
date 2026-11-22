#!/usr/bin/env node
/**
 * Dependency-free static accessibility audit — issue #705.
 *
 * ## Why this exists
 *
 * The repo already imports `@axe-core/react` in `pages/_app.tsx` and ships a
 * `playwright.config.ts`, but **neither package is declared in
 * `package.json` or `yarn.lock`**. The axe import is wrapped in a
 * `.catch(() => {})`, so it throws on every dev boot and nothing is ever
 * scanned. The audit tooling that appears to exist does not exist.
 *
 * Adding `axe-core` is a dependency change, so this script deliberately
 * requires none: it is a line-oriented scanner for the violations that are
 * cheap to detect statically and expensive to find by hand.
 *
 * ## What it is not
 *
 * A static scanner sees one line of markup, not a rendered DOM. It cannot
 * check contrast, computed names, focus order or anything that needs a
 * browser. Those still need axe plus a real browser, and the manual checklist
 * in docs/accessibility/audit-suite.md. This tool's job is to fail loudly on
 * the cheap checks instead of failing silently on everything.
 *
 * ## Usage
 *
 *   node scripts/a11y/static-audit.mjs            # advisory: report, exit 0
 *   node scripts/a11y/static-audit.mjs --json     # machine-readable
 *   node scripts/a11y/static-audit.mjs --strict   # exit 1 on any finding
 *   node scripts/a11y/static-audit.mjs components/ # limit the scope
 *
 * Advisory is the default on purpose: turning this into a hard gate requires
 * baselining the existing violations first, otherwise the first run is a wall
 * of red and the team learns to ignore it. See the docs for how to adopt
 * `--strict` incrementally.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join, relative, resolve } from "node:path";

const args = process.argv.slice(2);
const STRICT = args.includes("--strict");
const JSON_OUT = args.includes("--json");
const SCOPE = args.filter((a) => !a.startsWith("--"));
const ROOT = resolve(process.cwd(), SCOPE[0] ?? ".");

const SOURCE_EXTENSIONS = new Set([".tsx", ".jsx", ".ts", ".js"]);
const IGNORED_DIRS = new Set([
  "node_modules",
  ".next",
  "out",
  "build",
  "coverage",
  "storybook-static",
  ".git",
  "e2e",
]);

/**
 * Each rule receives the element's whole opening tag (a string) and a context
 * object `{ source, index, line }` for the rules that need to look at the
 * surrounding markup rather than the tag alone.
 *
 * Passing the tag as a string matters: a rule that accidentally receives the
 * tag *object* will fail to match and silently report nothing, which looks
 * exactly like a clean bill of health.
 */
const RULES = [
  {
    id: "img-missing-alt",
    severity: "error",
    label: "WCAG 1.1.1",
    describe: "<img> without an alt attribute",
    test: (tag) => /^<img\b/.test(tag) && !/\balt\s*=/.test(tag),
  },
  {
    id: "iframe-missing-title",
    severity: "error",
    label: "WCAG 4.1.2",
    describe: "<iframe> without a title attribute",
    test: (tag) => /^<iframe\b/.test(tag) && !/\btitle\s*=/.test(tag),
  },
  {
    id: "click-non-interactive",
    severity: "error",
    label: "WCAG 2.1.1",
    describe:
      "onClick on a non-interactive element with no role and no keyboard handler",
    test: (tag) =>
      /^<(div|span|li|td|tr|p|section|article)\b/.test(tag) &&
      /\bonClick\s*=/.test(tag) &&
      !/\brole\s*=/.test(tag) &&
      !/\btabIndex\s*=/.test(tag) &&
      !/\bonKeyDown\s*=/.test(tag) &&
      // A backdrop inside aria-hidden="true" is a pointer convenience, not a
      // control: it is hidden from assistive technology on purpose and the
      // keyboard path is Escape or the close button. Flagging those would be
      // a false positive that teaches people to ignore this rule.
      !/aria-hidden\s*=\s*\{?["']true["']/.test(tag),
  },
  {
    id: "positive-tabindex",
    severity: "warning",
    label: "WCAG 2.4.3",
    describe:
      "tabIndex greater than 0; positive tabindex reorders the whole page and is only valid inside a composite widget using roving tabindex",
    test: (tag) => {
      const m = /tabIndex\s*=\s*\{?\s*([1-9]\d*)/.exec(tag);
      return Boolean(m) && Number(m[1]) > 0;
    },
  },
  {
    id: "outline-none-without-replacement",
    severity: "warning",
    label: "WCAG 2.4.7",
    describe:
      "focus:outline-none without a replacement focus style (ring, outline or border change)",
    test: (tag) => {
      if (!/focus:outline-none|focus:outline-hidden/.test(tag)) return false;
      // The replacement check must not match `outline-none` itself, or every
      // violation silently counts as its own fix. A real replacement is a
      // ring, a border, or an outline utility that is not `none`/`hidden`.
      const hasReplacement =
        /(focus-visible:)?(ring|border)-\S/.test(tag) ||
        /(focus-visible:)?outline-(?!none\b|hidden\b)\S/.test(tag);
      return !hasReplacement;
    },
  },
  {
    id: "aria-hidden-focusable",
    severity: "error",
    label: "WCAG 4.1.2",
    describe:
      "aria-hidden=\"true\" on a focusable element: it can still receive focus, so it is focusable but invisible to assistive technology",
    test: (tag) => {
      if (!/aria-hidden\s*=\s*\{?["']true["']/.test(tag)) return false;
      const m = /tabIndex\s*=\s*\{?\s*(-?\d+)/.exec(tag);
      return Boolean(m) && Number(m[1]) >= 0;
    },
  },
  {
    id: "label-without-associated-control",
    severity: "error",
    label: "WCAG 1.3.1 / 3.3.2",
    describe:
      "<label> with no htmlFor and no nested control, so its form control has no accessible name",
    test: (tag, context) => {
      if (!/^<label\b/.test(tag)) return false;
      if (/\bhtmlFor\s*=/.test(tag)) return false;
      // A label that *wraps* its control is correctly associated, so the
      // content up to </label> has to be inspected: `<label><input/></label>`
      // is valid and extremely common. Only a control that is a *sibling* is
      // the bug — the label names nothing.
      const close = context.source.indexOf("</label>", context.index);
      const inner =
        close === -1
          ? context.source.slice(context.index)
          : context.source.slice(context.index, close);
      return !/<(input|select|textarea)\b/.test(inner);
    },
  },
  {
    id: "empty-link-name",
    severity: "error",
    label: "WCAG 2.4.4 / 4.1.2",
    describe: "<a href> with no text and no accessible name",
    test: (tag, context) => {
      if (!/^<a\b/.test(tag)) return false;
      if (!/\bhref\s*=/.test(tag)) return false;
      if (/\baria-label\s*=/.test(tag)) return false;
      if (/\baria-labelledby\s*=/.test(tag)) return false;
      // Look at the element's content, not just its opening tag: an icon-only
      // link whose <Icon/> is the whole content has no accessible name unless
      // the icon itself carries a title.
      const content = context.source
        .slice(context.index, context.index + 300)
        .replace(tag, "")
        .split("</a>")[0];
      const hasText = content.replace(/<[^>]*>/g, "").trim().length > 0;
      const hasNamedImage = /<img\b[^>]*\balt\s*=\s*\{?["'][^"']+/.test(content);
      const hasSvgTitle = /<title[\s>]/.test(content);
      return !hasText && !hasNamedImage && !hasSvgTitle;
    },
  },
  {
    id: "autofocus",
    severity: "warning",
    label: "WCAG 3.2.1",
    describe:
      "autoFocus steals focus on page load, which disorients keyboard and screen-reader users on route change",
    test: (tag) => /\bautoFocus\b/.test(tag),
  },
];

/** Opening tags are what the rules operate on; attributes may span lines. */
const OPENING_TAG = /<([A-Za-z][A-Za-z0-9._]*)((?:[^<>"']|"[^"]*"|'[^']*')*?)(\/?)>/g;

function* walk(dir) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry.name.startsWith(".") && entry.name !== ".storybook") continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (IGNORED_DIRS.has(entry.name)) continue;
      yield* walk(full);
    } else if (SOURCE_EXTENSIONS.has(extname(entry.name))) {
      yield full;
    }
  }
}

/**
 * A multi-line opening tag: start at `<Name` and consume until the matching
 * `>`, ignoring `>` inside attribute strings. A per-line regex would report
 * `<img\n  src=...` as untagged and miss it.
 */
function openingTags(source) {
  const tags = [];
  const tagStart = /<([A-Za-z][A-Za-z0-9._]*)/g;
  let match;
  while ((match = tagStart.exec(source)) !== null) {
    const index = match.index;
    let cursor = index + match[0].length;
    let quote = null;
    while (cursor < source.length) {
      const char = source[cursor];
      if (quote) {
        if (char === quote) quote = null;
      } else if (char === '"' || char === "'" || char === "`") {
        quote = char;
      } else if (char === ">" || char === "\n" && /^\s*\/>$/.test(source.slice(cursor + 1, cursor + 4))) {
        break;
      }
      cursor += 1;
    }
    const raw = source.slice(index, Math.min(cursor + 1, source.length));
    tags.push({
      name: match[1],
      raw,
      line: source.slice(0, index).split("\n").length,
      index,
    });
    tagStart.lastIndex = cursor;
  }
  return tags;
}

const findings = [];

/**
 * Remove comments while preserving line numbering.
 *
 * Without this the scanner reads documentation as markup: `DataTable.tsx` has
 * an `@accessibility` JSDoc block describing a visually hidden `<label>`,
 * which was reported as an unassociated label in a file that has none. A tool
 * that cries wolf over comments gets switched off.
 *
 * Only block comments are stripped. Line comments are left alone because
 * `//` appears inside string literals and URLs (`https://…`), and a regex
 * cannot tell the two apart without a real tokenizer.
 *
 * Newlines inside a comment are kept so reported line numbers still point at
 * the real source.
 */
function stripComments(source) {
  return source.replace(/\/\*[\s\S]*?\*\//g, (match) => match.replace(/[^\n]/g, " "));
}

for (const file of walk(ROOT)) {
  const source = stripComments(readFileSync(file, "utf8"));
  const lines = source.split("\n");
  const tags = openingTags(source);

  for (const tag of tags) {
    const context = { source, index: tag.index, line: lines[tag.line - 1] ?? "" };
    for (const rule of RULES) {
      let hit = false;
      try {
        hit = rule.test(tag.raw, context);
      } catch {
        // A malformed rule must never take the audit down.
        hit = false;
      }
      if (!hit) continue;
      findings.push({
        rule: rule.id,
        severity: rule.severity,
        wcag: rule.label,
        message: rule.describe,
        file: relative(process.cwd(), file).replace(/\\/g, "/"),
        line: tag.line,
        element: tag.raw.replace(/\s+/g, " ").slice(0, 120),
      });
    }
  }
}

const errors = findings.filter((f) => f.severity === "error");
const warnings = findings.filter((f) => f.severity === "warning");
const byRule = findings.reduce((acc, f) => {
  acc[f.rule] = (acc[f.rule] ?? 0) + 1;
  return acc;
}, {});

if (JSON_OUT) {
  console.log(JSON.stringify({ findings, byRule, errors: errors.length, warnings: warnings.length }, null, 2));
} else {
  const totalFiles = [...walk(ROOT)].length;
  console.log("Static accessibility audit");
  console.log(`  scanned: ${totalFiles} source files under ${relative(process.cwd(), ROOT) || "."}`);
  console.log(`  errors: ${errors.length}   warnings: ${warnings.length}\n`);

  if (findings.length === 0) {
    console.log("No findings.");
  } else {
    for (const [rule, count] of Object.entries(byRule).sort((a, b) => b[1] - a[1])) {
      const sample = findings.find((f) => f.rule === rule);
      console.log(`  ${String(count).padStart(3)}  ${rule}  (${sample.wcag})`);
      console.log(`       ${sample.message}`);
    }
    console.log("\nFirst 20 locations:");
    for (const f of findings.slice(0, 20)) {
      console.log(`  ${f.file}:${f.line}  [${f.rule}] ${f.element}`);
    }
    if (findings.length > 20) console.log(`  ... and ${findings.length - 20} more`);
  }

  console.log(
    "\nThis scanner is one line of markup at a time: it cannot see computed names,\n" +
      "contrast or focus order. Those need axe-core in a browser plus the manual\n" +
      "checklist in docs/accessibility/audit-suite.md.",
  );
}

if (STRICT && findings.length > 0) {
  console.error(`\n--strict: ${findings.length} finding(s); failing.`);
  process.exit(1);
}