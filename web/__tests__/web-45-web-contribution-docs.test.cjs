// web/__tests__/web-45-web-contribution-docs.test.cjs — contract test for
// WEB-45 (issue #846): the development guide must describe the real web setup
// and the commands a contributor is expected to run.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('path');

const docs = fs.readFileSync(path.join(__dirname, '..', '..', 'docs', 'development.md'), 'utf8');
const rootPackage = JSON.parse(
  fs.readFileSync(path.join(__dirname, '..', '..', 'package.json'), 'utf8'),
);
const webPackage = JSON.parse(
  fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'),
);

test('the guide pins the package manager declared by the repository', () => {
  assert.ok(rootPackage.packageManager.startsWith('yarn@'), 'root pins Yarn');
  assert.ok(docs.includes('yarn@4.9.1'), 'guide names the pinned Yarn version');
  assert.ok(docs.includes('corepack enable'), 'guide explains Corepack setup');
  assert.ok(docs.includes('yarn install --immutable'), 'guide gives the immutable install command');
  assert.ok(!docs.includes('npm install'), 'stale npm install instructions were removed');
});

test('the guide documents the Node version used by web CI', () => {
  assert.ok(docs.includes('Node.js 24'), 'guide states the CI Node version');
});

test('the guide lists the essential web commands from web/package.json', () => {
  for (const script of ['dev', 'lint', 'typecheck', 'test', 'build', 'docs:check']) {
    assert.ok(webPackage.scripts[script], `web/package.json defines ${script}`);
    assert.ok(docs.includes(script), `guide mentions ${script}`);
  }
  assert.ok(docs.includes('node --test'), 'guide explains the node test runner');
  assert.ok(docs.includes('source-contract tests'), 'guide explains the .cjs test convention');
});

test('the guide explains the fork-based contribution workflow', () => {
  assert.ok(docs.includes('git fetch upstream main'), 'guide documents syncing upstream');
  assert.ok(docs.includes('never push to `OdyxeeeLabs/Perigee:main`'), 'guide forbids pushing main');
  assert.ok(docs.includes('Closes #N'), 'guide documents issue links');
});
