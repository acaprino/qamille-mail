import assert from 'node:assert/strict';
import test from 'node:test';
import {
  canGiveUp,
  canUndo,
  confirmingUndoActions,
  hasOpenUndoWindow,
  isRetryNotice,
  retrySeconds,
  visibleUndoActions,
} from '../src/lib/undo-notices.ts';

const action = (id, overrides = {}) => ({
  id,
  operation: { kind: 'mail', email_id: id, action: 'markAsDone', value: null },
  subject: `Message ${id}`,
  created_at_ms: 0,
  expires_at_ms: 5000,
  status: 'pending',
  error: null,
  retry_at_ms: null,
  attempts: 0,
  ...overrides,
});
const ids = (actions) => actions.map((a) => a.id);
const send = (id, overrides = {}) =>
  action(id, { operation: { kind: 'send_draft', draft_id: id }, ...overrides });
/** A confirmation that never reached the provider: queued again with a backoff. */
const retrying = (id, overrides = {}) =>
  action(id, {
    attempts: 1,
    retry_at_ms: 20_000,
    error: 'IMAP connect: os error 10060',
    ...overrides,
  });

test('a pending action can be undone only inside its window', () => {
  assert.equal(canUndo(action(1), 4999), true);
  assert.equal(canUndo(action(1), 5000), false);
  assert.equal(canUndo(action(1, { status: 'committing' }), 0), false);
});

test('a mail notice stays while its undo window is open', () => {
  assert.deepEqual(ids(visibleUndoActions([action(1)], 4999)), [1]);
  assert.deepEqual(ids(confirmingUndoActions([action(1)], 4999)), []);
});

test('after the window, mail and draft actions leave the stack and are counted as confirming', () => {
  const actions = [
    action(1),
    action(2, { status: 'committing' }),
    action(3, { operation: { kind: 'discard_draft', draft_id: 3 } }),
    action(4, { operation: { kind: 'schedule_draft', draft_id: 4, send_at: 9 } }),
  ];
  assert.deepEqual(ids(visibleUndoActions(actions, 5000)), []);
  assert.deepEqual(ids(confirmingUndoActions(actions, 5000)), [1, 2, 3, 4]);
});

test('a failed action stays until dismissed and is not counted as confirming', () => {
  const failed = action(1, { status: 'failed', error: 'Network unavailable' });
  assert.deepEqual(ids(visibleUndoActions([failed], 60_000)), [1]);
  assert.deepEqual(ids(confirmingUndoActions([failed], 60_000)), []);
});

test('a send stays visible until delivery, whether waiting, sending or failed', () => {
  const actions = [send(1), send(2, { status: 'committing' }), send(3, { status: 'failed' })];
  assert.deepEqual(ids(visibleUndoActions(actions, 60_000)), [1, 2, 3]);
  assert.deepEqual(ids(confirmingUndoActions(actions, 60_000)), []);
});

test('the countdown ticks only while some action still has an open undo window', () => {
  assert.equal(hasOpenUndoWindow([], 0), false);
  assert.equal(hasOpenUndoWindow([action(1)], 4999), true);
  // Window closed: the action is only being confirmed, nothing on screen changes with time.
  assert.equal(hasOpenUndoWindow([action(1)], 5000), false);
  assert.equal(hasOpenUndoWindow([action(1, { status: 'committing' })], 0), false);
  // A failed notice and a send waiting for delivery are visible but static.
  assert.equal(hasOpenUndoWindow([action(1, { status: 'failed' })], 0), false);
  assert.equal(hasOpenUndoWindow([send(1), send(2, { status: 'committing' })], 60_000), false);
  // One open window among closed ones is enough.
  assert.equal(hasOpenUndoWindow([action(1, { status: 'committing' }), action(2)], 4999), true);
});

test('a retrying confirmation gets its own row instead of the generic counter', () => {
  assert.equal(isRetryNotice(retrying(1), 60_000), true);
  assert.deepEqual(ids(visibleUndoActions([retrying(1)], 60_000)), [1]);
  assert.deepEqual(ids(confirmingUndoActions([retrying(1)], 60_000)), []);
});

test('only a closed window with attempts behind it is a retry row', () => {
  // Inside its window the action is still cancellable, not retrying.
  assert.equal(isRetryNotice(retrying(1), 4999), false);
  // Closed but never attempted: it is being confirmed right now.
  assert.equal(isRetryNotice(action(1), 60_000), false);
  assert.deepEqual(ids(confirmingUndoActions([action(1)], 60_000)), [1]);
  // A retry attempt in flight keeps its row, so the reason stays on screen.
  assert.equal(isRetryNotice(retrying(1, { status: 'committing' }), 60_000), true);
  // A definitive failure keeps the red row: it is not a retry.
  assert.equal(isRetryNotice(retrying(1, { status: 'failed' }), 60_000), false);
  assert.deepEqual(ids(visibleUndoActions([retrying(1, { status: 'failed' })], 60_000)), [1]);
  assert.deepEqual(ids(confirmingUndoActions([retrying(1, { status: 'failed' })], 60_000)), []);
});

test('the retry countdown counts the seconds left before the next attempt', () => {
  assert.equal(retrySeconds(retrying(1), 19_999), 1);
  assert.equal(retrySeconds(retrying(1), 15_000), 5);
  assert.equal(retrySeconds(retrying(1), 20_000), 0);
  // The deadline passed: the attempt is due, nothing is left to count.
  assert.equal(retrySeconds(retrying(1), 60_000), 0);
  assert.equal(retrySeconds(action(1), 60_000), null);
  assert.equal(retrySeconds(retrying(1, { retry_at_ms: null }), 60_000), null);
});

test('the tick also runs while a retry row counts down, and stops when it is due', () => {
  assert.equal(hasOpenUndoWindow([retrying(1)], 19_999), true);
  assert.equal(hasOpenUndoWindow([retrying(1)], 20_000), false);
  assert.equal(hasOpenUndoWindow([retrying(1, { retry_at_ms: null })], 19_999), false);
  assert.equal(hasOpenUndoWindow([retrying(1), action(2, { status: 'failed' })], 60_000), false);
  assert.equal(hasOpenUndoWindow([retrying(1), action(2)], 4999), true);
});

test('giving up is offered on a retry row and on a real failure only', () => {
  assert.equal(canGiveUp(retrying(1), 60_000), true);
  assert.equal(canGiveUp(action(1, { status: 'failed' }), 60_000), true);
  // An open window is cancelled with Annulla, and a first attempt is still running.
  assert.equal(canGiveUp(action(1), 4999), false);
  assert.equal(canGiveUp(send(1), 60_000), false);
  assert.equal(canGiveUp(send(1, { attempts: 2, retry_at_ms: 90_000 }), 60_000), true);
});

test('a send waiting for its retry keeps one visible row', () => {
  const queued = send(1, { attempts: 3, retry_at_ms: 90_000, error: 'IMAP connect: os error 10060' });
  assert.deepEqual(ids(visibleUndoActions([queued], 60_000)), [1]);
  assert.deepEqual(ids(confirmingUndoActions([queued], 60_000)), []);
});
