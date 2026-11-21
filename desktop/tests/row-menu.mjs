// The entries of the native menu of the mail rows (src/lib/row-menu.ts, docs/aspetto-nativo.md
// rule 33): what one row offers, what several checked rows offer, what a locked row still offers,
// and that the words are all of one language. Rust only draws the entries; they are decided here.
import assert from 'node:assert/strict';
import test from 'node:test';
import { rowMenuAction, rowMenuEntries } from '../src/lib/row-menu.ts';

const italian = (it) => it;
const english = (_it, en) => en;
const row = (overrides = {}) => ({
  id: 1,
  message_ids: [1],
  account_id: 1,
  from_name: 'Maria',
  from_addr: 'maria@example.test',
  subject: 'Budget',
  snippet: '',
  date: 0,
  read: true,
  pinned: false,
  done: false,
  category: 'Personal',
  section: 'Seen',
  labels: [],
  attachment_count: 0,
  due: null,
  folder: 'Inbox',
  ...overrides,
});
const values = (entries) => entries.map((entry) => entry.value);
const chosen = (entries) => entries.filter((entry) => entry.value !== null);

test('a single row offers reply, triage, read state, pin and inbox', () => {
  const inbox = rowMenuEntries([row()], 'Inbox', false, english);
  assert.deepEqual(values(inbox), [
    'reply', 'reply-all', 'forward', null,
    'archive', 'moveToTrash', 'markAsDone', 'snooze', null,
    'markAsUnseen', null,
    'pin',
  ]);
  assert.deepEqual(chosen(inbox).map((entry) => entry.label), [
    'Reply', 'Reply all', 'Forward',
    'Archive', 'Move to trash', 'Done', 'Snooze until tomorrow',
    'Mark as unread',
    'Pin',
  ]);
  assert.ok(chosen(inbox).every((entry) => entry.enabled), 'nothing is locked');
  for (const separator of inbox.filter((entry) => entry.value === null)) assert.equal(separator.enabled, false);

  // An unread, pinned row out of the Inbox: the other read state, Unpin, and back to the Inbox.
  const archived = rowMenuEntries([row({ read: false, pinned: true, folder: 'Archive' })], 'Archive', false, english);
  assert.deepEqual(values(archived).slice(-5), ['markAsSeen', null, 'unpin', null, 'moveToInbox']);
  assert.equal(archived.find((entry) => entry.value === 'moveToInbox').label, 'Move to inbox');
  // A view that is not a folder (Tasks) holds Inbox mail too: nothing to move back.
  assert.ok(!values(rowMenuEntries([row()], 'Tasks', false, english)).includes('moveToInbox'));

  // What each entry does: the same actions as the row and the reader.
  assert.deepEqual(rowMenuAction('archive'), { action: 'archive' });
  assert.deepEqual(rowMenuAction('snooze'), { action: 'snooze', value: 'Tomorrow' });
  assert.deepEqual(rowMenuAction('markAsUnseen'), { action: 'markAsUnseen' });
  assert.equal(rowMenuAction('reply'), null, 'a reply opens the composer, it is not an action');
  assert.equal(rowMenuAction('nonsense'), null);
});

test('several checked rows offer only group actions', () => {
  const entries = rowMenuEntries([row({ id: 1 }), row({ id: 2, read: false })], 'Inbox', false, english);
  const offered = values(entries);
  for (const single of ['reply', 'reply-all', 'forward']) assert.ok(!offered.includes(single), `${single} is for one message`);
  assert.equal(offered[0], 'archive', 'the menu does not start with a separator');
  // Read and unread rows together: both read states.
  assert.ok(offered.includes('markAsSeen') && offered.includes('markAsUnseen'));
  // Pin until every row is pinned.
  assert.ok(offered.includes('pin') && !offered.includes('unpin'));
  const pinned = values(rowMenuEntries([row({ id: 1, pinned: true }), row({ id: 2, pinned: true })], 'Inbox', false, english));
  assert.ok(pinned.includes('unpin') && !pinned.includes('pin'));
  assert.deepEqual(rowMenuEntries([], 'Inbox', false, english), [], 'no rows, no menu');
});

test('a locked section disables what would change the message', () => {
  const entries = chosen(rowMenuEntries([row({ folder: 'Archive' })], 'Archive', true, english));
  const enabled = entries.filter((entry) => entry.enabled).map((entry) => entry.value);
  assert.deepEqual(enabled, ['reply', 'reply-all', 'forward'], 'answering changes nothing in the message');
  assert.ok(entries.some((entry) => entry.value === 'moveToInbox' && !entry.enabled), 'locked entries stay in sight, greyed');
});

test('labels are Italian or English, never mixed', () => {
  const cases = [
    [[row()], 'Inbox'],
    [[row({ read: false, pinned: true, folder: 'Archive' })], 'Archive'],
    [[row({ id: 1 }), row({ id: 2, read: false })], 'Inbox'],
  ];
  const words = (tr) =>
    cases.flatMap(([rows, view]) => chosen(rowMenuEntries(rows, view, false, tr)).map((entry) => entry.label));
  // Every label goes through the language helper: none is written in one language only.
  const tagged = words((it, en) => `«${it}|${en}»`);
  assert.ok(tagged.every((label) => /^«[^|]+\|[^»]+»$/.test(label)), `a label skips tr(): ${tagged.join(', ')}`);
  const it = new Set(words(italian));
  const en = new Set(words(english));
  assert.deepEqual([...it].filter((label) => en.has(label)), [], 'an Italian label is also an English one');
  assert.ok(it.has('Rispondi a tutti') && it.has('Sposta in Posta in arrivo') && it.has('Segna come letto'));
  assert.ok(en.has('Reply all') && en.has('Move to inbox') && en.has('Mark as read'));
});
