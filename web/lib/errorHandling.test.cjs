// web/lib/errorHandling.test.cjs — unit tests for the backend error parsing and
// formatting helpers in web/lib/errorHandling.ts (Issue #475 / WEB-21).
// Runs with `node --test ./lib/**/*.test.cjs`.
//
// `package.json`'s `test` script only executes `.cjs` files, so — following the
// convention already used by web/lib/api-middleware.test.cjs — the pure logic is
// mirrored here in plain JS. The mirror is a line-for-line port, so the
// assertions below pin the *actual* error-type mapping, fallback ordering and
// WASM pattern precedence that the app relies on.
//
// If you change behaviour in web/lib/errorHandling.ts, mirror the change here.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

/* ── Mirror of web/lib/errorHandling.ts ──────────────────────────────── */

async function extractErrorDetails(response) {
  try {
    const data = await response.json();
    return {
      error: data.error || 'UNKNOWN_ERROR',
      message: data.message || response.statusText || 'An error occurred',
      statusCode: response.status,
    };
  } catch {
    return {
      error: getErrorType(response.status),
      message: response.statusText || 'An error occurred',
      statusCode: response.status,
    };
  }
}

function getErrorType(status) {
  switch (status) {
    case 400:
      return 'BAD_REQUEST';
    case 401:
      return 'UNAUTHORIZED';
    case 404:
      return 'NOT_FOUND';
    case 500:
      return 'INTERNAL_SERVER_ERROR';
    case 503:
      return 'SERVICE_UNAVAILABLE';
    default:
      return 'UNKNOWN_ERROR';
  }
}

function formatError(error) {
  if (error instanceof Response) {
    return {
      type: getErrorType(error.status),
      message: error.statusText || 'Network error',
      statusCode: error.status,
      isNetworkError: true,
    };
  }

  if (error instanceof TypeError) {
    return {
      type: 'NETWORK_ERROR',
      message: 'Failed to connect to backend. Please ensure the server is running.',
      details: error.message,
      statusCode: 0,
      isNetworkError: true,
    };
  }

  if (error instanceof Error) {
    if (error.message.includes('JSON')) {
      return {
        type: 'PARSE_ERROR',
        message: 'Failed to parse response from backend',
        details: error.message,
        statusCode: 0,
        isNetworkError: false,
      };
    }

    return {
      type: 'ERROR',
      message: error.message || 'An unexpected error occurred',
      statusCode: 0,
      isNetworkError: false,
    };
  }

  return {
    type: 'UNKNOWN_ERROR',
    message: 'An unexpected error occurred',
    statusCode: 0,
    isNetworkError: false,
  };
}

function createUserFriendlyMessage(errorResponse) {
  const errorMessages = {
    BAD_REQUEST: 'Invalid request. Please check your inputs and try again.',
    UNAUTHORIZED: 'You are not authorized to perform this action.',
    NOT_FOUND: 'The requested resource was not found.',
    INTERNAL_SERVER_ERROR: 'Server error. Please try again later.',
    SERVICE_UNAVAILABLE: 'The service is currently unavailable. Please try again later.',
  };

  return (
    errorMessages[errorResponse.error] ||
    errorResponse.message ||
    'An error occurred during analysis'
  );
}

function parseWasmError(response, errorMessage) {
  const status = response.status;

  const wasmErrorPatterns = [
    {
      pattern: /Invalid base64|base64 decoding|base64 WASM data/i,
      title: 'Invalid WASM Encoding',
      details: () =>
        "The file appears to be corrupted or improperly encoded. Ensure you're uploading a valid compiled Soroban contract.",
    },
    {
      pattern: /Invalid WASM|malformed|not a valid WebAssembly/i,
      title: 'Invalid WASM Format',
      details: () =>
        "This doesn't appear to be a valid WebAssembly module. Make sure you're uploading a compiled .wasm file from Soroban.",
    },
    {
      pattern: /version|unsupported/i,
      title: 'Unsupported WASM Version',
      details: () =>
        'The WASM version is not supported. Please recompile using a compatible Soroban version.',
    },
    {
      pattern: /memory|out of|limit|overflow/i,
      title: 'WASM Resource Exceeded',
      details: () =>
        'The contract exceeds analysis resource limits. Try simplifying the contract or splitting it into smaller modules.',
    },
    {
      pattern: /timeout|took too long|analysis timeout/i,
      title: 'Analysis Timeout',
      details: () =>
        'The analysis took too long to complete. The contract might be too complex. Please try again or simplify the contract.',
    },
    {
      pattern: /function|export|not found/i,
      title: 'Function Not Found',
      details: () =>
        'The specified contract function was not found. Ensure the function is properly exported from your contract.',
    },
  ];

  for (const { pattern, title, details } of wasmErrorPatterns) {
    const match = pattern.exec(errorMessage);
    if (match) {
      return {
        title,
        message: details(match),
        statusCode: status,
        suggestedAction: 'Please check your contract and try uploading again.',
      };
    }
  }

  const defaultErrors = {
    400: {
      title: 'Invalid WASM File',
      message:
        errorMessage ||
        "The backend rejected the WASM file. Please ensure it's a valid compiled Soroban contract.",
      statusCode: 400,
      suggestedAction: 'Try uploading a different contract or check the build logs.',
    },
    401: {
      title: 'Unauthorized',
      message: "You don't have permission to analyze contracts.",
      statusCode: 401,
      suggestedAction: 'Please connect your wallet and try again.',
    },
    413: {
      title: 'File Too Large',
      message: 'The WASM file is too large for analysis.',
      statusCode: 413,
      suggestedAction: 'Optimize your contract to reduce its size.',
    },
    500: {
      title: 'Server Error',
      message: 'The backend encountered an error while analyzing your contract.',
      statusCode: 500,
      suggestedAction: 'Please try again later.',
    },
    503: {
      title: 'Service Unavailable',
      message: 'The analysis service is temporarily unavailable.',
      statusCode: 503,
      suggestedAction: 'Please try again in a few moments.',
    },
  };

  return (
    defaultErrors[status] || {
      title: 'Analysis Failed',
      message: errorMessage || 'An error occurred while analyzing the WASM file.',
      statusCode: status,
      suggestedAction: 'Please try uploading again.',
    }
  );
}

/* ── Test helpers ────────────────────────────────────────────────────── */

function jsonResponse(body, status = 400, statusText = '') {
  return new Response(typeof body === 'string' ? body : JSON.stringify(body), {
    status,
    statusText,
    headers: { 'content-type': 'application/json' },
  });
}

function sourceText() {
  return fs.readFileSync(path.join(__dirname, 'errorHandling.ts'), 'utf8');
}

/* ── extractErrorDetails() ───────────────────────────────────────────── */

test('extractErrorDetails: reads error, message and status from a JSON body', async () => {
  const details = await extractErrorDetails(
    jsonResponse({ error: 'BAD_REQUEST', message: 'contract_id is required' }, 400),
  );
  assert.deepEqual(details, {
    error: 'BAD_REQUEST',
    message: 'contract_id is required',
    statusCode: 400,
  });
});

test('extractErrorDetails: falls back to UNKNOWN_ERROR when the field is missing', async () => {
  const details = await extractErrorDetails(jsonResponse({ message: 'boom' }, 500));
  assert.equal(details.error, 'UNKNOWN_ERROR');
  assert.equal(details.message, 'boom');
  assert.equal(details.statusCode, 500);
});

test('extractErrorDetails: falls back to statusText, then to the generic message', async () => {
  const withText = await extractErrorDetails(jsonResponse('{}', 404, 'Not Found'));
  assert.deepEqual(withText, {
    error: 'UNKNOWN_ERROR',
    message: 'Not Found',
    statusCode: 404,
  });

  const withoutText = await extractErrorDetails(jsonResponse('{}', 404));
  assert.equal(withoutText.message, 'An error occurred');
});

test('extractErrorDetails: non-JSON body maps the status code to an error type', async () => {
  const details = await extractErrorDetails(
    new Response('<html>gateway error</html>', { status: 503, statusText: 'Service Unavailable' }),
  );
  assert.deepEqual(details, {
    error: 'SERVICE_UNAVAILABLE',
    message: 'Service Unavailable',
    statusCode: 503,
  });
});

test('extractErrorDetails: unmapped status with a non-JSON body resolves to UNKNOWN_ERROR', async () => {
  const details = await extractErrorDetails(new Response('nope', { status: 418 }));
  assert.equal(details.error, 'UNKNOWN_ERROR');
  assert.equal(details.message, 'An error occurred');
});

test('extractErrorDetails: a JSON `null` body throws inside the try and hits the fallback', async () => {
  // `data.error` on null raises a TypeError, which the catch block absorbs.
  const details = await extractErrorDetails(jsonResponse('null', 401, 'Unauthorized'));
  assert.deepEqual(details, {
    error: 'UNAUTHORIZED',
    message: 'Unauthorized',
    statusCode: 401,
  });
});

/* ── formatError() — Response instances ──────────────────────────────── */

test('formatError: maps every known Response status to its error type', () => {
  const expectations = [
    [400, 'BAD_REQUEST'],
    [401, 'UNAUTHORIZED'],
    [404, 'NOT_FOUND'],
    [500, 'INTERNAL_SERVER_ERROR'],
    [503, 'SERVICE_UNAVAILABLE'],
    [418, 'UNKNOWN_ERROR'],
  ];

  for (const [status, type] of expectations) {
    const formatted = formatError(new Response(null, { status }));
    assert.equal(formatted.type, type);
    assert.equal(formatted.statusCode, status);
    assert.equal(formatted.isNetworkError, true);
    assert.equal(formatted.details, undefined);
  }
});

test('formatError: a Response without statusText reports "Network error"', () => {
  assert.equal(formatError(new Response(null, { status: 500 })).message, 'Network error');
});

test('formatError: a Response with statusText keeps it verbatim', () => {
  const formatted = formatError(new Response(null, { status: 504, statusText: 'Gateway Timeout' }));
  assert.equal(formatted.message, 'Gateway Timeout');
  assert.equal(formatted.type, 'UNKNOWN_ERROR');
  assert.equal(formatted.statusCode, 504);
});

/* ── formatError() — thrown values ───────────────────────────────────── */

test('formatError: TypeError becomes a NETWORK_ERROR with the original detail', () => {
  const formatted = formatError(new TypeError('fetch failed'));
  assert.deepEqual(formatted, {
    type: 'NETWORK_ERROR',
    message: 'Failed to connect to backend. Please ensure the server is running.',
    details: 'fetch failed',
    statusCode: 0,
    isNetworkError: true,
  });
});

test('formatError: a JSON parse error becomes PARSE_ERROR', () => {
  const formatted = formatError(new SyntaxError('Unexpected token < in JSON at position 0'));
  assert.equal(formatted.type, 'PARSE_ERROR');
  assert.equal(formatted.message, 'Failed to parse response from backend');
  assert.equal(formatted.details, 'Unexpected token < in JSON at position 0');
  assert.equal(formatted.statusCode, 0);
  assert.equal(formatted.isNetworkError, false);
});

test('formatError: the JSON check is case-sensitive', () => {
  const formatted = formatError(new Error('invalid json payload'));
  assert.equal(formatted.type, 'ERROR');
  assert.equal(formatted.message, 'invalid json payload');
});

test('formatError: a plain Error keeps its message', () => {
  const formatted = formatError(new Error('analysis exploded'));
  assert.deepEqual(formatted, {
    type: 'ERROR',
    message: 'analysis exploded',
    statusCode: 0,
    isNetworkError: false,
  });
});

test('formatError: an Error with an empty message gets a friendly fallback', () => {
  const formatted = formatError(new Error(''));
  assert.equal(formatted.type, 'ERROR');
  assert.equal(formatted.message, 'An unexpected error occurred');
});

test('formatError: non-Error throwables resolve to UNKNOWN_ERROR', () => {
  for (const thrown of ['boom', 42, null, undefined, { message: 'shape-alike' }, ['a']]) {
    const formatted = formatError(thrown);
    assert.deepEqual(
      formatted,
      {
        type: 'UNKNOWN_ERROR',
        message: 'An unexpected error occurred',
        statusCode: 0,
        isNetworkError: false,
      },
      `unexpected result for ${JSON.stringify(thrown)}`,
    );
  }
});

test('formatError: always returns the full FormattedError shape', () => {
  const allowed = ['type', 'message', 'details', 'statusCode', 'isNetworkError'];
  for (const thrown of [new Response(null, { status: 500 }), new TypeError('x'), new Error('y'), 'z']) {
    const formatted = formatError(thrown);
    assert.equal(typeof formatted.type, 'string');
    assert.equal(typeof formatted.message, 'string');
    assert.equal(typeof formatted.statusCode, 'number');
    assert.equal(typeof formatted.isNetworkError, 'boolean');
    for (const key of Object.keys(formatted)) {
      assert.ok(allowed.includes(key), `unexpected key ${key}`);
    }
  }
});

/* ── createUserFriendlyMessage() ─────────────────────────────────────── */

test('createUserFriendlyMessage: maps each known error type to its copy', () => {
  const expectations = {
    BAD_REQUEST: 'Invalid request. Please check your inputs and try again.',
    UNAUTHORIZED: 'You are not authorized to perform this action.',
    NOT_FOUND: 'The requested resource was not found.',
    INTERNAL_SERVER_ERROR: 'Server error. Please try again later.',
    SERVICE_UNAVAILABLE: 'The service is currently unavailable. Please try again later.',
  };

  for (const [type, copy] of Object.entries(expectations)) {
    assert.equal(
      createUserFriendlyMessage({ error: type, message: 'raw backend text' }),
      copy,
      `${type} must win over the raw message`,
    );
  }
});

test('createUserFriendlyMessage: unknown types fall back to the backend message', () => {
  assert.equal(
    createUserFriendlyMessage({ error: 'WAT', message: 'raw backend text' }),
    'raw backend text',
  );
});

test('createUserFriendlyMessage: empty message falls back to the generic copy', () => {
  assert.equal(
    createUserFriendlyMessage({ error: 'WAT', message: '' }),
    'An error occurred during analysis',
  );
});

/* ── parseWasmError() — message patterns ─────────────────────────────── */

test('parseWasmError: base64 patterns map to Invalid WASM Encoding', () => {
  for (const message of ['Invalid base64 input', 'base64 decoding failed', 'bad base64 WASM data']) {
    const parsed = parseWasmError(new Response(null, { status: 400 }), message);
    assert.equal(parsed.title, 'Invalid WASM Encoding');
    assert.equal(parsed.statusCode, 400);
    assert.equal(parsed.suggestedAction, 'Please check your contract and try uploading again.');
  }
});

test('parseWasmError: format patterns map to Invalid WASM Format', () => {
  for (const message of ['Invalid WASM module', 'malformed header', 'not a valid WebAssembly binary']) {
    assert.equal(parseWasmError(new Response(null, { status: 400 }), message).title, 'Invalid WASM Format');
  }
});

test('parseWasmError: version patterns map to Unsupported WASM Version', () => {
  assert.equal(
    parseWasmError(new Response(null, { status: 400 }), 'unsupported version 7').title,
    'Unsupported WASM Version',
  );
});

test('parseWasmError: resource patterns map to WASM Resource Exceeded', () => {
  for (const message of ['out of memory', 'resource limit hit', 'counter overflow']) {
    assert.equal(parseWasmError(new Response(null, { status: 400 }), message).title, 'WASM Resource Exceeded');
  }
});

test('parseWasmError: timeout patterns map to Analysis Timeout', () => {
  for (const message of ['request timeout', 'analysis took too long', 'analysis timeout exceeded']) {
    assert.equal(parseWasmError(new Response(null, { status: 504 }), message).title, 'Analysis Timeout');
  }
});

test('parseWasmError: export patterns map to Function Not Found', () => {
  for (const message of ['function missing', 'export table empty', 'symbol not found']) {
    assert.equal(parseWasmError(new Response(null, { status: 400 }), message).title, 'Function Not Found');
  }
});

test('parseWasmError: pattern matching is case-insensitive', () => {
  assert.equal(parseWasmError(new Response(null, { status: 400 }), 'TIMEOUT').title, 'Analysis Timeout');
});

test('parseWasmError: the earliest matching pattern wins', () => {
  // Both the version and timeout patterns match; the version entry is listed first.
  const parsed = parseWasmError(new Response(null, { status: 400 }), 'unsupported version caused a timeout');
  assert.equal(parsed.title, 'Unsupported WASM Version');
});

test('parseWasmError: pattern matches keep the live HTTP status', () => {
  assert.equal(parseWasmError(new Response(null, { status: 502 }), 'malformed input').statusCode, 502);
});

/* ── parseWasmError() — status fallbacks ─────────────────────────────── */

test('parseWasmError: 400 falls back to Invalid WASM File and echoes the message', () => {
  const parsed = parseWasmError(new Response(null, { status: 400 }), 'rejected by backend');
  assert.equal(parsed.title, 'Invalid WASM File');
  assert.equal(parsed.message, 'rejected by backend');
  assert.equal(parsed.statusCode, 400);
});

test('parseWasmError: 400 with an empty message uses the canned copy', () => {
  const parsed = parseWasmError(new Response(null, { status: 400 }), '');
  assert.equal(parsed.title, 'Invalid WASM File');
  assert.match(parsed.message, /valid compiled Soroban contract/);
});

test('parseWasmError: 401 always returns the Unauthorized copy', () => {
  const parsed = parseWasmError(new Response(null, { status: 401 }), 'anything at all unique');
  assert.deepEqual(parsed, {
    title: 'Unauthorized',
    message: "You don't have permission to analyze contracts.",
    statusCode: 401,
    suggestedAction: 'Please connect your wallet and try again.',
  });
});

test('parseWasmError: 413, 500 and 503 have dedicated copy', () => {
  assert.equal(parseWasmError(new Response(null, { status: 413 }), 'too big').title, 'File Too Large');
  assert.equal(parseWasmError(new Response(null, { status: 500 }), 'kaboom').title, 'Server Error');
  assert.equal(
    parseWasmError(new Response(null, { status: 503 }), 'down').title,
    'Service Unavailable',
  );
});

test('parseWasmError: unknown status with a message becomes Analysis Failed', () => {
  const parsed = parseWasmError(new Response(null, { status: 502 }), 'upstream blew up');
  assert.deepEqual(parsed, {
    title: 'Analysis Failed',
    message: 'upstream blew up',
    statusCode: 502,
    suggestedAction: 'Please try uploading again.',
  });
});

test('parseWasmError: unknown status with an empty message uses the generic copy', () => {
  const parsed = parseWasmError(new Response(null, { status: 502 }), '');
  assert.equal(parsed.title, 'Analysis Failed');
  assert.equal(parsed.message, 'An error occurred while analyzing the WASM file.');
  assert.equal(parsed.statusCode, 502);
});

test('parseWasmError: an unlisted 2xx status still resolves to Analysis Failed', () => {
  const parsed = parseWasmError(new Response(null, { status: 200 }), '');
  assert.equal(parsed.title, 'Analysis Failed');
  assert.equal(parsed.statusCode, 200);
});

/* ── Source contract ─────────────────────────────────────────────────── */

test('errorHandling.ts still exports the helpers mirrored above', () => {
  const src = sourceText();
  assert.match(src, /export async function extractErrorDetails\(/);
  assert.match(src, /export function formatError\(/);
  assert.match(src, /export function createUserFriendlyMessage\(/);
  assert.match(src, /export function parseWasmError\(/);
});
