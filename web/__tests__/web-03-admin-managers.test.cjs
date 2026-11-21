// web/__tests__/web-03-admin-managers.test.cjs — source-contract and mirrored
// state tests for WEB-03 (issue #804): the App Router admin page must load the
// authorized manager API and distinguish loading, populated, forbidden and
// service-error states without ever caching privileged data.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('path');

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');

// Mirrors web/lib/adminManagersState.ts so the mapping is executable by
// node --test without a TypeScript build step.
function stateFromLoadResult(rowCount) {
  return rowCount > 0 ? 'populated' : 'empty';
}
function stateFromLoadError(error) {
  if (error && typeof error.status === 'number') {
    if (error.status === 401) return 'unauthenticated';
    if (error.status === 403) return 'forbidden';
  }
  return 'service-error';
}

test('the App Router page renders a client component instead of a fake empty state', () => {
  const page = read('app/admin/managers/page.tsx');
  assert.ok(page.includes('export const metadata: Metadata'), 'metadata stays in the server component');
  assert.ok(page.includes('import AdminManagersClient'), 'client component is imported');
  assert.ok(page.includes('<AdminManagersClient />'), 'client component is rendered');
  assert.ok(!/TODO/.test(page), 'placeholder TODO was removed');
});

test('the API helper requests managers with no-store caching', () => {
  const api = read('lib/api.ts');
  assert.ok(api.includes('export interface PagedResponse<T>'), 'paged response type is exported');
  assert.ok(api.includes('listAdminManagers(token?: string)'), 'typed admin request exists');
  assert.ok(api.includes('"/managers"'), 'uses the admin-protected manager endpoint');
  assert.ok(api.includes('cache: "no-store"'), 'admin read is not cached');
  assert.ok(api.includes('token,'), 'bearer token is forwarded when available');
});

test('forbidden and unauthenticated responses are distinct from service errors', () => {
  const helper = read('lib/adminManagersState.ts');
  assert.ok(helper.includes('status === 401'), '401 mapped to unauthenticated');
  assert.ok(helper.includes('status === 403'), '403 mapped to forbidden');
  assert.ok(helper.includes('"service-error"'), 'other failures mapped to service error');
  assert.ok(!/length\s*===\s*0.*authorized/.test(helper), 'empty list is not an auth signal');

  assert.equal(stateFromLoadError({ status: 401 }), 'unauthenticated');
  assert.equal(stateFromLoadError({ status: 403 }), 'forbidden');
  assert.equal(stateFromLoadError({ status: 500 }), 'service-error');
  assert.equal(stateFromLoadError(new TypeError('network down')), 'service-error');
});

test('loading, populated and empty states are conveyed to assistive technology', () => {
  const component = read('components/admin/AdminManagersClient.tsx');
  assert.ok(component.includes('data-testid="admin-managers-live-status"'), 'live region is mounted');
  assert.ok(component.includes('LiveStatus'), 'status announcements are used');
  assert.ok(component.includes('state === "loading"'), 'loading state is rendered');
  assert.ok(component.includes('state === "populated"'), 'populated state is rendered');
  assert.ok(component.includes('state === "empty"'), 'empty state is rendered');
  assert.ok(component.includes('role="alert"'), 'failures interrupt');
  assert.ok(!/cachedGet/.test(component), 'admin reads bypass the shared cache helper');
});

test('a privilege loss clears cached manager data before showing the error', () => {
  const component = read('components/admin/AdminManagersClient.tsx');
  assert.ok(component.includes('clearCache()'), 'shared cache is cleared on failure');
  assert.ok(component.includes('setManagers([])'), 'manager rows are dropped on failure');
  const clearIndex = component.indexOf('clearCache()');
  const errorIndex = component.indexOf('setState(stateFromLoadError(error))');
  assert.ok(clearIndex > -1 && errorIndex > clearIndex, 'cache is cleared before error state');
});

test('load results map to populated or empty without authorization assumptions', () => {
  assert.equal(stateFromLoadResult(3), 'populated');
  assert.equal(stateFromLoadResult(0), 'empty');
});

test('new admin strings are localized', () => {
  const messages = JSON.parse(read('messages/en.json'));
  for (const key of [
    'admin.managers.forbidden',
    'admin.managers.unauthenticated',
    'admin.managers.serviceUnavailable',
  ]) {
    assert.ok(messages[key], `missing translation for ${key}`);
  }
});
