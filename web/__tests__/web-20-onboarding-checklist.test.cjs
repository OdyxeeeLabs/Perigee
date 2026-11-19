// web/__tests__/web-20-onboarding-checklist.test.cjs — source-contract and
// mirrored state-mapping tests for WEB-20 (issue #821).
//
// The test runner has no React renderer, so component contracts are pinned by
// reading source and the pure lifecycle mapping is mirrored here. Run with
// `cd web && node --test ./__tests__/web-20-onboarding-checklist.test.cjs`.

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('path');

const read = (relative) =>
  fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');

// Mirror of web/lib/onboardingChecklist.ts. Keep in sync with the source test
// below; it exists so every lifecycle state can be exercised by node --test.
function derive(status, walletConnected) {
  const submitted = status !== 'unregistered';
  const reviewState =
    status === 'approved' ? 'complete' : status === 'rejected' ? 'blocked' : 'pending';
  return [
    { key: 'wallet', actor: 'user', state: walletConnected ? 'complete' : 'pending' },
    { key: 'details', actor: 'user', state: submitted ? 'complete' : 'pending' },
    { key: 'kyc', actor: 'user', state: submitted ? 'complete' : 'pending' },
    { key: 'review', actor: 'admin', state: reviewState },
  ];
}

function statesFor(status, walletConnected) {
  return Object.fromEntries(derive(status, walletConnected).map((s) => [s.key, s.state]));
}

test('the helper derives progress only from API status and wallet connection', () => {
  const src = read('lib/onboardingChecklist.ts');

  assert.ok(src.includes('export function deriveOnboardingChecklist'), 'pure helper is exported');
  assert.ok(src.includes('status !== "unregistered"'), 'submission comes from API status');
  assert.ok(src.includes('status === "approved"'), 'approval comes from API status');
  assert.ok(src.includes('status === "rejected"'), 'rejection comes from API status');
  assert.ok(src.includes('walletConnected'), 'wallet flag is explicit');
  assert.ok(!/localStorage|sessionStorage/.test(src), 'local form draft must not infer progress');
});

test('the checklist labels actor and state in text, not colour alone', () => {
  const src = read('components/manager/OnboardingChecklist.tsx');

  assert.ok(src.includes('data-testid="onboarding-checklist"'), 'checklist is mounted for tests');
  assert.ok(src.includes('aria-labelledby="onboarding-checklist-heading"'), 'checklist has a heading');
  assert.ok(src.includes('onboarding.checklistActorYou'), 'user actions are labelled');
  assert.ok(src.includes('onboarding.checklistActorAdmin'), 'admin review is labelled');
  assert.ok(src.includes('onboarding.checklistStateComplete'), 'complete is labelled');
  assert.ok(src.includes('onboarding.checklistStatePending'), 'pending is labelled');
  assert.ok(src.includes('onboarding.checklistStateBlocked'), 'blocked is labelled');
  assert.ok(src.includes('aria-hidden="true"'), 'decorative marker is hidden from AT');
  assert.ok(src.includes('role="alert"'), 'API errors are announced');
});

test('the onboarding page feeds the checklist from the status API', () => {
  const src = read('pages/managers/onboarding.tsx');

  assert.ok(
    src.includes('import OnboardingChecklist from "../../components/manager/OnboardingChecklist"'),
    'checklist component is imported',
  );
  assert.ok(
    src.includes('useState<OnboardingStatus>("unregistered")'),
    'initial state is the documented unregistered lifecycle',
  );
  for (const status of ['approved', 'pending', 'rejected', 'unregistered']) {
    assert.ok(
      src.includes(`setOnboardingStatus("${status}")`),
      `${status} status is stored from the API response`,
    );
  }
  assert.ok(src.includes('setOnboardingStatus("pending")'), 'successful registration becomes pending');
  assert.ok(src.includes('<OnboardingChecklist'), 'checklist is rendered');
  assert.ok(
    src.includes('walletConnected={Boolean(address)}'),
    'wallet flag is derived from the connected address',
  );
});

test('every documented lifecycle state maps to the expected steps', () => {
  assert.deepEqual(statesFor('unregistered', false), {
    wallet: 'pending',
    details: 'pending',
    kyc: 'pending',
    review: 'pending',
  });
  assert.deepEqual(statesFor('unregistered', true), {
    wallet: 'complete',
    details: 'pending',
    kyc: 'pending',
    review: 'pending',
  });
  assert.deepEqual(statesFor('pending', true), {
    wallet: 'complete',
    details: 'complete',
    kyc: 'complete',
    review: 'pending',
  });
  assert.deepEqual(statesFor('approved', true), {
    wallet: 'complete',
    details: 'complete',
    kyc: 'complete',
    review: 'complete',
  });
  assert.deepEqual(statesFor('rejected', true), {
    wallet: 'complete',
    details: 'complete',
    kyc: 'complete',
    review: 'blocked',
  });
});

test('administrator review is always a separate admin-owned step', () => {
  for (const status of ['unregistered', 'pending', 'approved', 'rejected']) {
    const review = derive(status, true).find((step) => step.key === 'review');
    assert.equal(review.actor, 'admin', `${status} keeps review with the administrator`);
  }
  const completed = derive('approved', true);
  assert.equal(completed.filter((step) => step.state === 'complete').length, 4);
});

test('new checklist strings are localized', () => {
  const messages = JSON.parse(read('messages/en.json'));
  const required = [
    'onboarding.checklistTitle',
    'onboarding.checklistActorYou',
    'onboarding.checklistActorAdmin',
    'onboarding.checklistStateComplete',
    'onboarding.checklistStatePending',
    'onboarding.checklistStateBlocked',
    'onboarding.checklistStepWallet',
    'onboarding.checklistStepDetails',
    'onboarding.checklistStepKyc',
    'onboarding.checklistStepReview',
    'onboarding.checklistError',
  ];
  for (const key of required) {
    assert.ok(messages[key], `missing translation for ${key}`);
  }
});
