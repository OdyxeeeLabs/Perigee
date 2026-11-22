// contrast.test.cjs — WCAG 2.1 contrast maths and the design tokens in
// styles/variables.css (issue #696).
//
// Runs with: node --test ./lib/a11y/contrast.test.cjs
//
// The functions are re-implemented here in the same style as
// __tests__/config.test.cjs: this runner executes plain CommonJS, so the
// TypeScript source is mirrored rather than transpiled.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const MIN_TEXT_CONTRAST = 4.5;
const MIN_NON_TEXT_CONTRAST = 3;

function parseHexColor(value) {
  const hex = String(value).trim().replace(/^#/, "");

  if (/^[0-9a-f]{3}$/i.test(hex)) {
    return {
      r: parseInt(hex[0] + hex[0], 16),
      g: parseInt(hex[1] + hex[1], 16),
      b: parseInt(hex[2] + hex[2], 16),
    };
  }
  if (/^[0-9a-f]{6}$/i.test(hex)) {
    return {
      r: parseInt(hex.slice(0, 2), 16),
      g: parseInt(hex.slice(2, 4), 16),
      b: parseInt(hex.slice(4, 6), 16),
    };
  }
  return null;
}

function relativeLuminance({ r, g, b }) {
  const channel = (raw) => {
    const c = raw / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrastRatio(foreground, background) {
  const fg = parseHexColor(foreground);
  const bg = parseHexColor(background);
  if (!fg || !bg) return null;
  const lighter = Math.max(relativeLuminance(fg), relativeLuminance(bg));
  const darker = Math.min(relativeLuminance(fg), relativeLuminance(bg));
  return (lighter + 0.05) / (darker + 0.05);
}

function readVariables() {
  const file = path.join(__dirname, "..", "..", "styles", "variables.css");
  return fs.readFileSync(file, "utf8");
}

function tokenValue(css, name) {
  const match = css.match(new RegExp(`--${name}:\\s*([^;]+);`));
  return match ? match[1].trim() : null;
}

// ── parsing ─────────────────────────────────────────────────────────────────

test("parseHexColor handles 3, 6 digit and missing-# forms", () => {
  assert.deepEqual(parseHexColor("#fff"), { r: 255, g: 255, b: 255 });
  assert.deepEqual(parseHexColor("ffffff"), { r: 255, g: 255, b: 255 });
  assert.deepEqual(parseHexColor("  #2563EB "), { r: 37, g: 99, b: 235 });
});

test("parseHexColor returns null for values that are not hex colours", () => {
  for (const bad of ["", "#12", "rebeccapurple", "#gggggg", "rgb(0,0,0)"]) {
    assert.equal(parseHexColor(bad), null, `expected null for ${JSON.stringify(bad)}`);
  }
});

// ── known ratios (WCAG reference pairs) ─────────────────────────────────────

test("black on white is the 21:1 maximum", () => {
  const ratio = contrastRatio("#000000", "#ffffff");
  assert.ok(Math.abs(ratio - 21) < 0.01, `got ${ratio}`);
});

test("identical colours have a ratio of 1", () => {
  assert.ok(Math.abs(contrastRatio("#3b82f6", "#3b82f6") - 1) < 0.001);
});

test("#767676 on white is the classic 4.54:1 AA boundary", () => {
  const ratio = contrastRatio("#767676", "#ffffff");
  assert.ok(ratio > 4.5 && ratio < 4.6, `got ${ratio}`);
});

test("#2563eb on white is 5.17:1 as documented in variables.css", () => {
  const ratio = contrastRatio("#2563eb", "#ffffff");
  assert.ok(ratio > 5.1 && ratio < 5.25, `got ${ratio}`);
});

test("ratio is symmetric in its arguments", () => {
  const a = contrastRatio("#b45309", "#ffffff");
  const b = contrastRatio("#ffffff", "#b45309");
  assert.ok(Math.abs(a - b) < 1e-9);
});

test("an unparsable colour yields null rather than a misleading number", () => {
  assert.equal(contrastRatio("nope", "#ffffff"), null);
});

// ── the shipped design tokens ───────────────────────────────────────────────

test("light-theme text tokens meet the 4.5:1 text minimum on --color-background", () => {
  const css = readVariables();
  const background = tokenValue(css, "color-background");
  assert.ok(background, "--color-background must be defined");

  for (const token of ["color-text", "color-text-muted", "color-primary", "color-secondary", "color-danger"]) {
    const value = tokenValue(css, token);
    assert.ok(value, `--${token} must be defined`);
    const ratio = contrastRatio(value, background);
    assert.ok(
      ratio >= MIN_TEXT_CONTRAST,
      `--${token} (${value}) is ${ratio.toFixed(2)}:1 on ${background}, needs ${MIN_TEXT_CONTRAST}:1`,
    );
  }
});

test("non-text tokens (focus ring, borders) meet the 3:1 minimum", () => {
  const css = readVariables();
  const background = tokenValue(css, "color-background");

  for (const token of ["color-border", "color-border-subtle"]) {
    const value = tokenValue(css, token);
    assert.ok(value, `--${token} must be defined`);
    const ratio = contrastRatio(value, background);
    assert.ok(
      ratio >= MIN_NON_TEXT_CONTRAST,
      `--${token} (${value}) is ${ratio.toFixed(2)}:1, needs ${MIN_NON_TEXT_CONTRAST}:1`,
    );
  }
});

test("a focus ring token is defined and is visible against both themes", () => {
  const css = readVariables();
  const ring = tokenValue(css, "focus-ring-color");
  assert.ok(ring, "--focus-ring-color must be defined (issue #696)");

  const lightBg = tokenValue(css, "color-background");
  const darkBg = tokenValue(css, "color-background-dark") ?? "#020617";

  assert.ok(
    contrastRatio(ring, lightBg) >= MIN_NON_TEXT_CONTRAST,
    `focus ring is not visible on the light background`,
  );
  assert.ok(
    contrastRatio(ring, darkBg) >= MIN_NON_TEXT_CONTRAST,
    `focus ring is not visible on the dark background`,
  );
});
