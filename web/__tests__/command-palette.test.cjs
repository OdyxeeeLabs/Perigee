/**
 * Unit tests for the command palette registry (WEB-52 / #185).
 *
 * The registry helpers in `lib/commandRegistry.ts` are pure data + pure
 * functions, so they can be mirrored inline here (CJS/ESM boundary — same
 * pattern as `DynamicForm.test.cjs`).
 *
 * Run with:
 *   node --test ./__tests__/command-palette.test.cjs
 */

"use strict";

const assert = require("node:assert/strict");
const { test, describe } = require("node:test");

// ---------------------------------------------------------------------------
// Inline registry mirror (must stay in sync with lib/commandRegistry.ts)
// ---------------------------------------------------------------------------

const NAVIGATION_COMMANDS = [
  {
    kind: "navigation",
    id: "nav-home",
    route: "/",
    labelKey: "palette.goHome",
    keywords: ["home", "analyzer", "contract", "dashboard", "start"],
  },
  {
    kind: "navigation",
    id: "nav-vaults",
    route: "/vaults",
    labelKey: "palette.goVaults",
    keywords: ["vaults", "portfolio", "funds", "positions", "list"],
  },
];

const SETTINGS_COMMANDS = [
  {
    kind: "settings",
    id: "settings-admin-managers",
    route: "/admin/managers",
    labelKey: "palette.goAdminManagers",
    keywords: [
      "admin",
      "managers",
      "kyc",
      "approve",
      "whitelist",
      "settings",
      "administration",
    ],
  },
  {
    kind: "settings",
    id: "settings-manager-onboarding",
    route: "/managers/onboarding",
    labelKey: "palette.goManagerOnboarding",
    keywords: [
      "onboarding",
      "manager",
      "register",
      "kyc",
      "signup",
      "sign up",
      "settings",
    ],
  },
];

const ACTION_COMMANDS = [
  {
    kind: "action",
    id: "action-copy-wallet-address",
    actionId: "copy-wallet-address",
    labelKey: "palette.copyWalletAddress",
    keywords: ["copy", "wallet", "address", "clipboard", "account", "key"],
  },
];

const COMMAND_GROUP_ORDER = ["navigation", "vaults", "settings", "actions"];

function buildVaultCommands(vaults) {
  return vaults
    .filter((v) => v && typeof v.id === "string" && v.id.trim().length > 0)
    .map((v) => ({
      kind: "vault",
      id: `vault-${v.id}`,
      route: `/vault/${encodeURIComponent(v.id)}`,
      label: typeof v.name === "string" && v.name.trim() ? v.name : `Vault ${v.id}`,
      vaultId: v.id,
      keywords: ["vault", v.id, typeof v.name === "string" ? v.name : ""].filter(
        Boolean
      ),
    }));
}

function commandMatches(entry, query, resolveLabel) {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;

  const haystack = [
    resolveLabel(entry),
    entry.keywords.join(" "),
    entry.kind === "vault" ? entry.vaultId : "",
  ]
    .join(" ")
    .toLowerCase();

  return words.every((w) => haystack.includes(w));
}

function filterCommands(entries, query, resolveLabel) {
  const q = query.trim().toLowerCase();
  if (!q) return entries;

  return entries
    .map((entry) => {
      const label = resolveLabel(entry).toLowerCase();
      let score = 0;
      if (label.startsWith(q)) score = 2;
      else if (label.includes(q)) score = 1;
      else if (!commandMatches(entry, q, resolveLabel)) score = -1;
      return { entry, score };
    })
    .filter((r) => r.score >= 0)
    .sort((a, b) => b.score - a.score)
    .map((r) => r.entry);
}

const KIND_TO_GROUP = {
  navigation: "navigation",
  vault: "vaults",
  settings: "settings",
  action: "actions",
};

function groupCommands(entries) {
  const buckets = new Map();
  for (const entry of entries) {
    const group = KIND_TO_GROUP[entry.kind];
    const list = buckets.get(group) ?? [];
    list.push(entry);
    buckets.set(group, list);
  }
  return COMMAND_GROUP_ORDER.filter((g) => buckets.has(g)).map((group) => ({
    group,
    entries: buckets.get(group),
  }));
}

function groupHeadingKey(group) {
  return `palette.group${group.charAt(0).toUpperCase()}${group.slice(1)}`;
}

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

const LABELS = {
  "palette.goHome": "Go to Home (Analyzer)",
  "palette.goVaults": "Go to Vaults dashboard",
  "palette.goAdminManagers": "Go to Manager administration",
  "palette.goManagerOnboarding": "Go to Manager onboarding",
  "palette.copyWalletAddress": "Copy wallet address",
  "palette.openVault": "Open vault",
};

function resolveLabel(entry) {
  if (entry.kind === "vault") return entry.label;
  return LABELS[entry.labelKey] ?? entry.labelKey;
}

const ALL_STATIC = [...NAVIGATION_COMMANDS, ...SETTINGS_COMMANDS, ...ACTION_COMMANDS];

// ---------------------------------------------------------------------------
// Static registry
// ---------------------------------------------------------------------------

describe("command registry · static entries", () => {
  test("covers navigation, vaults dashboard, admin and onboarding routes", () => {
    const routes = [...NAVIGATION_COMMANDS, ...SETTINGS_COMMANDS].map(
      (c) => c.route
    );
    assert.ok(routes.includes("/"), "home route missing");
    assert.ok(routes.includes("/vaults"), "vaults route missing");
    assert.ok(
      SETTINGS_COMMANDS.some((c) => c.route === "/admin/managers"),
      "admin managers route missing"
    );
    assert.ok(
      SETTINGS_COMMANDS.some((c) => c.route === "/managers/onboarding"),
      "manager onboarding route missing"
    );
  });

  test("every static entry carries a unique id and non-empty keywords", () => {
    const ids = new Set();
    for (const entry of ALL_STATIC) {
      assert.ok(entry.id, "missing id");
      assert.ok(!ids.has(entry.id), `duplicate id: ${entry.id}`);
      ids.add(entry.id);
      assert.ok(
        Array.isArray(entry.keywords) && entry.keywords.length > 0,
        `${entry.id} has no keywords`
      );
    }
  });

  test("every labelKey resolves in the en message bundle contract", () => {
    // Guard against renaming a message key without updating en.json.
    const fs = require("node:fs");
    const path = require("node:path");
    const messages = JSON.parse(
      fs.readFileSync(
        path.join(__dirname, "..", "messages", "en.json"),
        "utf8"
      )
    );
    for (const entry of ALL_STATIC) {
      assert.ok(
        typeof messages[entry.labelKey] === "string" &&
          messages[entry.labelKey].length > 0,
        `missing message for ${entry.labelKey}`
      );
    }
    for (const key of [
      "palette.title",
      "palette.openLabel",
      "palette.triggerLabel",
      "palette.empty",
      "palette.groupNavigation",
      "palette.groupVaults",
      "palette.groupSettings",
      "palette.groupActions",
      "palette.searchVaultsHint",
      "palette.walletNotConnected",
      "palette.close",
    ]) {
      assert.ok(typeof messages[key] === "string", `missing message ${key}`);
    }
  });
});

// ---------------------------------------------------------------------------
// Dynamic vault commands
// ---------------------------------------------------------------------------

describe("command registry · vault commands", () => {
  test("builds a route per vault and keeps names searchable", () => {
    const cmds = buildVaultCommands([
      { id: "abc123", name: "Bull Basket" },
      { id: "def456" },
    ]);

    assert.equal(cmds.length, 2);
    assert.equal(cmds[0].route, "/vault/abc123");
    assert.equal(cmds[0].label, "Bull Basket");
    assert.deepEqual(cmds[0].keywords, ["vault", "abc123", "Bull Basket"]);
    assert.equal(cmds[1].label, "Vault def456");
  });

  test("URL-encodes special characters in vault ids", () => {
    const cmds = buildVaultCommands([{ id: "id/with spaces" }]);
    assert.equal(cmds[0].route, "/vault/id%2Fwith%20spaces");
  });

  test("drops malformed vault records", () => {
    const cmds = buildVaultCommands([
      null,
      {},
      { id: "   " },
      { id: "valid" },
    ]);
    assert.equal(cmds.length, 1);
    assert.equal(cmds[0].vaultId, "valid");
  });

  test("empty registry yields no vault group", () => {
    const grouped = groupCommands([...buildVaultCommands([]), ...ALL_STATIC]);
    assert.ok(!grouped.some((g) => g.group === "vaults"));
  });
});

// ---------------------------------------------------------------------------
// Filtering / search
// ---------------------------------------------------------------------------

describe("command palette · filtering", () => {
  test("empty query returns all entries", () => {
    assert.equal(filterCommands(ALL_STATIC, "", resolveLabel).length, ALL_STATIC.length);
    assert.equal(filterCommands(ALL_STATIC, "   ", resolveLabel).length, ALL_STATIC.length);
  });

  test("matches labels case-insensitively", () => {
    const out = filterCommands(ALL_STATIC, "copy wallet", resolveLabel);
    assert.equal(out.length, 1);
    assert.equal(out[0].id, "action-copy-wallet-address");
  });

  test("matches keywords, not just labels (settings via 'kyc')", () => {
    const ids = filterCommands(ALL_STATIC, "kyc", resolveLabel).map((c) => c.id);
    assert.ok(ids.includes("settings-admin-managers"));
    assert.ok(ids.includes("settings-manager-onboarding"));
  });

  test("vault search matches id and name", () => {
    const vaults = buildVaultCommands([{ id: "vault99", name: "Bear LP" }]);
    assert.equal(filterCommands(vaults, "bear", resolveLabel).length, 1);
    assert.equal(filterCommands(vaults, "vault99", resolveLabel).length, 1);
    assert.equal(filterCommands(vaults, "nope", resolveLabel).length, 0);
  });

  test("multi-word queries require all words (AND semantics)", () => {
    const out = filterCommands(ALL_STATIC, "manager admin", resolveLabel);
    assert.ok(out.some((c) => c.id === "settings-admin-managers"));
    assert.ok(!out.some((c) => c.id === "action-copy-wallet-address"));
  });

  test("prefix matches rank above substring matches", () => {
    // "go t" — "Go to ..." labels all start with it; ensure prefix items
    // come before any keyword-only match.
    const out = filterCommands(
      [...ALL_STATIC, ...buildVaultCommands([{ id: "t1", name: "Get Tea" }])],
      "go t",
      resolveLabel
    );
    const firstNonPrefixIdx = out.findIndex(
      (c) => !resolveLabel(c).toLowerCase().startsWith("go t")
    );
    const lastPrefixIdx = out.reduce(
      (acc, c, i) => (resolveLabel(c).toLowerCase().startsWith("go t") ? i : acc),
      -1
    );
    assert.ok(
      lastPrefixIdx < firstNonPrefixIdx || firstNonPrefixIdx === -1,
      "prefix matches should be ranked before substring matches"
    );
  });

  test("no results for a nonsense query", () => {
    assert.equal(filterCommands(ALL_STATIC, "zzzznotfound", resolveLabel).length, 0);
  });

  test("commandMatches: empty query matches everything", () => {
    assert.ok(commandMatches(ALL_STATIC[0], "", resolveLabel));
  });
});

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

describe("command palette · grouping", () => {
  test("groups follow the fixed display order", () => {
    const grouped = groupCommands([
      ACTION_COMMANDS[0],
      buildVaultCommands([{ id: "v1" }])[0],
      NAVIGATION_COMMANDS[0],
    ]);
    assert.deepEqual(
      grouped.map((g) => g.group),
      ["navigation", "vaults", "actions"]
    );
  });

  test("empty groups are dropped", () => {
    const grouped = groupCommands([NAVIGATION_COMMANDS[0]]);
    assert.deepEqual(
      grouped.map((g) => g.group),
      ["navigation"]
    );
  });

  test("vault and action groups render when populated", () => {
    const grouped = groupCommands([
      ACTION_COMMANDS[0],
      buildVaultCommands([{ id: "v1" }])[0],
    ]);
    assert.deepEqual(
      grouped.map((g) => g.group),
      ["vaults", "actions"]
    );
  });

  test("group headings map to message keys", () => {
    assert.equal(groupHeadingKey("navigation"), "palette.groupNavigation");
    assert.equal(groupHeadingKey("vaults"), "palette.groupVaults");
    assert.equal(groupHeadingKey("settings"), "palette.groupSettings");
    assert.equal(groupHeadingKey("actions"), "palette.groupActions");
  });
});
