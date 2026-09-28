// web/lib/utils.test.cjs — unit tests for the shared UI helpers in web/lib/utils.ts
// (Issue #475 / WEB-21). Runs with `node --test ./lib/**/*.test.cjs`.
//
// `package.json`'s `test` script only executes `.cjs` files, so — following the
// convention already used by web/lib/api-middleware.test.cjs and
// web/lib/gasGolfingSort.test.cjs — the pure logic is mirrored here in plain JS.
// `cn` is mirrored verbatim on top of the real `clsx` / `tailwind-merge`
// runtime dependencies, and `arrayBufferToBase64` is cross-checked against
// Node's `Buffer` base64 encoder as an independent oracle.
//
// If you change behaviour in web/lib/utils.ts, mirror the change here.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const { clsx } = require('clsx');
const { twMerge } = require('tailwind-merge');

/* ── Mirror of web/lib/utils.ts ──────────────────────────────────────── */

function cn(...inputs) {
  return twMerge(clsx(inputs));
}

const BASE64_ALPHABET =
  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function encodeBase64Chunk(bytes) {
  let result = '';
  let i = 0;

  for (; i + 2 < bytes.length; i += 3) {
    const chunk = (bytes[i] << 16) | (bytes[i + 1] << 8) | bytes[i + 2];
    result +=
      BASE64_ALPHABET[(chunk >> 18) & 0x3f] +
      BASE64_ALPHABET[(chunk >> 12) & 0x3f] +
      BASE64_ALPHABET[(chunk >> 6) & 0x3f] +
      BASE64_ALPHABET[chunk & 0x3f];
  }

  const remaining = bytes.length - i;
  if (remaining === 1) {
    const chunk = bytes[i] << 16;
    result += BASE64_ALPHABET[(chunk >> 18) & 0x3f] + BASE64_ALPHABET[(chunk >> 12) & 0x3f] + '==';
  } else if (remaining === 2) {
    const chunk = (bytes[i] << 16) | (bytes[i + 1] << 8);
    result +=
      BASE64_ALPHABET[(chunk >> 18) & 0x3f] +
      BASE64_ALPHABET[(chunk >> 12) & 0x3f] +
      BASE64_ALPHABET[(chunk >> 6) & 0x3f] +
      '=';
  }

  return result;
}

function arrayBufferToBase64(buffer, chunkSizeBytes = 32 * 1024) {
  const bytes = new Uint8Array(buffer);
  if (bytes.length === 0) return '';

  // Keep chunk size aligned to 3-byte groups for exact base64 boundaries.
  const normalizedChunkSize = Math.max(3, chunkSizeBytes - (chunkSizeBytes % 3));

  const encodedChunks = [];
  for (let offset = 0; offset < bytes.length; offset += normalizedChunkSize) {
    const end = Math.min(offset + normalizedChunkSize, bytes.length);
    encodedChunks.push(encodeBase64Chunk(bytes.subarray(offset, end)));
  }

  return encodedChunks.join('');
}

/* ── Test helpers ────────────────────────────────────────────────────── */

/** Independent oracle: Node's own base64 encoder. */
function bufferToBase64(bytes) {
  return Buffer.from(bytes).toString('base64');
}

function bytesOf(length, seed = 7) {
  const out = new Uint8Array(length);
  for (let i = 0; i < length; i++) out[i] = (i * 31 + seed) & 0xff;
  return out;
}

function sourceText() {
  return fs.readFileSync(path.join(__dirname, 'utils.ts'), 'utf8');
}

/* ── cn() ────────────────────────────────────────────────────────────── */

test('cn: returns an empty string when no input is given', () => {
  assert.equal(cn(), '');
});

test('cn: drops falsy values (null, undefined, false, 0, NaN, "")', () => {
  assert.equal(cn('', null, undefined, false, 0, NaN), '');
});

test('cn: keeps non-conflicting utility classes', () => {
  assert.equal(cn('px-2', 'py-4', 'font-bold'), 'px-2 py-4 font-bold');
});

test('cn: last conflicting class wins', () => {
  assert.equal(cn('text-red-500', 'text-blue-500'), 'text-blue-500');
  assert.equal(cn('p-2', 'p-4', 'p-3'), 'p-3');
  assert.equal(cn('w-1/2', 'w-full'), 'w-full');
});

test('cn: a later shorthand supersedes earlier longhand classes', () => {
  assert.equal(cn('px-2', 'py-4', 'p-4'), 'p-4');
  // …but longhand after shorthand is kept, because it is more specific.
  assert.equal(cn('p-4', 'px-2'), 'p-4 px-2');
});

test('cn: resolves conflicts per variant, not across variants', () => {
  assert.equal(cn('hover:p-2', 'hover:p-4'), 'hover:p-4');
  assert.equal(
    cn('bg-red-500', 'bg-blue-500', 'hover:bg-green-500'),
    'bg-blue-500 hover:bg-green-500',
  );
});

test('cn: merges nested arrays and conditional objects', () => {
  assert.equal(cn('a', false && 'b', ['c', { d: true, e: false }], null), 'a c d');
  assert.equal(cn(['flex', 'items-center'], 'gap-2'), 'flex items-center gap-2');
  assert.equal(cn({ 'sr-only': true, hidden: false, block: true }), 'sr-only block');
});

test('cn: handles negative and important modifiers', () => {
  assert.equal(cn('mt-4', '-mt-2'), '-mt-2');
  assert.equal(cn('!p-2', 'p-4'), '!p-2 p-4');
});

test('cn: merges grid-cols conflicts like other utilities', () => {
  assert.equal(cn('grid', 'grid-cols-2', 'grid-cols-3'), 'grid grid-cols-3');
});

/* ── arrayBufferToBase64() ───────────────────────────────────────────── */

test('arrayBufferToBase64: empty buffer encodes to an empty string', () => {
  assert.equal(arrayBufferToBase64(new ArrayBuffer(0)), '');
});

test('arrayBufferToBase64: 1/2/3-byte tail padding matches the base64 spec', () => {
  // Pinned literals: [7] -> "Bw==", [7,38] -> "ByY=", [7,38,69] -> "ByZF".
  assert.equal(arrayBufferToBase64(bytesOf(1).buffer), 'Bw==');
  assert.equal(arrayBufferToBase64(bytesOf(2).buffer), 'ByY=');
  assert.equal(arrayBufferToBase64(bytesOf(3).buffer), 'ByZF');
});

test('arrayBufferToBase64: matches Buffer for every length 1..70', () => {
  for (let len = 1; len <= 70; len++) {
    const bytes = bytesOf(len);
    assert.equal(
      arrayBufferToBase64(bytes.buffer),
      bufferToBase64(bytes),
      `length ${len} diverged`,
    );
  }
});

test('arrayBufferToBase64: round-trips arbitrary bytes through the oracle', () => {
  const bytes = bytesOf(1000, 3);
  const encoded = arrayBufferToBase64(bytes.buffer);
  assert.deepEqual(new Uint8Array(Buffer.from(encoded, 'base64')), bytes);
});

test('arrayBufferToBase64: encodes multi-byte UTF-8 text', () => {
  const text = 'héllo ✓ 世界 🚀';
  const bytes = new TextEncoder().encode(text);
  assert.equal(arrayBufferToBase64(bytes.buffer), bufferToBase64(bytes));
  assert.equal(Buffer.from(arrayBufferToBase64(bytes.buffer), 'base64').toString('utf8'), text);
});

test('arrayBufferToBase64: chunking never emits interior padding', () => {
  // 32 KiB default chunk is not a multiple of 3, so the helper must realign it.
  const bytes = bytesOf(32 * 1024 + 1);
  const encoded = arrayBufferToBase64(bytes.buffer);
  assert.equal(encoded, bufferToBase64(bytes));
  assert.equal(encoded.slice(0, -2).includes('='), false, 'padding may only appear at the end');
});

test('arrayBufferToBase64: honours explicit chunk sizes 3..10 on multi-chunk input', () => {
  const bytes = bytesOf(1000, 11);
  const expected = bufferToBase64(bytes);
  for (let chunk = 3; chunk <= 10; chunk++) {
    assert.equal(arrayBufferToBase64(bytes.buffer, chunk), expected, `chunk ${chunk} diverged`);
  }
});

test('arrayBufferToBase64: normalizes degenerate chunk sizes to 3 bytes', () => {
  const bytes = bytesOf(10, 5);
  const expected = bufferToBase64(bytes);
  for (const chunk of [0, 1, 2, -1, -100]) {
    assert.equal(arrayBufferToBase64(bytes.buffer, chunk), expected, `chunk ${chunk} diverged`);
  }
});

test('arrayBufferToBase64: handles large buffers across many chunks', () => {
  const bytes = bytesOf(256 * 1024, 13);
  assert.equal(arrayBufferToBase64(bytes.buffer), bufferToBase64(bytes));
  assert.equal(arrayBufferToBase64(bytes.buffer, 256 * 1024), bufferToBase64(bytes));
});

test('arrayBufferToBase64: does not mutate the source buffer', () => {
  const bytes = bytesOf(500, 17);
  const before = Uint8Array.from(bytes);
  arrayBufferToBase64(bytes.buffer, 7);
  assert.deepEqual(bytes, before);
});

test('arrayBufferToBase64: encodes the whole backing buffer, ignoring view windows', () => {
  // The helper wraps the entire ArrayBuffer, so a subarray view is *not*
  // honoured — callers must slice the buffer themselves. Pinned so a future
  // change to view-aware behaviour is a deliberate decision.
  const backing = bytesOf(9, 19);
  const view = backing.subarray(3, 6);
  assert.equal(arrayBufferToBase64(view.buffer), bufferToBase64(backing));
  assert.notEqual(arrayBufferToBase64(view.buffer), bufferToBase64(Uint8Array.from(view)));
});

/* ── Source contract ─────────────────────────────────────────────────── */

test('utils.ts still exports the helpers mirrored above', () => {
  const src = sourceText();
  assert.match(src, /export function cn\(/);
  assert.match(src, /export function arrayBufferToBase64\(/);
  assert.match(src, /chunkSizeBytes = 32 \* 1024/);
});
