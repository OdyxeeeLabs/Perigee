// web/__tests__/web-34-network-status-routes.test.cjs — tests for issue #835
// (WEB-34). Transaction-capable views must show network status and only
// disable actions that actually require a reachable network.
// Runs with: node --test ./__tests__/**/*.test.cjs

"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, "..", relative), "utf8");

function connectionState({ online, reachable }) {
  if (!online) return "offline";
  if (reachable === false) return "degraded";
  return "online";
}

function resolveTransactionGate({ online, reachable, lastHealthyAt = null }) {
  const status = connectionState({ online, reachable });
  const canTransact = status === "online";
  return {
    status,
    canTransact,
    disabledReason:
      status === "offline"
        ? "Transactions are disabled while you are offline. Reconnect to submit."
        : status === "degraded"
          ? "Transactions are disabled because the chain data is unreachable. Reconnect to submit."
          : null,
    showingCachedData: !canTransact && Boolean(lastHealthyAt),
  };
}

test("online: transactions allowed, no explanation, no cached-data marker", () => {
  const gate = resolveTransactionGate({ online: true, reachable: true });
  assert.equal(gate.status, "online");
  assert.equal(gate.canTransact, true);
  assert.equal(gate.disabledReason, null);
  assert.equal(gate.showingCachedData, false);
});

test("degraded: transactions blocked with an explanation, cached data marked read-only", () => {
  const gate = resolveTransactionGate({ online: true, reachable: false, lastHealthyAt: Date.now() });
  assert.equal(gate.status, "degraded");
  assert.equal(gate.canTransact, false);
  assert.match(gate.disabledReason, /chain data is unreachable/);
  assert.equal(gate.showingCachedData, true, "cached data is distinguishable from live data");

  const neverHealthy = resolveTransactionGate({ online: true, reachable: false, lastHealthyAt: null });
  assert.equal(neverHealthy.showingCachedData, false);
});

test("offline: transactions blocked regardless of cached data", () => {
  const gate = resolveTransactionGate({ online: false, reachable: null, lastHealthyAt: Date.now() });
  assert.equal(gate.status, "offline");
  assert.equal(gate.canTransact, false);
  assert.match(gate.disabledReason, /offline/);
});

test("hook derives status from navigator.onLine plus shared RPC health", () => {
  const hook = read("hooks/useTransactionNetwork.ts");
  assert.match(hook, /useRpcStatus\(API_URL\)/);
  assert.match(hook, /window\.navigator\.onLine/);
  assert.match(hook, /resolveTransactionGate\(/);
  assert.match(hook, /addEventListener\("online", update\)/);
  assert.match(hook, /addEventListener\("offline", update\)/);
});

test("transaction notice explains the block, marks cached data, and offers retry", () => {
  const src = read("components/TransactionNetworkNotice.tsx");
  assert.match(src, /role="status"/);
  assert.match(src, /network\.disabledReason/);
  assert.match(src, /Cached data — read only/);
  assert.match(src, /data-testid="cached-data-badge"/);
  assert.match(src, /onClick=\{network\.retry\}/);
});

test("vault transaction actions are disabled only when the network gate is closed", () => {
  const content = read("components/vault/VaultDetailContent.tsx");
  assert.match(content, /transactionsEnabled\?: boolean/);
  assert.equal((content.match(/disabled=\{!transactionsEnabled\}/g) || []).length, 3);
  assert.match(content, /vault-actions-disabled-reason/);
  for (const file of [
    "components/vault/VaultDetailPageClient.tsx",
    "components/vault/VaultDetailModal.tsx",
  ]) {
    const src = read(file);
    assert.match(src, /useTransactionNetwork\(\)/, `${file} reads network status`);
    assert.match(src, /transactionsEnabled=\{network\.canTransact\}/, `${file} gates actions`);
    assert.match(src, /<TransactionNetworkNotice network=\{network\} \/>/, `${file} shows status`);
  }
});
