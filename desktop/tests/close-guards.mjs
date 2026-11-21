// The order of the guards a window close goes through (src/lib/close-guards.ts). Synthetic guards.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createCloseGuards } from '../src/lib/close-guards.ts';
import { createDirtyGuard } from '../src/lib/dirty-guard.ts';

// The meetings guard stops the recording and cancels the jobs: it may do that only when every
// other guard has let the window close.
function rig({ settingsVeto }) {
  const log = [];
  const guards = createCloseGuards();
  guards.register(
    'meetings',
    async () => {
      log.push('meeting_stop', 'meeting_cancel_all_jobs');
      return true;
    },
    { last: true },
  );
  guards.register('composers', async () => (log.push('composers'), true));
  guards.register('settings', async () => (log.push('settings'), !settingsVeto));
  return { guards, log };
}

test('a_vetoed_close_does_not_stop_the_recording_or_cancel_the_jobs', async () => {
  const { guards, log } = rig({ settingsVeto: true });
  assert.equal(await guards.run(), false);
  assert.deepEqual(log, ['composers', 'settings'], 'nothing irreversible happened');
});

test('a_close_nobody_vetoes_runs_the_meetings_guard_last', async () => {
  const { guards, log } = rig({ settingsVeto: false });
  assert.equal(await guards.run(), true);
  assert.deepEqual(log, ['composers', 'settings', 'meeting_stop', 'meeting_cancel_all_jobs']);
});

test('the_meetings_guard_can_veto_too_and_a_removed_guard_is_not_run', async () => {
  const guards = createCloseGuards();
  const log = [];
  guards.register('a', async () => (log.push('a'), true));
  const off = guards.register('b', async () => (log.push('b'), false));
  guards.register('late', async () => (log.push('late'), false), { last: true });
  assert.equal(await guards.run(), false);
  assert.deepEqual(log, ['a', 'b'], 'the first veto stops the rest');
  off();
  log.length = 0;
  assert.equal(await guards.run(), false);
  assert.deepEqual(log, ['a', 'late']);
});

// An editor with unsaved typing (the calendar event sheet, the contact form) registers a guard while
// it is dirty (src/lib/dirty-guard.ts): it vetoes the close and asks, and at session end it saves.
function editor({ dirty, saveNow = async () => true }) {
  const guards = createCloseGuards();
  const calls = { veto: 0, save: 0 };
  const release = createDirtyGuard(guards.register)(
    'editor',
    dirty,
    () => calls.veto++,
    async () => (calls.save++, saveNow()),
  );
  return { guards, calls, release };
}

test('a dirty editor vetoes the close and asks', async () => {
  const { guards, calls } = editor({ dirty: true });
  assert.equal(await guards.run(), false);
  assert.equal(calls.veto, 1);
  assert.equal(calls.save, 0, 'the close only asks: nothing is saved without the person asking');
});

test('a clean editor leaves the close alone', async () => {
  const { guards, calls } = editor({ dirty: false });
  assert.equal(await guards.run(), true);
  assert.equal(calls.veto, 0);
  assert.equal(await guards.endSession(true), true);
  assert.equal(calls.save, 0, 'nothing to save');
});

test('an editor that turns clean or goes away releases its guard', async () => {
  const { guards, calls, release } = editor({ dirty: true });
  assert.equal(await guards.run(), false);
  release();
  assert.equal(await guards.run(), true);
  assert.equal(await guards.endSession(false), true);
  assert.deepEqual(calls, { veto: 1, save: 0 });
});

test('at session end a dirty editor saves, and an invalid form answers false', async () => {
  const valid = editor({ dirty: true });
  assert.equal(await valid.guards.endSession(true), true);
  assert.equal(valid.calls.save, 1);
  assert.equal(valid.calls.veto, 0, 'no dialog at session end');

  const invalid = editor({ dirty: true, saveNow: async () => false });
  assert.equal(await invalid.guards.endSession(true), false, 'not saved: the reload asks');
  assert.equal(invalid.calls.save, 1);
  assert.equal(invalid.calls.veto, 0);

  const failing = editor({
    dirty: true,
    saveNow: async () => {
      throw new Error('disk full');
    },
  });
  assert.equal(await failing.guards.endSession(true), false, 'a throwing save does not escape');
});
