// web/__tests__/web-16-aria-live.test.cjs — source-contract tests for the
// aria-live status regions added for WEB-16 (issue #470). Runs with
// `node --test ./__tests__/**/*.test.cjs`.
//
// web/node_modules is not installed in every environment, so — following the
// repo's convention of mirrored `.cjs` tests (see web/lib/apiCache.test.cjs) —
// these tests pin the *contract* the issue asks for by reading the component
// sources directly:
//
//   1. a reusable live-region module exists,
//   2. every dynamic status surface in the app announces its changes through
//      an aria-live region (polite for progress, assertive for failures),
//   3. the regions stay mounted so changes are actually announced,
//   4. every announced string is localized in messages/en.json.
//
// If the markup moves, update the regexes here together with the components.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('path');

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');

test('a shared LiveRegion module exports polite status and assertive alert regions', () => {
  const src = read('components/LiveRegion.tsx');

  assert.match(src, /export function LiveStatus/, 'polite status region exported');
  assert.match(src, /export function LiveAlert/, 'assertive alert region exported');

  // The status region must be a polite live region…
  assert.match(src, /aria-live="polite"/, 'status region announces politely');
  // …and the alert region must interrupt (role=alert implies assertive).
  assert.match(src, /role="alert"/, 'alert region interrupts');
  assert.match(src, /aria-live="assertive"/, 'alert region is explicitly assertive');

  // Both must be screen-reader-only so they never change the visual layout.
  assert.match(src, /className="sr-only"/, 'regions are visually hidden');
});

test('analysis page announces loading and result outcomes (index.tsx)', () => {
  const src = read('pages/index.tsx');

  assert.match(src, /import \{ LiveAlert, LiveStatus \} from "\.\.\/components\/LiveRegion"/);
  assert.match(src, /data-testid="analysis-live-status"/);
  assert.match(src, /data-testid="analysis-live-alert"/);
  // The message is derived from state, so a mounted region always reflects it.
  assert.match(src, /analysisStatusMessage = loading/, 'status message derives from loading state');
  assert.match(src, /a11y\.analysisRunning/, 'running state is announced');
  assert.match(src, /a11y\.resultReady/, 'completion is announced');
});

test('dynamic form announces the simulating state and assertive field errors', () => {
  const src = read('components/DynamicForm.tsx');

  assert.match(src, /data-testid="dynamic-form-live-status"/);
  assert.match(src, /role="status"/, 'form status region is a status');
  assert.match(src, /aria-live="polite"/, 'form status is announced politely');
  assert.match(src, /loading \? t\("dynamicForm\.simulating"\)/, 'simulating state is announced');

  // Inline field validation keeps role=alert; make the interruption explicit.
  assert.match(src, /role="alert"\s*\n\s*aria-live="assertive"/, 'field errors interrupt');
});

test('upload zone announces scanning, success and rejection states', () => {
  const src = read('components/upload-zone.tsx');

  assert.match(src, /data-testid="upload-live-status"/);
  assert.match(src, /displayState === "scanning" && t\("upload\.scanning"\)/);
  assert.match(src, /displayState === "success" && t\("upload\.success"\)/);
  assert.match(src, /displayState === "error" && t\("upload\.rejected"\)/);
});

test('WasmUpload announces aggregate upload progress', () => {
  const src = read('components/WasmUpload.tsx');

  assert.match(src, /data-testid="wasm-upload-live-status"/);
  assert.match(src, /uploadStatusMessage/, 'aggregate status message exists');
});

test('admin managers page announces loading and error transitions', () => {
  const src = read('pages/admin/managers.tsx');

  assert.match(src, /data-testid="admin-managers-live-status"/);
  assert.match(src, /loading \? t\("admin\.managers\.loading"\) : \(error \?\? ""\)/);
});

test('manager onboarding announces step transitions and errors', () => {
  const src = read('pages/managers/onboarding.tsx');

  assert.match(src, /data-testid="onboarding-live-status"/);
  assert.match(src, /stepStatusMessage/, 'step status message exists');
});

test('wallet modal announces connecting state and assertive connection errors', () => {
  const src = read('components/WalletModal.tsx');

  assert.match(src, /data-testid="wallet-modal-live-status"/);
  assert.match(src, /isConnecting \? t\("connecting"\) : ""/, 'connecting state is announced');

  assert.match(
    src,
    /role="alert"\s*\n\s*aria-live="assertive"/,
    'connection errors interrupt',
  );
});

test('every announced string is localized in messages/en.json', () => {
  const messages = JSON.parse(read('messages/en.json'));

  const required = [
    'a11y.statusRegionLabel',
    'a11y.analysisRunning',
    'a11y.analysisFailed',
    'a11y.resultReady',
  ];
  for (const key of required) {
    assert.ok(messages[key], `${key} exists in en.json`);
    assert.strictEqual(typeof messages[key], 'string');
  }

  // Pre-existing keys referenced by the new regions must not regress.
  for (const key of [
    'upload.scanning',
    'upload.success',
    'upload.rejected',
    'admin.managers.loading',
    'dynamicForm.simulating',
    'walletModal.connecting',
  ]) {
    assert.ok(messages[key], `${key} still exists in en.json`);
  }
});

test('pre-existing live regions still cover network and RPC banners', () => {
  // Guards against "fixing" the issue by deleting earlier a11y work.
  const network = read('components/NetworkStatusBanner.tsx');
  assert.match(network, /aria-live="assertive"/);
  assert.match(network, /aria-live="polite"/);

  const rpc = read('components/RpcFallbackBanner.tsx');
  assert.match(rpc, /aria-live="assertive"/);
});
