#!/usr/bin/env node
/**
 * Route JavaScript budget guard (issue #842 / WEB-41).
 *
 *   node scripts/check-route-budgets.mjs [buildDir] [--budget-file path]
 *
 * Reads the route -> chunk mapping from Next's build output
 * (`.next/build-manifest.json` and `.next/app-build-manifest.json`) and sums
 * the on-disk size of every JavaScript chunk in each budgeted route. Sizes are
 * deterministic bytes, not load timings, so a breach is reproducible.
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs(argv) {
  let buildDir = ".next";
  let budgetFile = path.join(webRoot, "route-budgets.json");
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--budget-file") {
      budgetFile = argv[++i];
      if (!budgetFile) throw new Error("--budget-file needs a path");
    } else if (!arg.startsWith("--")) {
      buildDir = arg;
    }
  }
  return { buildDir, budgetFile };
}

const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const formatBytes = (bytes) => bytes.toLocaleString("en-US");

function routeFiles(manifests, route) {
  const pages = manifests.pages?.pages?.[route.manifestKey]?.files;
  const app = route.appManifestKey
    ? manifests.app?.pages?.[route.appManifestKey]?.files
    : undefined;
  const files = app ?? pages;
  return files ? files.filter((file) => file.endsWith(".js")) : null;
}

export function checkRouteBudgets(buildDir, budgetFile) {
  const budgets = readJson(budgetFile);
  if (!Array.isArray(budgets.routes)) throw new Error("budget file has no routes array");

  const buildPath = path.resolve(buildDir);
  const manifests = {
    pages: fs.existsSync(path.join(buildPath, "build-manifest.json"))
      ? readJson(path.join(buildPath, "build-manifest.json"))
      : null,
    app: fs.existsSync(path.join(buildPath, "app-build-manifest.json"))
      ? readJson(path.join(buildPath, "app-build-manifest.json"))
      : null,
  };
  if (!manifests.pages && !manifests.app) {
    throw new Error(`no Next build manifests found in ${buildPath}; run the build first`);
  }

  const results = [];
  let failed = false;

  for (const route of budgets.routes) {
    const files = routeFiles(manifests, route);
    if (!files || files.length === 0) {
      failed = true;
      results.push({
        name: route.name,
        key: route.appManifestKey ?? route.manifestKey,
        bytes: null,
        maxBytes: route.maxBytes,
        chunks: [],
        error: "route not found in build output",
      });
      continue;
    }

    const chunks = files
      .map((file) => {
        const fullPath = path.join(buildPath, file);
        return { file, bytes: fs.existsSync(fullPath) ? fs.statSync(fullPath).size : 0 };
      })
      .sort((a, b) => b.bytes - a.bytes);
    const bytes = chunks.reduce((total, chunk) => total + chunk.bytes, 0);
    if (bytes > route.maxBytes) failed = true;
    results.push({
      name: route.name,
      key: route.appManifestKey ?? route.manifestKey,
      bytes,
      maxBytes: route.maxBytes,
      chunks,
      error: null,
    });
  }

  return { results, failed };
}

function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`route budgets: ${error.message}`);
    process.exit(1);
  }

  let report;
  try {
    report = checkRouteBudgets(options.buildDir, options.budgetFile);
  } catch (error) {
    console.error(`route budgets: ${error.message}`);
    process.exit(1);
  }

  console.log("Route JavaScript budgets (raw bytes from Next build output)");
  for (const route of report.results) {
    if (route.error) {
      console.log(`FAIL ${route.name} (${route.key}): ${route.error}`);
      continue;
    }
    const over = route.bytes - route.maxBytes;
    const status = over > 0 ? "FAIL" : "PASS";
    console.log(
      `${status} ${route.name} (${route.key}): ${formatBytes(route.bytes)} / ${formatBytes(route.maxBytes)} bytes` +
        (over > 0 ? ` — ${formatBytes(over)} over budget` : ""),
    );
    if (over > 0) {
      for (const chunk of route.chunks) {
        console.log(`       ${chunk.file}: ${formatBytes(chunk.bytes)} bytes`);
      }
    }
  }

  if (report.failed) {
    console.error("Route JavaScript budget exceeded. Reduce the route chunk or update web/route-budgets.json deliberately.");
    process.exit(1);
  }
  console.log("All route JavaScript bundles are within budget.");
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
