"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const WEB_ROOT = path.join(__dirname, "..");
const read = (relative) => fs.readFileSync(path.join(WEB_ROOT, relative), "utf8");

const FIELDS = {
  currency: {
    label: "Currency",
    tab: "display",
    validate: (value) => (["USD", "EUR", "XLM"].includes(value) ? null : "Choose a supported currency."),
  },
  slippage: {
    label: "Slippage tolerance",
    tab: "trading",
    validate: (value) => {
      const parsed = Number(value);
      if (!value.trim() || !Number.isFinite(parsed)) return "Enter a number.";
      if (parsed < 0.1 || parsed > 5) return "Enter a value between 0.1% and 5%.";
      return null;
    },
  },
  "custom-rpc": {
    label: "Custom RPC endpoint",
    tab: "network",
    validate: (value) => {
      if (!value.trim()) return null;
      try {
        const url = new URL(value);
        return url.protocol === "http:" || url.protocol === "https:"
          ? null
          : "Use an http:// or https:// URL.";
      } catch {
        return "Enter a valid URL.";
      }
    },
  },
};

function validateValues(values, settingsFieldValues) {
  const errors = [];
  for (const [id, definition] of Object.entries(FIELDS)) {
    const message = definition.validate(values[id] ?? settingsFieldValues[id] ?? "");
    if (message) errors.push({ id, message, label: definition.label });
  }
  return errors;
}

function normalizeServerErrors(payload) {
  if (!payload || typeof payload !== "object") return [];
  const body = payload;
  const errors = [];
  for (const [id, message] of Object.entries(body.fieldErrors ?? {})) {
    if (typeof message === "string" && message) errors.push({ id, message, label: FIELDS[id]?.label });
  }
  for (const entry of body.errors ?? []) {
    const id = entry.field ?? entry.id;
    if (id && entry.message) errors.push({ id, message: entry.message, label: FIELDS[id]?.label });
  }
  if (errors.length === 0 && typeof body.message === "string" && body.message) {
    errors.push({ id: "custom-rpc", message: body.message, label: FIELDS["custom-rpc"].label });
  }
  return errors;
}

test("WEB-23: client validation reports invalid fields with labels", () => {
  const errors = validateValues(
    { currency: "JPY", slippage: "9", "custom-rpc": "ftp://rpc.example.com" },
    {},
  );

  assert.deepEqual(
    errors.map((error) => error.id),
    ["currency", "slippage", "custom-rpc"],
  );
  assert.equal(errors[0].label, "Currency");
  assert.match(errors[0].message, /supported currency/);
  assert.match(errors[1].message, /between 0.1% and 5%/);
  assert.equal(errors[2].message, "Use an http:// or https:// URL.");
});

test("WEB-23: valid values pass validation", () => {
  const errors = validateValues(
    { currency: "USD", slippage: "1.5", "custom-rpc": "https://rpc.example.com" },
    {},
  );
  assert.deepEqual(errors, []);
});

test("WEB-23: server field errors normalize into summaries", () => {
  const errors = normalizeServerErrors({
    fieldErrors: { "custom-rpc": "Endpoint is not allowed." },
    errors: [{ id: "slippage", message: "Over account limit." }],
  });

  assert.deepEqual(errors, [
    { id: "custom-rpc", message: "Endpoint is not allowed.", label: "Custom RPC endpoint" },
    { id: "slippage", message: "Over account limit.", label: "Slippage tolerance" },
  ]);
});

test("WEB-23: server action adds a production RPC guard", () => {
  const source = read(path.join("app", "settings", "actions.ts"));
  assert.match(source, /"use server"/);
  assert.match(source, /validateSettingsValues\(values, settings\)/);
  assert.match(source, /NODE_ENV === "production"/);
  assert.match(source, /localhost.*127.*::1/);
  assert.match(source, /id: "custom-rpc"/);
});

test("WEB-23: summary links navigate to the field's tab and move focus", () => {
  const summary = read(path.join("components", "ErrorSummary.tsx"));
  assert.match(summary, /onNavigate\?:\s*\(id: string\) => void/);
  assert.match(summary, /onNavigate\?\.\(error\.id\)/);
  assert.match(summary, /requestAnimationFrame/);
  assert.match(summary, /target\.focus\(\)/);
  assert.match(summary, /tabIndex={-1}/);

  const settingsPage = read(path.join("components", "settings", "SettingsPage.tsx"));
  assert.match(settingsPage, /<ErrorSummary/);
  assert.match(settingsPage, /SETTINGS_FIELDS\[id\]\?\.tab/);
  assert.match(settingsPage, /setActiveTab\(tab\)/);
  assert.match(settingsPage, /validateSettingsValues\(validation\.values, settings\)/);
  assert.match(settingsPage, /validateSettingsUpdate\(settings, validation\.values\)/);
  assert.match(settingsPage, /validation\.setErrors\(fieldErrorsToMap\(result\.errors\)\)/);

  const handleSaveStart = settingsPage.indexOf("const handleSave = async () => {");
  const handleSaveEnd = settingsPage.indexOf("const ActiveTabComponent", handleSaveStart);
  const handleSave = settingsPage.slice(handleSaveStart, handleSaveEnd);
  assert.doesNotMatch(handleSave, /resetSettings/, "server rejection must preserve valid draft values");
});

test("WEB-23: fields expose aria-invalid and aria-describedby", () => {
  const context = read(path.join("components", "settings", "SettingsValidationContext.tsx"));
  assert.match(context, /"aria-invalid": Boolean\(errors\[id\]\)/);
  assert.match(context, /"aria-describedby": `\$\{id\}-error`/);

  for (const file of [
    "NetworkSettingsTab.tsx",
    "TradingSettingsTab.tsx",
    "DisplaySettingsTab.tsx",
    "SecuritySettingsTab.tsx",
  ]) {
    const source = read(path.join("components", "settings", file));
    assert.match(source, /fieldProps\(/, `${file} must apply shared validation props`);
    assert.match(source, /-error"/, `${file} must render inline error messages`);
  }

  const dynamicForm = read(path.join("components", "DynamicForm.tsx"));
  assert.match(dynamicForm, /<ErrorSummary/);
  assert.match(dynamicForm, /aria-describedby=\{fieldErrors\[input\.name\]/);
});
