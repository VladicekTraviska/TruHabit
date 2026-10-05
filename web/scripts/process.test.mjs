import { test } from 'node:test';
import assert from 'node:assert/strict';
import { beginApiTrace, finishApiTrace, emitProcess, clearProcess, getProcessSnapshot, resetProcessSession, getProcessScope } from '../src/process.ts';

test('auth and wallet identity requests are never traced', () => {
  clearProcess();
  for (const path of ['/api/auth/login', '/api/auth/session', '/api/wallet/challenge', '/api/wallet/link']) assert.equal(beginApiTrace(path, 'POST'), null);
  assert.equal(getProcessSnapshot().length, 0);
});
test('local credit balances are not reported as blockchain observations', () => {
  clearProcess();
  assert.equal(beginApiTrace('/api/prototype/local/balance', 'GET').category, 'network');
  assert.equal(beginApiTrace('/api/prototype/balance', 'GET').category, 'chain');
});

test('team test-credit funding is a network observation and private invitations never enter the log', () => {
  clearProcess();
  const org = '970726d1-96e1-4c3d-b89a-d4bae0fa158a';
  const program = '871a9e67-4dde-4fb0-af61-cf8d3d5b8671';
  const trace = beginApiTrace(`/api/organizations/${org}/programs/${program}/publish`, 'POST');
  assert.equal(trace.category, 'network');
  finishApiTrace(trace, 200);
  assert.ok(getProcessSnapshot().at(-1).en.includes('/api/organizations/:id/programs/:id/publish'));
  for (const path of [`/api/organizations/${org}/invitations?token=private`, '/api/organization-invitations/accept', `/api/organizations/${org}/members/private@example.com`, `/api/organizations/${org}/programs/${program}/enrollments/${org}/uploads/${program}/source`]) assert.equal(beginApiTrace(path, 'POST'), null);
  assert.ok(!JSON.stringify(getProcessSnapshot()).includes('private'));
});

test('team evidence tracing hides identities and FIT session query parameters', () => {
  clearProcess();
  const id = '970726d1-96e1-4c3d-b89a-d4bae0fa158a';
  const trace = beginApiTrace(`/api/organizations/${id}/programs/${id}/enrollments/${id}/upload?session=0`, 'POST');
  assert.equal(trace.category, 'evidence');
  finishApiTrace(trace, 200);
  assert.ok(!JSON.stringify(getProcessSnapshot()).includes(id));
  assert.ok(!JSON.stringify(getProcessSnapshot()).includes('session='));
});
test('private activity source requests never enter the process log', () => {
  clearProcess();
  const id = '970726d1-96e1-4c3d-b89a-d4bae0fa158a';
  for (const path of [
    `/api/prototype/challenges/${id}/uploads/${id}/source?private=hidden`,
    `/api/organizations/${id}/programs/${id}/enrollments/${id}/uploads/${id}/source`,
  ]) {
    for (const method of ['GET', 'DELETE']) {
      const trace = beginApiTrace(path, method);
      assert.equal(trace, null);
      finishApiTrace(trace, 409, 'CONFLICT', 'BUSINESS_REVIEW_PENDING');
    }
  }
  assert.equal(getProcessSnapshot().length, 0);
});
test('trace strips query parameters and challenge identifiers; only bounded codes are allowed', () => {
  clearProcess();
  const trace = beginApiTrace('/api/prototype/challenges/970726d1-96e1-4c3d-b89a-d4bae0fa158a/upload?session=0&private=hidden', 'POST');
  finishApiTrace(trace, 409, 'ACTIVITY_ALREADY_USED');
  const log = JSON.stringify(getProcessSnapshot());
  assert.ok(log.includes(':id/upload'));
  assert.ok(log.includes('ACTIVITY_ALREADY_USED'));
  assert.ok(!log.includes('970726d1'));
  assert.ok(!log.includes('hidden'));
  finishApiTrace(trace, 500, 'arbitrary private text');
  assert.ok(!JSON.stringify(getProcessSnapshot()).includes('arbitrary private text'));
});
test('a successful HTTP status with invalid JSON is not shown as successful', () => {
  clearProcess();
  finishApiTrace(beginApiTrace('/api/prototype/balance', 'GET'), 200, 'INVALID_RESPONSE');
  assert.equal(getProcessSnapshot().at(-1).level, 'error');
  finishApiTrace(beginApiTrace('/api/prototype/balance', 'GET'), 0);
  assert.equal(getProcessSnapshot().at(-1).level, 'error');
});
test('generic prepare errors expose only allowlisted public prerequisites', () => {
  clearProcess();
  finishApiTrace(beginApiTrace('/api/prototype/challenges/970726d1-96e1-4c3d-b89a-d4bae0fa158a/prepare', 'POST'), 400, 'INVALID_INPUT', 'LINK_PHANTOM_FIRST');
  const missingWallet = getProcessSnapshot().at(-1);
  assert.equal(missingWallet.level, 'error');
  assert.ok(missingWallet.en.includes('INVALID_INPUT · LINK_PHANTOM_FIRST'));
  assert.ok(missingWallet.cs.includes('LINK_PHANTOM_FIRST'));
  for (const privateReason of ['private@example.com', 'SECRET_TOKEN', 'LINK_PHANTOM_FIRST\nprivate@example.com']) {
    finishApiTrace(beginApiTrace('/api/prototype/challenges/:id/prepare', 'POST'), 400, 'INVALID_INPUT', privateReason);
    assert.ok(!getProcessSnapshot().at(-1).en.includes(privateReason));
  }
  finishApiTrace(beginApiTrace('/api/prototype/challenges/:id/prepare', 'POST'), 200, undefined, 'LINK_PHANTOM_FIRST');
  assert.ok(!getProcessSnapshot().at(-1).en.includes('LINK_PHANTOM_FIRST'));
});
test('business lifecycle errors explain their public prerequisite without exposing identifiers', () => {
  clearProcess();
  const org = '970726d1-96e1-4c3d-b89a-d4bae0fa158a';
  const program = '871a9e67-4dde-4fb0-af61-cf8d3d5b8671';
  for (const [action, reason] of [
    ['close', 'BUSINESS_UPLOAD_WINDOW_OPEN'],
    ['close', 'BUSINESS_ACCEPTED_REWARDS_MUST_BE_PAID'],
    ['close', 'BUSINESS_REVIEW_PENDING'],
    ['close', 'BUSINESS_VERSION_CHANGED'],
    ['publish', 'INVALID_BUSINESS_TERMS'],
    ['publish', 'INVALID_BUSINESS_BUDGET'],
    ['publish', 'INSUFFICIENT_SIMULATION_CREDITS'],
    ['archive', 'CLOSE_BUSINESS_PROGRAM_FIRST'],
    ['archive', 'BUSINESS_HISTORY_MUST_BE_RETAINED'],
    ['publish', 'WORKSPACE_ARCHIVED'],
  ]) {
    const trace = beginApiTrace(`/api/organizations/${org}/programs/${program}/${action}?private=hidden`, 'POST');
    finishApiTrace(trace, 409, 'CONFLICT', reason);
    const result = getProcessSnapshot().at(-1);
    assert.equal(result.level, 'error');
    assert.equal(result.category, 'network');
    for (const text of [result.en, result.cs]) {
      assert.ok(text.includes(`CONFLICT · ${reason}`));
      assert.ok(text.includes(`/api/organizations/:id/programs/:id/${action}`));
      assert.ok(!text.includes(org));
      assert.ok(!text.includes(program));
      assert.ok(!text.includes('hidden'));
    }
  }
});
test('business error reasons must match exactly and never appear on successful requests', () => {
  clearProcess();
  for (const reason of [
    'Unknown backend explanation',
    'BUSINESS_PRIVATE_EMAIL',
    'private@example.com',
    'BUSINESS_REVIEW_PENDING\nprivate@example.com',
    'BUSINESS_REVIEW_PENDING private@example.com',
    'BUSINESS_REVIEW_PENDING_EXTRA',
  ]) {
    finishApiTrace(beginApiTrace('/api/organizations/:id/programs/:id/close', 'POST'), 409, 'CONFLICT', reason);
    const result = getProcessSnapshot().at(-1);
    for (const text of [result.en, result.cs]) {
      assert.ok(text.includes('CONFLICT'));
      assert.ok(!text.includes(reason));
      assert.ok(!text.includes('BUSINESS_REVIEW_PENDING'));
      assert.ok(!text.includes('private@example.com'));
    }
  }
  finishApiTrace(beginApiTrace('/api/organizations/:id/programs/:id/close', 'POST'), 200, undefined, 'BUSINESS_REVIEW_PENDING');
  const success = getProcessSnapshot().at(-1);
  assert.equal(success.level, 'success');
  assert.ok(!success.en.includes('BUSINESS_REVIEW_PENDING'));
  assert.ok(!success.cs.includes('BUSINESS_REVIEW_PENDING'));
  finishApiTrace(beginApiTrace('/api/organizations/:id/programs/:id/close', 'POST'), 409, 'BUSINESS_REVIEW_PENDING', 'BUSINESS_REVIEW_PENDING');
  assert.equal(getProcessSnapshot().at(-1).en.split('BUSINESS_REVIEW_PENDING').length, 2);
});
test('the process log is bounded, monotonic and clearable without persistence', () => {
  clearProcess();
  for (let i = 0; i < 250; i++) emitProcess({ category: 'app', level: 'info', en: 'x'.repeat(400), cs: 'y'.repeat(400) });
  const events = getProcessSnapshot();
  assert.equal(events.length, 200);
  assert.equal(events[0].en.length, 350);
  assert.ok(events.every((event, index) => !index || event.id > events[index - 1].id));
  clearProcess();
  assert.equal(getProcessSnapshot().length, 0);
});
test('cleared or signed-out logs discard completions from the previous session', () => {
  clearProcess();
  const previous = beginApiTrace('/api/prototype/challenges', 'GET');
  clearProcess();
  finishApiTrace(previous, 200);
  assert.equal(getProcessSnapshot().length, 0);
  finishApiTrace(beginApiTrace('/api/prototype/balance', 'GET'), 200);
  assert.equal(getProcessSnapshot().length, 2);
});
test('late observations from a signed-out component cannot enter a new session', () => {
  const previousScope = getProcessScope();
  resetProcessSession();
  emitProcess({ category: 'evidence', level: 'info', en: 'previous activity', cs: 'předchozí aktivita' }, previousScope);
  assert.equal(getProcessSnapshot().length, 0);
  emitProcess({ category: 'app', level: 'info', en: 'current session', cs: 'současné přihlášení' }, getProcessScope());
  clearProcess();
  emitProcess({ category: 'app', level: 'info', en: 'after manual clear', cs: 'po vymazání' }, getProcessScope());
  assert.equal(getProcessSnapshot().length, 1);
});
