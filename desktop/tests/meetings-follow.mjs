// Task 14 (fix round): the follower of the meetings in flight (src/lib/meetings-follow.ts) and the
// helpers it relies on. Synthetic data only; the timer is node's mock timer, so no test waits.
import assert from 'node:assert/strict';
import test, { afterEach, beforeEach, mock } from 'node:test';
import {
  FOLLOW_FAILURES,
  FOLLOW_MS,
  MeetingFollower,
  SETTLE_TICKS,
} from '../src/lib/meetings-follow.ts';

const job = (id, kind, state) => ({
  id,
  kind,
  state,
  input_revision: 1,
  progress: 0,
  error: null,
  created_at: 1,
});
const meeting = (id, jobs = []) => ({
  meeting: { id, title: `Riunione ${id}`, date: 1, duration_secs: 60, event_id: null },
  capture: { meeting_id: id, state: 'saved' },
  jobs,
  deleted: false,
});
const running = (id) => meeting(id, [job('t', 'transcribe', 'running')]);
const transcribed = (id) => meeting(id, [job('t', 'transcribe', 'completed')]);
const summarizing = (id) =>
  meeting(id, [job('t', 'transcribe', 'completed'), job('s', 'summarize', 'running')]);
const summarized = (id) =>
  meeting(id, [job('t', 'transcribe', 'completed'), job('s', 'summarize', 'completed')]);
const failed = (id) => meeting(id, [job('t', 'transcribe', 'failed')]);

/** A follower on a fake backend: `answers[id]` is what the next reads of that meeting return. */
function rig(over = {}) {
  const log = { reads: [], applied: [], reloads: 0, giveUps: [] };
  const answers = {};
  let stamp = 0;
  let hidden = false;
  const follower = new MeetingFollower({
    read: async (id) => {
      log.reads.push(id);
      const answer = answers[id];
      if (answer instanceof Error) throw answer;
      return typeof answer === 'function' ? answer() : answer;
    },
    apply: (detail) => log.applied.push(detail.meeting.id),
    reload: async () => {
      log.reloads++;
    },
    stamp: () => stamp,
    giveUp: (error) => log.giveUps.push(error),
    hidden: () => hidden,
    ...over,
  });
  return {
    follower,
    log,
    answers,
    bump: () => stamp++,
    hide: (value) => (hidden = value),
  };
}

beforeEach(() => mock.timers.enable({ apis: ['setInterval'] }));
afterEach(() => mock.timers.reset());

test('the_timer_runs_only_while_something_is_followed', () => {
  const { follower } = rig();
  assert.equal(follower.running, false, 'a page at rest has no timer');
  follower.sync([]);
  assert.equal(follower.running, false);
  follower.sync([4]);
  assert.equal(follower.running, true);
  follower.sync([4, 9]);
  assert.equal(follower.running, true);
  follower.sync([]);
  assert.equal(follower.running, false, 'nothing in flight, nothing running');
  follower.dispose();
});

test('a_tick_reads_the_meetings_in_flight_and_only_them', async () => {
  const { follower, log, answers } = rig();
  answers[3] = running(3);
  answers[8] = running(8);
  follower.sync([8, 3]);
  await follower.tick();
  assert.deepEqual(log.reads, [3, 8]);
  assert.deepEqual(log.applied, [3, 8]);
  assert.equal(log.reloads, 0, 'the list is not read while the jobs go on');
  assert.equal(follower.running, true);
  follower.dispose();
});

test('the_interval_is_one_second', async () => {
  const { follower, log, answers } = rig();
  answers[3] = running(3);
  follower.sync([3]);
  mock.timers.tick(FOLLOW_MS - 1);
  await Promise.resolve();
  assert.deepEqual(log.reads, []);
  mock.timers.tick(1);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(log.reads, [3]);
  follower.dispose();
});

test('a_meeting_with_no_job_and_nothing_to_wait_for_reads_the_list_once_and_is_let_go', async () => {
  const { follower, log, answers } = rig();
  answers[3] = summarized(3);
  answers[4] = failed(4);
  follower.sync([3, 4]);
  await follower.tick();
  assert.equal(log.reloads, 1);
  assert.equal(follower.running, false, 'it stops by itself when nothing is in flight');
  assert.deepEqual(follower.following, []);
  await follower.tick();
  assert.deepEqual(log.reads, [3, 4], 'nothing is read after that');
  follower.dispose();
});

test('a_transcript_without_its_summary_is_followed_for_a_grace_then_the_list_is_read_once', async () => {
  const { follower, log, answers } = rig();
  answers[5] = transcribed(5);
  follower.sync([5]);
  await follower.tick(); // the transcription just ended: the summary may be about to be queued
  assert.equal(log.reloads, 0, 'not yet: a summary may come');
  assert.equal(follower.running, true);
  // The shell has the row without a job now: it says so, and the meeting is still followed.
  follower.sync([]);
  assert.equal(follower.running, true);
  assert.deepEqual(follower.following, [5]);
  for (let i = 1; i < SETTLE_TICKS; i++) {
    await follower.tick();
    assert.equal(log.reloads, 0, `still waiting at tick ${i + 1}`);
  }
  await follower.tick();
  assert.equal(log.reloads, 1, 'nothing came: the list is read once');
  assert.equal(follower.running, false);
  assert.equal(log.reads.length, SETTLE_TICKS + 1);
  follower.dispose();
});

test('a_summary_that_appears_during_the_grace_is_followed_to_its_end', async () => {
  const { follower, log, answers } = rig();
  answers[5] = transcribed(5);
  follower.sync([5]);
  await follower.tick();
  follower.sync([]);
  await follower.tick(); // still nothing
  answers[5] = summarizing(5); // the service queued it
  await follower.tick();
  assert.equal(log.reloads, 0);
  assert.deepEqual(follower.following, [5]);
  follower.sync([5]); // the shell now sees the job on the row
  await follower.tick();
  assert.equal(log.reloads, 0, 'the summary is not done yet');
  answers[5] = summarized(5);
  await follower.tick();
  assert.equal(log.reloads, 1, 'the summary is done: the list is read once');
  assert.equal(follower.running, false);
  follower.dispose();
});

test('a_stopped_recording_is_followed_until_its_transcription_shows_up', async () => {
  const { follower, log, answers } = rig();
  answers[7] = meeting(7); // saved, and no job yet: the service queues it a moment after the stop
  follower.settle(7);
  assert.equal(follower.running, true);
  await follower.tick();
  await follower.tick();
  assert.equal(log.reloads, 0);
  answers[7] = running(7);
  await follower.tick();
  assert.deepEqual(follower.following, [7], 'followed as a meeting in flight now');
  follower.sync([7]);
  answers[7] = summarized(7);
  await follower.tick();
  assert.equal(log.reloads, 1);
  assert.equal(follower.running, false);
  follower.dispose();
});

test('a_stopped_recording_that_gets_no_job_reads_the_list_once_after_the_grace', async () => {
  const { follower, log, answers } = rig();
  answers[7] = meeting(7); // the engine is blocked: no transcription will be queued
  follower.settle(7);
  follower.settle(7); // again: it does not start the count over
  for (let i = 0; i < SETTLE_TICKS - 1; i++) await follower.tick();
  assert.equal(log.reloads, 0);
  await follower.tick();
  assert.equal(log.reloads, 1);
  assert.equal(follower.running, false);
  assert.equal(log.reads.length, SETTLE_TICKS);
  follower.dispose();
});

test('reads_that_fail_give_up_after_three_ticks_and_start_over_for_another_set', async () => {
  const { follower, log, answers } = rig();
  const boom = new Error('boom');
  answers[3] = boom;
  follower.sync([3]);
  for (let i = 0; i < FOLLOW_FAILURES - 1; i++) await follower.tick();
  assert.deepEqual(log.giveUps, [], 'two failures in a row are not enough');
  await follower.tick();
  assert.deepEqual(log.giveUps, [boom]);
  assert.equal(follower.running, false, 'it stopped trying');
  follower.sync([3]); // the same set: it stays given up
  assert.equal(follower.running, false);

  // Another set: the count starts from zero, so the first failures do not give up at once.
  answers[9] = boom;
  follower.sync([3, 9]);
  assert.equal(follower.running, true);
  for (let i = 0; i < FOLLOW_FAILURES - 1; i++) await follower.tick();
  assert.equal(log.giveUps.length, 1, 'two failures of the new set do not give up yet');
  await follower.tick();
  assert.equal(log.giveUps.length, 2);
  follower.dispose();
});

test('a_read_that_works_resets_the_count_of_failures', async () => {
  const { follower, log, answers } = rig();
  answers[3] = new Error('boom');
  follower.sync([3]);
  await follower.tick();
  await follower.tick();
  answers[3] = running(3);
  await follower.tick();
  answers[3] = new Error('boom');
  await follower.tick();
  await follower.tick();
  assert.deepEqual(log.giveUps, [], 'never three in a row');
  follower.dispose();
});

test('a_meeting_that_no_longer_exists_is_dropped_and_never_gives_up_the_others', async () => {
  const { follower, log, answers } = rig();
  const gone = Object.assign(new Error('Meeting not found'), { code: 'not_found' });
  answers[5] = gone;
  answers[8] = running(8);
  follower.sync([8]);
  follower.settle(5); // its recording ended, then the meeting was deleted before the job showed up
  assert.deepEqual(follower.following, [5, 8]);
  await follower.tick();
  assert.deepEqual(follower.following, [8], 'the deleted meeting is not read again');
  for (let i = 0; i < FOLLOW_FAILURES + 2; i++) await follower.tick();
  assert.deepEqual(log.giveUps, [], 'a deleted meeting is not a failing read');
  assert.equal(follower.running, true, 'the other meeting is still followed');
  assert.equal(log.reads.filter((id) => id === 5).length, 1, 'read once, then dropped');
  follower.dispose();
});

test('a_followed_meeting_that_is_deleted_is_dropped_and_the_list_is_read_once', async () => {
  const { follower, log, answers } = rig();
  answers[5] = Object.assign(new Error('Meeting not found'), { code: 'not_found' });
  follower.sync([5]);
  await follower.tick();
  assert.deepEqual(follower.following, []);
  assert.equal(follower.running, false, 'nothing left to follow, no timer');
  assert.deepEqual(log.giveUps, []);
  assert.equal(log.reloads, 1, 'the list is read once to drop the row');
  follower.dispose();
});

test('a_failed_read_does_not_hold_back_the_others', async () => {
  const { follower, log, answers } = rig();
  answers[3] = new Error('boom');
  answers[4] = running(4);
  follower.sync([3, 4]);
  await follower.tick();
  assert.deepEqual(log.applied, [4]);
  follower.dispose();
});

test('a_hidden_window_is_not_read_and_the_grace_does_not_run_down', async () => {
  const { follower, log, answers, hide } = rig();
  answers[5] = transcribed(5);
  follower.sync([5]);
  await follower.tick(); // settle starts
  hide(true);
  for (let i = 0; i < 10; i++) await follower.tick();
  assert.equal(log.reads.length, 1, 'ten ticks hidden, no read');
  assert.equal(log.reloads, 0);
  hide(false);
  for (let i = 0; i < SETTLE_TICKS; i++) await follower.tick();
  assert.equal(log.reloads, 1, 'the whole grace was still there');
  follower.dispose();
});

test('an_answer_older_than_what_the_page_has_taken_is_dropped', async () => {
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  const { follower, log, answers, bump } = rig();
  answers[3] = async () => {
    await gate;
    return summarized(3);
  };
  follower.sync([3]);
  const pending = follower.tick();
  bump(); // a list read, or the pane's read, came in meanwhile
  release();
  await pending;
  assert.deepEqual(log.applied, []);
  assert.equal(log.reloads, 0);
  assert.equal(follower.running, true, 'it will read again at the next tick');
  follower.dispose();
});

test('one_tick_at_a_time', async () => {
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  const { follower, log, answers } = rig();
  answers[3] = async () => {
    await gate;
    return running(3);
  };
  follower.sync([3]);
  const first = follower.tick();
  await follower.tick(); // returns at once: the first is still on its way
  release();
  await first;
  assert.deepEqual(log.reads, [3]);
  follower.dispose();
});

test('dispose_stops_the_timer_and_everything_after_it', async () => {
  const { follower, log, answers } = rig();
  answers[3] = running(3);
  follower.sync([3]);
  follower.settle(4);
  follower.dispose();
  assert.equal(follower.running, false);
  assert.deepEqual(follower.following, []);
  follower.sync([3]);
  follower.settle(5);
  assert.equal(follower.running, false, 'a gone page does not start it again');
  await follower.tick();
  mock.timers.tick(5 * FOLLOW_MS);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(log.reads, []);
});

test('an_answer_that_comes_after_dispose_changes_nothing', async () => {
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  const { follower, log, answers } = rig();
  answers[3] = async () => {
    await gate;
    return summarized(3);
  };
  follower.sync([3]);
  const pending = follower.tick();
  follower.dispose();
  release();
  await pending;
  assert.deepEqual(log.applied, []);
  assert.equal(log.reloads, 0);
});
