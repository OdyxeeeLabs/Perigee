"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { test, describe } = require("node:test");
const assert = require("node:assert/strict");

const projectRoot = path.resolve(__dirname, "..");
const scannedFiles = [];

function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (
      entry.name === "node_modules" ||
      entry.name === ".next" ||
      entry.name === ".git"
    ) {
      continue;
    }

    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(fullPath);
      continue;
    }

    if (/\.(?:ts|tsx|js|jsx|mdx|html)$/i.test(entry.name)) {
      scannedFiles.push(fullPath);
    }
  }
}

walk(projectRoot);

describe("external link security audit", () => {
  test('all target="_blank" links include rel="noopener noreferrer"', () => {
    const failures = [];

    for (const filePath of scannedFiles) {
      const content = fs.readFileSync(filePath, "utf8");
      const matches = [
        ...content.matchAll(/<a\b[^>]*target\s*=\s*['"]_blank['"][^>]*>/gi),
      ];

      for (const match of matches) {
        const tag = match[0];
        const relMatch = /\brel\s*=\s*['"]([^'"]+)['"]/i.exec(tag);

        if (!relMatch) {
          failures.push(
            `${path.relative(projectRoot, filePath)}: missing rel attribute for target="_blank" link`,
          );
          continue;
        }

        const relTokens = new Set(relMatch[1].toLowerCase().split(/\s+/));
        if (
          !relTokens.has("noopener") ||
          !relTokens.has("noreferrer")
        ) {
          failures.push(
            `${path.relative(projectRoot, filePath)}: rel="${relMatch[1]}" is missing noopener/noreferrer for target="_blank" link`,
          );
        }
      }
    }

    assert.deepEqual(
      failures,
      [],
      `External link audit failed:\n${failures.join("\n")}`,
    );
  });
});
