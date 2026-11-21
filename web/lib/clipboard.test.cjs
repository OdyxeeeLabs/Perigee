// Tests for the full-value copy action (issue #844 / WEB-43).
// The clipboard helper is dependency-free TypeScript, so its logic is mirrored
// here in the same style as web/lib/utils.test.cjs; the component contract is
// checked against the source because this harness has no DOM renderer.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const read = (...parts) =>
  fs.readFileSync(path.join(__dirname, "..", ...parts), "utf8");

async function copyValue(value, writer, legacyCopy) {
  if (writer) {
    try {
      await writer.writeText(value);
      return "copied";
    } catch {
      // permission denied — try the legacy path
    }
  }
  if (legacyCopy && legacyCopy(value)) return "copied";
  return "manual";
}

function shortenAddress(value, leading = 6, trailing = 6) {
  if (!value || value.length <= leading + trailing) return value;
  return `${value.slice(0, leading)}…${value.slice(-trailing)}`;
}

test("copyValue passes the exact full value to the clipboard", async () => {
  const written = [];
  const FULL = "GABCDEF1234567890XYZQWERTYUIOPASDFGHJKLZXCVBNM234567";
  const result = await copyValue(FULL, {
    writeText: (value) => {
      written.push(value);
      return Promise.resolve();
    },
  });
  assert.equal(result, "copied");
  assert.deepEqual(written, [FULL]);
  assert.equal(written[0], FULL, "no label, prefix or truncation may be copied");
});

test("a denied clipboard permission falls back to the legacy path", async () => {
  let attempted = null;
  const result = await copyValue(
    "0x1234",
    { writeText: () => Promise.reject(new Error("NotAllowedError")) },
    (value) => {
      attempted = value;
      return true;
    },
  );
  assert.equal(result, "copied");
  assert.equal(attempted, "0x1234");
});

test("when both clipboard paths fail the caller gets a manual-copy result", async () => {
  const result = await copyValue(
    "GCALLED",
    { writeText: () => Promise.reject(new Error("NotAllowedError")) },
    () => false,
  );
  assert.equal(result, "manual");
});

test("shortened display text keeps the full value for the copy action", () => {
  const full = "G1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234";
  assert.equal(shortenAddress(full), `G12345…EF1234`);
  assert.notEqual(shortenAddress(full), full);
  for (const short of ["", "G123", "123456789012"]) {
    assert.equal(shortenAddress(short), short);
  }
});

test("CopyButton announces status and reveals the full value when copying fails", () => {
  const src = read("components", "ui", "CopyButton.tsx");
  assert.match(src, /aria-label=\{/, "icon-only control has an accessible name");
  assert.match(src, /Copy full \$\{label\}/);
  assert.match(src, /<LiveStatus[\s\S]*data-testid="copy-status"/, "polite screen-reader update");
  assert.match(src, /Copy failed\. Select the full \$\{label\} manually\./);
  assert.match(src, /data-testid="copy-fallback-value"[\s\S]*?\{value\}/, "full value is selectable");
  assert.match(src, /setTimeout\(\(\) => setResult\(null\), feedbackMs\)/, "feedback is temporary");
  assert.match(src, /clearTimeout\(timerRef\.current\)/, "timer is cleaned up");
});

test("copy targets pass full IDs, not their shortened display strings", () => {
  const row = read("components", "VaultRow.tsx");
  assert.match(row, /shortenAddress\(vault\.owner\)/, "shortened text is display only");
  assert.match(row, /<CopyButton value=\{vault\.owner\}/);

  const detail = read("components", "vault", "VaultDetailContent.tsx");
  assert.match(detail, /shortenAddress\(vault\.tokenInfo\.underlyingAddress\)/);
  assert.match(detail, /<CopyButton value=\{vault\.tokenInfo\.underlyingAddress\}/);
  assert.match(detail, /shortenAddress\(vault\.tokenInfo\.shareTokenAddress\)/);
  assert.match(detail, /<CopyButton value=\{vault\.tokenInfo\.shareTokenAddress\}/);

  const history = read("components", "InvocationHistorySidebar.tsx");
  assert.match(history, /<CopyButton value=\{item\.id\}/);
});
