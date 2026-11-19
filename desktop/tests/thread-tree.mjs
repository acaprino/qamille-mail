// What the list shows of a conversation (spec 2026-11-07 §4.3): the replies left once some are
// folded, with what each folded one hides, and who each reply answers.
import assert from 'node:assert/strict';
import test from 'node:test';
import { personIndex } from '../src/lib/people.ts';
import { answerTarget, visibleReplies } from '../src/lib/thread-tree.ts';

const reply = (id, parent, extra = {}) => ({
  id, account_id: 1, parent_id: parent, from_name: `Sender ${id}`, from_addr: `s${id}@x`,
  to: ['me@x'], snippet: '', date: id, read: true, folder: 'Inbox', ...extra,
});
//   1
//   └ 2
//     ├ 3
//     └ 4 (unread)
//   5
const tree = () => [reply(1, null), reply(2, 1), reply(3, 2), reply(4, 2, { read: false }), reply(5, null)];
const ids = (rows) => rows.map((row) => row.reply.id);

test('with nothing folded every reply shows and only parents have children', () => {
  const rows = visibleReplies(tree(), []);
  assert.deepEqual(ids(rows), [1, 2, 3, 4, 5]);
  assert.deepEqual(rows.map((row) => row.hasChildren), [true, true, false, false, false]);
  assert.ok(rows.every((row) => !row.folded && row.hidden === 0 && !row.unreadHidden));
});

test('a folded reply hides everything under it and counts it', () => {
  const rows = visibleReplies(tree(), [2]);
  assert.deepEqual(ids(rows), [1, 2, 5]);
  const folded = rows.find((row) => row.reply.id === 2);
  assert.equal(folded.folded, true);
  assert.equal(folded.hidden, 2);
  assert.equal(folded.unreadHidden, true, 'an unread message is among the hidden ones');
});

test('the blue mark needs an unread message among the hidden ones', () => {
  const allRead = tree().map((r) => ({ ...r, read: true }));
  const rows = visibleReplies(allRead, [2]);
  assert.equal(rows.find((row) => row.reply.id === 2).unreadHidden, false);
});

test('folding the first reply hides the whole branch, and its descendants count', () => {
  const rows = visibleReplies(tree(), [1]);
  assert.deepEqual(ids(rows), [1, 5]);
  assert.equal(rows[0].hidden, 3);
  assert.equal(rows[0].unreadHidden, true);
});

test('a folded reply nobody answered hides nothing and a stale fold is ignored', () => {
  assert.deepEqual(ids(visibleReplies(tree(), [3, 999])), [1, 2, 3, 4, 5]);
  assert.equal(visibleReplies(tree(), [3]).find((row) => row.reply.id === 3).hidden, 0);
});

test('a reply whose parent is not loaded yet stays on screen', () => {
  const rows = visibleReplies([reply(10, 999), reply(11, 10)], [999]);
  assert.deepEqual(ids(rows), [10, 11]);
});

test('a parent chain that loops cannot hang the list', () => {
  const rows = visibleReplies([reply(1, 2), reply(2, 1)], [1]);
  assert.equal(rows.length >= 1, true);
});

// ---- who a reply answers ----
const index = personIndex([
  { name: 'Luca Bianchi', address: 'luca@x' },
  { name: 'Maria Rossi', address: 'maria@x' },
]);
const self = new Set(['me@x']);
const target = (r, replies, mainAuthor = 'maria@x') => answerTarget(r, replies, mainAuthor, index, self, 'you');

test('a reply names the author of the message it answers when they are among its recipients', () => {
  const parent = reply(1, null, { from_addr: 'luca@x' });
  const answer = reply(2, 1, { to: ['me@x', 'maria@x', 'luca@x', 'g1@x', 'g2@x'] });
  const found = target(answer, [parent, answer]);
  assert.equal(found.name, 'Luca Bianchi');
  assert.equal(found.you, false);
  assert.equal(found.more, 4, 'the other four recipients of To, never Cc');
});

test('otherwise it names the first recipient, and the reader is "you"', () => {
  const parent = reply(1, null, { from_addr: 'stranger@x' });
  const answer = reply(2, 1, { to: ['me@x', 'maria@x'] });
  const found = target(answer, [parent, answer]);
  assert.equal(found.name, 'you');
  assert.equal(found.you, true);
  assert.equal(found.more, 1);
});

test('a reply to the main message is read against the main message author', () => {
  const answer = reply(2, null, { to: ['luca@x', 'maria@x', 'me@x'] });
  const found = target(answer, [answer], 'maria@x');
  assert.equal(found.name, 'Maria Rossi');
  assert.equal(found.more, 2);
});

test('the reader among the recipients is "you" when they are the one answered', () => {
  const parent = reply(1, null, { from_addr: 'me@x' });
  const answer = reply(2, 1, { to: ['luca@x', 'ME@x'] });
  const found = target(answer, [parent, answer]);
  assert.equal(found.name, 'you');
  assert.equal(found.you, true);
});

test('a repeated address is counted once and nobody to name is no target', () => {
  const answer = reply(2, null, { to: ['luca@x', 'Luca <luca@x>', 'maria@x'] });
  assert.equal(target(answer, [answer], 'zed@x').more, 1);
  assert.equal(target(reply(3, null, { to: [] }), []), null);
});

test('the tooltip lists up to twelve names and counts the rest', () => {
  const to = Array.from({ length: 20 }, (_, i) => `p${i + 1}@x`);
  const found = target(reply(2, null, { to }), [], 'p1@x');
  assert.equal(found.names.length, 12);
  assert.equal(found.rest, 8);
  assert.equal(found.names[0], 'p1@x');
});
