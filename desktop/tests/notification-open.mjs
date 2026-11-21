// A click on a Windows notification of Qamille (phase 3, Task 3; docs/aspetto-nativo.md, rule 28):
// Rust brings the window back and sends `notification-open`; the page opens what the notification
// was about. Run: node --test tests/notification-open.mjs (from desktop/). Node strips the type
// annotations of this dependency-free module.
import assert from 'node:assert/strict';
import test from 'node:test';
import { parseNotificationOpen, routeNotificationOpen } from '../src/lib/notification-open.ts';

function recorder() {
  const log = [];
  return {
    log,
    actions: {
      openMail: (id) => log.push(['mail', id]),
      openDrafts: () => log.push(['drafts']),
      openAccounts: () => log.push(['accounts']),
      openServer: () => log.push(['server']),
    },
  };
}

test('a new-mail notification opens its message', () => {
  const { log, actions } = recorder();
  assert.equal(routeNotificationOpen({ kind: 'mail', email_id: 42, account_id: 1 }, actions), true);
  assert.deepEqual(log, [['mail', 42]]);
});

test('a failed scheduled send opens Drafts', () => {
  const { log, actions } = recorder();
  assert.equal(routeNotificationOpen({ kind: 'scheduled' }, actions), true);
  assert.deepEqual(log, [['drafts']]);
});

test('an expired sign-in opens the accounts, or the server for the server', () => {
  const { log, actions } = recorder();
  routeNotificationOpen({ kind: 'sign-in', account_id: 3 }, actions);
  routeNotificationOpen({ kind: 'sign-in' }, actions);
  assert.deepEqual(log, [['accounts'], ['server']]);
  // Without a server action the server's sign-in still lands in Settings.
  const plain = [];
  routeNotificationOpen({ kind: 'sign-in' }, { openMail: () => plain.push('mail'), openDrafts: () => plain.push('drafts'), openAccounts: () => plain.push('accounts') });
  assert.deepEqual(plain, ['accounts']);
});

test('anything else is ignored', () => {
  const { log, actions } = recorder();
  for (const payload of [
    null,
    'mail',
    {},
    { kind: 'mail' },
    { kind: 'mail', email_id: -1 },
    { kind: 'mail', email_id: 1.5 },
    { kind: 'mail', email_id: '7' },
    { kind: 'sign-in', account_id: 'x' },
    { kind: 'calendar' },
  ]) {
    assert.equal(routeNotificationOpen(payload, actions), false, JSON.stringify(payload));
  }
  assert.deepEqual(log, []);
  assert.deepEqual(parseNotificationOpen({ kind: 'sign-in', account_id: 2, extra: 1 }), { kind: 'sign-in', account_id: 2 });
  assert.deepEqual(parseNotificationOpen({ kind: 'sign-in', account_id: null }), { kind: 'sign-in' });
});
