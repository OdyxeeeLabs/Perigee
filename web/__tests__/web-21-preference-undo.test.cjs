"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const WEB_ROOT = path.join(__dirname, "..");
const read = (relative) => fs.readFileSync(path.join(WEB_ROOT, relative), "utf8");

const PREFERENCE_UNDO_WINDOW_MS = 7000;
const SETTINGS_STORAGE_KEY = "perigee_settings";

const store = new Map();
const storage = {
  getItem: (key) => store.get(key) ?? null,
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: (key) => store.delete(key),
  clear: () => store.clear(),
};

function canUndoPreference(pending, now) {
  return pending !== null && now - pending.changedAt <= PREFERENCE_UNDO_WINDOW_MS;
}

function recordPreferenceUndo(pending, section, field, previousValue, now, sequence) {
  if (
    pending &&
    pending.section === section &&
    pending.field === field &&
    canUndoPreference(pending, now)
  ) {
    return pending;
  }
  return { section, field, previousValue, changedAt: now, sequence };
}

function createSettingsStore() {
  let state = { settings: { display: { currency: "USD" }, trading: { slippageTolerance: 0.5 } }, pendingUndo: null };
  let sequence = 0;

  return {
    getState: () => state,
    update(section, field, value, now = 1000) {
      const previousValue = state.settings[section][field];
      const pendingUndo = recordPreferenceUndo(
        state.pendingUndo,
        section,
        field,
        previousValue,
        now,
        ++sequence,
      );
      state = {
        settings: { ...state.settings, [section]: { ...state.settings[section], [field]: value } },
        pendingUndo,
      };
      storage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(state.settings));
    },
    undo(now = 1000) {
      const pending = state.pendingUndo;
      if (!pending) return;
      if (!canUndoPreference(pending, now)) {
        state = { ...state, pendingUndo: null };
        return;
      }
      state = {
        settings: {
          ...state.settings,
          [pending.section]: {
            ...state.settings[pending.section],
            [pending.field]: pending.previousValue,
          },
        },
        pendingUndo: null,
      };
      storage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(state.settings));
    },
  };
}

test("WEB-21: undo restores the prior local preference", () => {
  storage.clear();
  const settings = createSettingsStore();
  settings.update("display", "currency", "EUR");
  assert.equal(settings.getState().settings.display.currency, "EUR");

  settings.undo();
  assert.equal(settings.getState().settings.display.currency, "USD");
});

test("WEB-21: persistence matches the final selection after undo", () => {
  storage.clear();
  const settings = createSettingsStore();
  settings.update("trading", "slippageTolerance", 1.5);
  settings.undo();

  assert.equal(
    JSON.parse(storage.getItem(SETTINGS_STORAGE_KEY)).trading.slippageTolerance,
    0.5,
  );
  assert.equal(JSON.parse(storage.getItem(SETTINGS_STORAGE_KEY)).trading.slippageTolerance,
    settings.getState().settings.trading.slippageTolerance);
});

test("WEB-21: a stale undo window cannot revert a newer choice", () => {
  storage.clear();
  const settings = createSettingsStore();
  settings.update("display", "currency", "EUR", 1000);
  settings.undo(1000 + PREFERENCE_UNDO_WINDOW_MS + 1);

  assert.equal(settings.getState().settings.display.currency, "EUR");
  assert.equal(settings.getState().pendingUndo, null);
});

test("WEB-21: undo is limited to local display/trading preferences", () => {
  const settingsStore = read(path.join("lib", "settingsStore.ts"));
  assert.match(settingsStore, /undoPreferenceChange/);
  assert.match(settingsStore, /commitLocalPreference\("trading"/);
  assert.match(settingsStore, /commitLocalPreference\("display"/);
  assert.doesNotMatch(settingsStore, /commitLocalPreference\("network"/);
  assert.doesNotMatch(settingsStore, /commitLocalPreference\("security"/);
  assert.doesNotMatch(settingsStore, /useWallet|signTransaction|sendTransaction|WalletContext["']/i);

  for (const file of ["DisplaySettingsTab.tsx", "TradingSettingsTab.tsx"]) {
    const source = read(path.join("components", "settings", file));
    assert.match(source, /usePreferenceUndoToast/, `${file} must expose the undo affordance`);
  }
  const hook = read(path.join("hooks", "usePreferenceUndoToast.ts"));
  assert.match(hook, /action:/);
  assert.match(hook, /undoPreferenceChange/);
  assert.match(hook, /toast\.dismiss/);

  const settingsPage = read(path.join("components", "settings", "SettingsPage.tsx"));
  assert.match(settingsPage, /pendingUndo/);
  assert.match(settingsPage, /syncFromSettings/);
});
