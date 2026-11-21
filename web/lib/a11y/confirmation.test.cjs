// Dependency-free unit tests for the shared destructive-confirmation wording
// (issue #845 / WEB-44). The implementation is TypeScript, so the pure logic
// is mirrored here in the same style as web/lib/a11y/focus.test.cjs.

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

function confirmationTitle(action) {
  return `${action.trim().replace(/[?.!]+$/, "")}?`;
}

function confirmationDescription({ action, target, consequence }) {
  const verb = action.trim().replace(/[?.!]+$/, "");
  return `${verb} ${target}? ${consequence.trim()} This cannot be undone.`;
}

function confirmationAnnouncement(action) {
  return `Confirmation required. ${confirmationDescription(action)}`;
}

test("confirmation title is a single question", () => {
  assert.equal(confirmationTitle("Revoke access for"), "Revoke access for?");
  assert.equal(confirmationTitle("Revoke access for?"), "Revoke access for?");
});

test("confirmation description names both the target and the consequence", () => {
  const message = confirmationDescription({
    action: "Remove manager",
    target: "Acme Wealth",
    consequence: "They will lose access to all client vaults.",
  });
  assert.match(message, /Remove manager Acme Wealth\?/);
  assert.match(message, /They will lose access to all client vaults\./);
  assert.match(message, /This cannot be undone\./);
});

test("announcement is a concise, self-contained sentence", () => {
  const action = {
    action: "Clear",
    target: "saved settings",
    consequence: "Every preference returns to its default.",
  };
  const announcement = confirmationAnnouncement(action);
  assert.match(announcement, /^Confirmation required\./);
  assert.ok(announcement.includes(action.target));
  assert.ok(announcement.includes(action.consequence));
});

test("the component wires focus, Escape and explicit confirm", () => {
  const src = fs.readFileSync(
    path.join(__dirname, "..", "..", "components", "ui", "ConfirmDialog.tsx"),
    "utf8",
  );
  assert.match(src, /role="dialog"/, "modal dialog role");
  assert.match(src, /aria-modal="true"/);
  assert.match(src, /aria-labelledby=/, "named dialog");
  assert.match(src, /aria-describedby=/, "description is associated");
  assert.match(src, /initialFocus: "container"/, "focus never starts on the destructive action");
  assert.match(src, /onEscape: onCancel/, "Escape cancels");
  assert.match(src, /onClick=\{onCancel\}/, "Cancel button cancels");
  assert.match(src, /onClick=\{onConfirm\}/, "only the confirm button runs the action");
  assert.match(src, /<LiveStatus[\s\S]*data-testid="confirm-live-status"/, "screen-reader announcement");
  assert.doesNotMatch(src, /\bfetch\(/, "the dialog itself performs no API call");
});
