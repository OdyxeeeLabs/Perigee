#!/usr/bin/env node
/**
 * Verifies response compression (issue #695).
 *
 *   node scripts/check-compression.mjs [baseUrl] [path ...]
 *
 * For every path it requests the resource with `Accept-Encoding: br, gzip`,
 * checks the `Content-Encoding` header (compressible types must be br or gzip;
 * images, video and other already-compressed types must not be re-compressed)
 * and prints the compression ratio against an identity request.
 * Exits non-zero if any check fails.
 */

const base = (process.argv[2] || "http://localhost:3000").replace(/\/$/, "");
const paths = process.argv.slice(3);
if (paths.length === 0) paths.push("/", "/favicon.ico", "/api/vitals");

const COMPRESSIBLE = /^(text\/|application\/(json|javascript|xml)|image\/svg\+xml|font\/(ttf|otf)|application\/vnd\.ms-fontobject)/;
const PRE_COMPRESSED = /^(image\/(png|jpe?g|webp|avif|gif)|video\/|audio\/|font\/woff2?|application\/(zip|gzip|wasm))/;

let failed = false;

for (const path of paths) {
  const url = `${base}${path}`;
  try {
    const compressed = await fetch(url, { headers: { "Accept-Encoding": "br, gzip" } });
    const identity = await fetch(url, { headers: { "Accept-Encoding": "identity" } });
    const type = (compressed.headers.get("content-type") || "").split(";")[0].trim();
    const encoding = compressed.headers.get("content-encoding") || "identity";

    const rawSize = (await identity.arrayBuffer()).byteLength;
    const sentSize = Number(compressed.headers.get("content-length")) || rawSize;
    const ratio = rawSize > 0 ? (sentSize / rawSize).toFixed(2) : "n/a";

    let ok = true;
    let note = "";
    if (COMPRESSIBLE.test(type) && rawSize > 1024 && !["br", "gzip"].includes(encoding)) {
      ok = false;
      note = "expected br or gzip";
    }
    if (PRE_COMPRESSED.test(type) && encoding !== "identity") {
      ok = false;
      note = "already-compressed type must not be re-compressed";
    }

    console.log(`${ok ? "ok  " : "FAIL"} ${path} [${type || "?"}] encoding=${encoding} raw=${rawSize}B ratio=${ratio} ${note}`);
    if (!ok) failed = true;
  } catch (error) {
    console.log(`FAIL ${path} request error: ${error.message}`);
    failed = true;
  }
}

process.exit(failed ? 1 : 0);
