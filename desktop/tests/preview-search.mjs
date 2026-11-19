import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';
registerHooks({
  resolve(specifier, context, nextResolve) {
    return nextResolve(specifier === './regional' ? './regional.ts' : specifier, context);
  },
});
const { sampleSearch } = await import('../src/lib/preview-search.ts');

const email = (id, values = {}) => ({
  id,
  account_id: 1,
  folder: 'Inbox',
  from_name: 'Maria Rossi',
  from_addr: 'maria@example.com',
  to: ['me@work.example'],
  cc: [],
  subject: 'Contratto firmato',
  body: 'Codice ORD-4821 ricevuto',
  date: Date.parse('2026-10-15T22:30:00Z') / 1000,
  read: false,
  date_missing: false,
  attachments: [],
  labels: [],
  category: 'Personal',
  ...values,
});
const store = (emails) => ({
  emails,
  accounts: [
    { id: 1, name: 'Work', email: 'me@work.example' },
    { id: 2, name: 'Home', email: 'me@home.example' },
  ],
  settings: { time_zone: 'Europe/Rome', locale: 'it-IT' },
});
const ids = (data, query, account = null) =>
  sampleSearch(data, query, account).emails.map((email) => email.id);

test('sample parser preserves exact addresses, phrases and OR/AND precedence', () => {
  const data = store([
    email(1),
    email(2, { from_addr: 'other.maria@example.com', subject: 'Altro' }),
    email(3, { from_addr: 'luca@example.com', read: true }),
  ]);
  assert.deepEqual(ids(data, 'from:maria@example.com'), [1]);
  assert.deepEqual(ids(data, 'from:@example.com subject:"contratto firmato"'), [3, 1]);
  assert.deepEqual(ids(data, '(from:maria@example.com OR from:luca@example.com) is:unread'), [1]);
  assert.deepEqual(ids(data, '"ORD-4821" -from:luca@example.com'), [2, 1]);
});

test('sample dates use configured local days, bounded around and inclusive between', () => {
  const data = store([
    email(1),
    email(2, { date: Date.parse('2026-10-09T10:00:00Z') / 1000 }),
    email(3, { date: Date.parse('2026-10-22T10:00:00Z') / 1000 }),
  ]);
  assert.deepEqual(ids(data, 'on:2026-10-16'), [1]);
  assert.deepEqual(ids(data, 'around:2026-10-15/6d'), [1, 2]);
  assert.deepEqual(ids(data, 'between:09/10/2026..16/10/2026'), [1, 2]);
  for (const query of [
    'around:2026-10-15/400d',
    'around:2026-10-15/2w',
    'newer_than:7è',
    'from:',
    'is:unknown',
    'on:31/02/2026',
    'after:2026-10-22 before:2026-10-09',
  ])
    assert.throws(() => ids(data, query));
});

test('sample default scope excludes Spam and Trash with branch-specific explicit inclusion', () => {
  const data = store([
    email(1),
    email(2, { folder: 'Trash' }),
    email(3, { folder: 'Spam', from_addr: 'luca@example.com' }),
  ]);
  assert.deepEqual(ids(data, 'contratto'), [1]);
  assert.deepEqual(ids(data, 'in:trash'), [2]);
  assert.deepEqual(ids(data, 'in:anywhere contratto'), [3, 2, 1]);
  assert.deepEqual(ids(data, '(in:trash from:luca@example.com) OR from:maria@example.com'), [1]);
});

test('sample account filtering and pagination source never truncate matching mail', () => {
  const data = store(
    Array.from({ length: 45 }, (_, i) => email(i + 1, { account_id: i < 30 ? 1 : 2 })),
  );
  assert.equal(ids(data, 'contratto').length, 45);
  assert.equal(ids(data, 'contratto', 2).length, 15);
  assert.equal(ids(data, 'to:me', 2).length, 0);
});
