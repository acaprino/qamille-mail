// The other messages of a conversation as a tree, flattened for the list (ADR-0010). The preview
// mirrors the Rust backend (app/src/domain/conversation.rs); these are the same cases as its tests.
import assert from 'node:assert/strict';
import test from 'node:test';
import { REPLY_CAP, conversationIds, replyRows } from '../src/lib/conversations.ts';

// A message as the provider stores it: a received reply carries its parent in `thread_id` too, a
// root carries its own Message-ID.
const mail = (id, folder, messageId, inReplyTo, date, extra = {}) => ({
  id, account_id: 1, folder, message_id: messageId, in_reply_to: inReplyTo,
  thread_id: inReplyTo || messageId, subject: 'Topic', from_name: `Sender ${id}`,
  from_addr: `sender${id}@x`, to: ['me@x'], cc: [], date, read: true, body: `Body ${id}`,
  ...extra,
});
// A reply Qamille sent: its thread_id is the root's, one level above its direct parent.
const sent = (id, messageId, inReplyTo, date) =>
  mail(id, 'Sent', messageId, inReplyTo, date, { thread_id: 'root@x' });
const chain = () => [
  mail(1, 'Inbox', 'root@x', '', 10),
  mail(2, 'Inbox', 'a@x', 'root@x', 20, { thread_id: 'root@x' }),
  sent(3, 's@x', 'a@x', 30),
  mail(4, 'Inbox', 'x@x', 's@x', 40),
];
const ids = (rows) => rows.map((row) => row.id);

test('a conversation lists every other message in tree order', () => {
  const rows = replyRows(chain(), 4);
  assert.deepEqual(ids(rows), [1, 2, 3]);
  assert.deepEqual(rows.map((row) => row.parent_id), [null, 1, 2]);
  assert.equal(rows[2].folder, 'Sent');
  assert.equal(rows[2].from_addr, 'sender3@x');
  assert.deepEqual(Object.keys(rows[0]).sort(), ['account_id', 'date', 'folder', 'from_addr', 'from_name', 'id', 'parent_id', 'read', 'snippet', 'to']);
});

test('Trash and Spam are not listed', () => {
  const emails = [
    ...chain(),
    mail(5, 'Trash', 't@x', 'root@x', 25, { thread_id: 'root@x' }),
    mail(6, 'Spam', 'p@x', 'root@x', 26, { thread_id: 'root@x' }),
  ];
  assert.deepEqual(ids(replyRows(emails, 4)), [1, 2, 3]);
});

test('an orphan hangs under the oldest message, or is a root when that one is not listed', () => {
  const orphan = mail(7, 'Sent', 'q@x', '', 15, { thread_id: 'root@x' });
  const rows = replyRows([mail(1, 'Inbox', 'root@x', '', 10), orphan, mail(4, 'Inbox', 'x@x', 'root@x', 40)], 4);
  assert.deepEqual(rows.map((row) => [row.id, row.parent_id]), [[1, null], [7, 1]]);
  const trashed = replyRows([mail(1, 'Trash', 'root@x', '', 10), orphan, mail(4, 'Inbox', 'x@x', 'root@x', 40)], 4);
  assert.deepEqual(trashed.map((row) => [row.id, row.parent_id]), [[7, null]]);
});

test('a reply cycle terminates and opens at its oldest message', () => {
  const rows = replyRows([
    mail(1, 'Inbox', 'a@x', 'b@x', 10),
    mail(2, 'Inbox', 'b@x', 'a@x', 20),
    mail(3, 'Inbox', 'x@x', 'a@x', 30),
  ], 3);
  assert.deepEqual(rows.map((row) => [row.id, row.parent_id]), [[1, null], [2, 1]]);
});

test('a copy of the main message or of a reply is never listed twice', () => {
  const emails = [
    ...chain(),
    mail(8, 'All', 'x@x', 's@x', 40, { thread_id: 's@x' }),
    mail(9, 'All', 'a@x', 'root@x', 20, { thread_id: 'root@x' }),
  ];
  const rows = replyRows(emails, 4);
  assert.deepEqual(ids(rows), [1, 2, 3]);
});

test('two accounts with the same Message-IDs do not join', () => {
  const other = chain().map((e) => ({ ...e, id: e.id + 100, account_id: 2 }));
  const emails = [...chain(), ...other];
  assert.deepEqual(ids(replyRows(emails, 4)), [1, 2, 3]);
  assert.deepEqual(ids(replyRows(emails, 104)), [101, 102, 103]);
});

test('the cap is 30 and the rest is what read_replies is for', () => {
  const emails = [mail(1, 'Inbox', 'root@x', '', 10)];
  for (let i = 0; i < 34; i++) emails.push(mail(10 + i, 'Inbox', `r${i}@x`, 'root@x', 20 + i, { thread_id: 'root@x' }));
  emails.push(mail(100, 'Inbox', 'x@x', 'root@x', 1000, { thread_id: 'root@x' }));
  assert.equal(REPLY_CAP, 30);
  assert.equal(replyRows(emails, 100).length, 35);
});

test('the conversation id is stable when newer mail arrives', () => {
  const before = conversationIds(chain());
  const after = conversationIds([...chain(), mail(200, 'Inbox', 'newer@x', 'x@x', 2000)]);
  assert.equal(before.get(4), before.get(1));
  assert.equal(after.get(200), before.get(4), 'the newest message joins the same conversation');
  assert.ok(before.get(4) > 0);
});

test('a message saved before the parent was stored gets it as the migration does', () => {
  // Received: the parent is what thread_id says; sent: unknown, so it hangs under the oldest.
  const legacy = [
    { ...mail(1, 'Inbox', 'root@x', '', 10), in_reply_to: undefined },
    { ...mail(2, 'Inbox', 'a@x', '', 20, { thread_id: 'root@x' }), in_reply_to: undefined },
    { ...mail(3, 'Sent', 's@x', '', 30, { thread_id: 'root@x' }), in_reply_to: undefined },
    { ...mail(4, 'Inbox', 'x@x', '', 40, { thread_id: 'a@x' }), in_reply_to: undefined },
  ];
  const rows = replyRows(legacy, 4);
  assert.deepEqual(rows.map((row) => [row.id, row.parent_id]), [[1, null], [2, 1], [3, 1]]);
});

test('an empty stored parent counts as not stored, the way the backend reads it', () => {
  // The backend's field is a String: a store written before it existed holds "" for every message.
  const legacy = [
    mail(1, 'Inbox', 'root@x', '', 10),
    mail(2, 'Inbox', 'a@x', '', 20, { thread_id: 'root@x' }),
    mail(3, 'Inbox', 'b@x', '', 30, { thread_id: 'a@x' }),
    mail(4, 'Inbox', 'x@x', '', 40, { thread_id: 'b@x' }),
  ];
  const rows = replyRows(legacy, 4);
  assert.deepEqual(rows.map((row) => [row.id, row.parent_id]), [[1, null], [2, 1], [3, 2]]);
});

test('the Trash and Spam views hold their own members; the live views still hide both', () => {
  const emails = [
    mail(1, 'Trash', 'root@x', '', 10),
    mail(2, 'Trash', 'a@x', 'root@x', 20),
    mail(3, 'Inbox', 'i@x', 'a@x', 30),
    mail(4, 'Spam', 'p@x', 'a@x', 35),
    mail(5, 'Trash', 'x@x', 'a@x', 40),
  ];
  assert.deepEqual(ids(replyRows(emails, 5, 'Trash')), [1, 2, 3], 'Trash view: its members and the live one, not Spam');
  assert.deepEqual(ids(replyRows(emails, 5, 'trash')), [1, 2, 3], 'the folder is matched without regard to case, like the list');
  assert.deepEqual(ids(replyRows(emails, 4, 'Spam')), [3], 'Spam view (its main row is the Spam message): the live member, not Trash');
  assert.deepEqual(ids(replyRows(emails, 3, 'Inbox')), [], 'Inbox view: nothing else of it is live');
  assert.deepEqual(ids(replyRows(emails, 3)), [], 'no view named: both hidden');
});
