// «Esci» with a send still undoable (audit Q8): the undo queue's close guard and its session-end step
// (src/lib/undo-close.ts), driven with a fake `call`. Run: node --test tests/undo-close.mjs (from desktop/).
// Node strips the type annotations of this dependency-free module.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createUndoQueueGuard, createUndoQueueSessionEnd, QUIT_WAIT_MS, SESSION_END_WAIT_MS } from '../src/lib/undo-close.ts';

/** A fake backend: `answers` maps a command to a function of its arguments. Every call is logged. */
function backend(answers) {
  const log = [];
  const call = async (command, args = {}) => {
    log.push({ command, args });
    const answer = answers[command];
    if (!answer) throw new Error(`unexpected command ${command}`);
    return answer(args);
  };
  return { call, log, commands: () => log.map((entry) => entry.command) };
}

test('nothing queued closes at once', async () => {
  const fake = backend({ undo_send_now: () => ({ remaining: 0 }) });
  assert.equal(await createUndoQueueGuard(fake.call)(), true);
  // No window brought back, no question: the close goes on.
  assert.deepEqual(fake.commands(), ['undo_send_now']);
});

test('queued sends are sent before closing', async () => {
  let sent;
  const leaving = new Promise((resolve) => (sent = resolve));
  const fake = backend({ undo_send_now: () => leaving });
  let answered = false;
  const closing = createUndoQueueGuard(fake.call)().then((value) => ((answered = true), value));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(answered, false, 'the close waits for the queue to leave');
  sent({ remaining: 0 });
  assert.equal(await closing, true);
  const [{ args }] = fake.log;
  assert.equal(args.waitMs, QUIT_WAIT_MS);
  assert.ok(QUIT_WAIT_MS > 0 && QUIT_WAIT_MS <= 30_000, 'a bounded wait');
});

test('a send that did not leave asks, and keeping Qamille open vetoes', async () => {
  for (const [keepOpen, closes] of [[true, false], [false, true]]) {
    const fake = backend({
      undo_send_now: () => ({ remaining: 1 }),
      restore_main_window: () => null,
      undo_quit_anyway: () => !keepOpen,
    });
    assert.equal(await createUndoQueueGuard(fake.call)(), closes);
    // The window comes back first (Esci from the tray: it is hidden), so the question is seen with it.
    assert.deepEqual(fake.commands(), ['undo_send_now', 'restore_main_window', 'undo_quit_anyway']);
    assert.deepEqual(fake.log[2].args, { remaining: 1 });
  }
});

test('session end sends only when the session really ends', async () => {
  const fake = backend({ undo_send_now: () => ({ remaining: 0 }) });
  const step = createUndoQueueSessionEnd(fake.call);
  // Windows only asks: another app can still cancel, so nothing irreversible happens.
  assert.notEqual(await step(false), false);
  assert.deepEqual(fake.commands(), []);
  // It ends: there is nobody to ask, and Windows waits about 8 s in all.
  await step(true);
  assert.deepEqual(fake.commands(), ['undo_send_now']);
  assert.ok(fake.log[0].args.waitMs <= 5000 && fake.log[0].args.waitMs === SESSION_END_WAIT_MS);
});
