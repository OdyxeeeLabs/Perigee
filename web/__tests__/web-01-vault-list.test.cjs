"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, "..", relative), "utf8");

test("App Router vault page uses the shared API for the connected manager", () => {
  const page = read("app/vaults/page.tsx");
  const api = read("lib/api.ts");
  assert.match(page, /managerService\.checkStatus\(address\)/);
  assert.match(page, /vaultService\.list\(manager\.id\)/);
  assert.match(page, /records\.map\(vaultFromRecord\)/);
  assert.doesNotMatch(page, /records\.filter\(/, "tenant access is not client-side filtering");
  assert.match(api, /async list\(managerId: string, token\?: string\)[\s\S]*?cache: "no-store"/);
  assert.match(page, /action: "connect"/);
  assert.match(page, /action: "retry"/);
});

test("vault list component renders loading, empty, error, and populated states", () => {
  const page = read("app/vaults/page.tsx");
  for (const status of ["loading", "empty", "error", "populated"]) {
    assert.ok(
      page.includes(`state.status === "${status}"`),
      `missing ${status} state`,
    );
  }
  assert.match(page, /<VaultListSkeleton/);
  assert.match(page, /No vaults found for this account\./);
  assert.match(page, /role="alert"/);
  assert.match(page, /<VaultList/);
});

test("vault rows link to detail pages and adapt their layout on narrow screens", () => {
  const list = read("components/vault/VaultList.tsx");
  const row = read("components/VaultRow.tsx");
  const skeleton = read("components/vault/VaultCardSkeleton.tsx");
  assert.match(list, /href=\{`\/vaults\/\$\{vault\.id\}`\}/);
  assert.match(row, /flex-col items-stretch[\s\S]*sm:flex-row/);
  assert.match(row, /grid grid-cols-3[\s\S]*sm:flex/);
  assert.match(row, /sm:hidden">Balance/);
  assert.match(skeleton, /flex-col items-stretch[\s\S]*sm:flex-row/);
});
