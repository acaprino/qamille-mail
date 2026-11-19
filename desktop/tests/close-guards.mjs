// The order of the guards a window close goes through (src/lib/close-guards.ts). Synthetic guards.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createCloseGuards } from '../src/lib/close-guards.ts';

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
