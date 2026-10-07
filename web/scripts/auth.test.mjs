import { afterEach, test } from 'node:test';
import assert from 'node:assert/strict';
import { ApiError, api, isAuthRetryBlocked, logoutSession, setCsrf } from '../src/api.ts';

const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
  setCsrf('');
});

test('logout completes when the current session has already been revoked', async () => {
  setCsrf('revoked-session-token');
  let requests = 0;
  globalThis.fetch = async (path, options) => {
    requests++;
    assert.equal(path, '/api/auth/logout');
    assert.equal(options.method, 'POST');
    assert.equal(options.credentials, 'same-origin');
    assert.equal(options.headers['X-CSRF-Token'], 'revoked-session-token');
    return Response.json({ code: 'UNAUTHORIZED', error: 'Přihlaste se prosím znovu.' }, { status: 401 });
  };
  await logoutSession();
  assert.equal(requests, 1);
});

test('logout preserves verification and server failures for recovery without retrying', async () => {
  for (const [status, code] of [[403, 'FORBIDDEN'], [500, 'INTERNAL'], [401, 'INVALID_CREDENTIALS']]) {
    let requests = 0;
    globalThis.fetch = async () => {
      requests++;
      return Response.json({ code, error: 'request failed' }, { status });
    };
    await assert.rejects(logoutSession, error => error instanceof ApiError && error.status === status && error.code === code);
    assert.equal(requests, 1);
  }
});

test('logout keeps a connection failure visible because server revocation is unknown', async () => {
  globalThis.fetch = async () => { throw new TypeError('offline'); };
  await assert.rejects(logoutSession, error => error instanceof ApiError && error.code === 'NETWORK');
});

test('rate-limit errors carry the server recovery delay without an automatic retry', async () => {
  let requests = 0;
  globalThis.fetch = async () => {
    requests++;
    return Response.json({ code: 'RATE_LIMITED', error: 'too many attempts' }, { status: 429, headers: { 'Retry-After': '237' } });
  };
  await assert.rejects(api('/api/auth/login', { method: 'POST', body: {} }), error =>
    error instanceof ApiError && error.status === 429 && error.code === 'RATE_LIMITED' && error.retryAfterSeconds === 237);
  assert.equal(requests, 1);
});

test('invalid and unrelated retry headers never lock the authentication form', async () => {
  for (const [status, header] of [[429, '-1'], [429, '0'], [429, '99999999999999'], [429, 'invalid'], [503, '120']]) {
    globalThis.fetch = async () => Response.json({ code: 'REQUEST_FAILED' }, { status, headers: { 'Retry-After': header } });
    await assert.rejects(api('/api/auth/login', { method: 'POST', body: {} }), error =>
      error instanceof ApiError && error.retryAfterSeconds === null);
  }
});

test('an email cooldown allows another account, survives switching back and expires', () => {
  for (const action of ['login', 'forgot']) {
    const cooldown = { action, email: 'First@Example.test', until: 15_000 };
    assert.equal(isAuthRetryBlocked(cooldown, action, 'first@example.test', 1_000), true);
    assert.equal(isAuthRetryBlocked(cooldown, action, 'second@example.test', 1_000), false);
    assert.equal(isAuthRetryBlocked(cooldown, action, ' FIRST@example.test ', 2_000), true);
    assert.equal(isAuthRetryBlocked(cooldown, 'register', 'first@example.test', 2_000), false);
    assert.equal(isAuthRetryBlocked(cooldown, action, 'first@example.test', 15_000), false);
  }
});

test('the registration IP cooldown remains active when the email changes', () => {
  const cooldown = { action: 'register', email: 'first@example.test', until: 15_000 };
  assert.equal(isAuthRetryBlocked(cooldown, 'register', 'second@example.test', 1_000), true);
  assert.equal(isAuthRetryBlocked(cooldown, 'login', 'second@example.test', 1_000), false);
});
