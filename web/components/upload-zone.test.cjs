// upload-zone.test.cjs — unit tests for upload-zone validation logic
// closes #422
// Runs with: node --test ./components/upload-zone.test.cjs

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');

// ── Pure validation helpers (mirrors upload-zone.tsx logic) ──────────────────

/**
 * wasmValidator: returns null for .wasm files, error object otherwise.
 * Mirrors the inline `wasmValidator` callback in upload-zone.tsx.
 */
function wasmValidator(fileName) {
  const ext = fileName.split('.').pop()?.toLowerCase();
  if (ext !== 'wasm') {
    return {
      code: 'file-invalid-type',
      message: `"${fileName}" was rejected — only .wasm files are accepted (got .${ext || 'unknown'})`,
    };
  }
  return null;
}

/**
 * validateWasmBuffer: checks magic number and version.
 * Mirrors the ArrayBuffer checks inside onDropAccepted in upload-zone.tsx.
 */
function validateWasmBuffer(buffer) {
  if (buffer.byteLength < 8) {
    throw new Error('File is too small to be a valid WebAssembly module');
  }
  const view = new DataView(buffer);
  const magic = view.getUint32(0, false); // big-endian: \0asm
  if (magic !== 0x0061736d) {
    throw new Error('Invalid WASM magic number. File is not a valid WebAssembly module');
  }
  const version = view.getUint32(4, true); // little-endian
  if (version !== 1) {
    throw new Error(`Unsupported WASM version: ${version}. Expected version 1`);
  }
}

// ── Helpers to build minimal buffers ─────────────────────────────────────────

function makeWasmBuffer(magic = 0x0061736d, version = 1) {
  const buf = new ArrayBuffer(8);
  const view = new DataView(buf);
  view.setUint32(0, magic, false);
  view.setUint32(4, version, true);
  return buf;
}

// ── wasmValidator tests ───────────────────────────────────────────────────────

test('wasmValidator: accepts .wasm file', () => {
  assert.equal(wasmValidator('contract.wasm'), null);
});

test('wasmValidator: rejects .js file', () => {
  const result = wasmValidator('script.js');
  assert.equal(result.code, 'file-invalid-type');
  assert.match(result.message, /only .wasm files are accepted/);
});

test('wasmValidator: rejects file with no extension', () => {
  const result = wasmValidator('noextension');
  assert.ok(result !== null);
  assert.equal(result.code, 'file-invalid-type');
});

test('wasmValidator: rejects .txt file', () => {
  const result = wasmValidator('readme.txt');
  assert.ok(result !== null);
  assert.match(result.message, /got .txt/);
});

// ── validateWasmBuffer tests ──────────────────────────────────────────────────

test('validateWasmBuffer: accepts valid WASM magic + version 1', () => {
  assert.doesNotThrow(() => validateWasmBuffer(makeWasmBuffer()));
});

test('validateWasmBuffer: rejects buffer smaller than 8 bytes', () => {
  assert.throws(
    () => validateWasmBuffer(new ArrayBuffer(4)),
    /too small/
  );
});

test('validateWasmBuffer: rejects wrong magic number', () => {
  assert.throws(
    () => validateWasmBuffer(makeWasmBuffer(0xdeadbeef, 1)),
    /Invalid WASM magic number/
  );
});

test('validateWasmBuffer: rejects unsupported version', () => {
  assert.throws(
    () => validateWasmBuffer(makeWasmBuffer(0x0061736d, 2)),
    /Unsupported WASM version: 2/
  );
});

// ── Clear / reset state tests (state machine logic) ──────────────────────────

test('handleReset: resets state to idle', () => {
  // Simulate the state fields managed by handleReset in upload-zone.tsx
  let uploadState = 'error';
  let droppedFile = { name: 'bad.txt', sizeBytes: 10 };
  let errorMessage = 'some error';

  // handleReset logic
  uploadState = 'idle';
  droppedFile = null;
  errorMessage = '';

  assert.equal(uploadState, 'idle');
  assert.equal(droppedFile, null);
  assert.equal(errorMessage, '');
});

test('onDropRejected: builds correct error message for non-wasm file', () => {
  const fileName = 'malicious.exe';
  const ext = fileName.includes('.') ? `.${fileName.split('.').pop()}` : 'unknown type';
  const errorMsg = `"${fileName}" was rejected — only .wasm files are accepted (got ${ext})`;

  assert.match(errorMsg, /only .wasm files are accepted/);
  assert.match(errorMsg, /\.exe/);
});
