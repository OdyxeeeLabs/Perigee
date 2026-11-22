#!/usr/bin/env node
// serve.mjs — zero-dependency static preview of the generated component docs.
//
//   npm run docs:serve            # http://localhost:4173
//   npm run docs:serve -- --port 8080
//
// The docs site is plain HTML, so a static server (or `vercel`, `gh-pages`,
// `python -m http.server`, …) can host it anywhere.

import { createServer } from 'node:http';
import { existsSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { LIBRARY } from './registry.mjs';

const WEB_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const ROOT = path.join(WEB_ROOT, LIBRARY.siteDir);

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.md': 'text/markdown; charset=utf-8',
};

function parseArgs(argv) {
  const portIndex = argv.indexOf('--port');
  return {
    port: portIndex === -1 ? 4173 : Number.parseInt(argv[portIndex + 1], 10),
  };
}

function resolveFile(urlPath) {
  const clean = decodeURIComponent(urlPath.split('?')[0]);
  let target = path.join(ROOT, clean);
  if (!target.startsWith(ROOT)) return null;
  if (existsSync(target) && statSync(target).isDirectory()) target = path.join(target, 'index.html');
  return existsSync(target) ? target : null;
}

export function createDocsServer() {
  return createServer((request, response) => {
    if (!existsSync(ROOT)) {
      response.writeHead(500, { 'content-type': 'text/plain' });
      response.end('Docs site not built yet. Run `npm run docs:build` first.\n');
      return;
    }
    const file = resolveFile(request.url ?? '/');
    if (!file) {
      response.writeHead(404, { 'content-type': 'text/html; charset=utf-8' });
      response.end(
        '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Not found</title></head>' +
          '<body><h1>404 — not found</h1><p><a href="/index.html">All components</a></p></body></html>',
      );
      return;
    }
    response.writeHead(200, { 'content-type': MIME[path.extname(file)] ?? 'application/octet-stream' });
    response.end(readFileSync(file));
  });
}

const isDirectRun = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isDirectRun) {
  const { port } = parseArgs(process.argv.slice(2));
  createDocsServer().listen(port, () => {
    process.stdout.write(`Component docs: http://localhost:${port}/\n`);
  });
}
