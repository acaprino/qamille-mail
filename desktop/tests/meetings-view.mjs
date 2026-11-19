// Task 10: the pure logic behind the simple Meetings page (src/lib/meetings-view.ts).
// Synthetic data only; `now` is fixed because the module never reads the clock.
import assert from 'node:assert/strict';
import test from 'node:test';
import { emptyCapture, lastCapture, publishCapture, subscribeCapture } from '../src/lib/capture-bus.ts';
import {
  GROUP_ORDER,
  PAST_DAYS,
  UPCOMING_DAYS,
  awaitsSummary,
  captureMark,
  clockLabel,
  detailState,
  groupRows,
  hasActiveJob,
  hitTarget,
  mergeMeetingRows,
  nextClockBoundary,
  pickDevices,
  processingIds,
  stoppedMeeting,
  summaryPlainText,
  transcriptPlainText,
  transcriptRows,
  usableHits,
} from '../src/lib/meetings-view.ts';

const HOUR = 3600;
const DAY = 86_400;
const NOW = 1_800_000_000;

const meeting = (id, over = {}, detailOver = {}) => ({
  schema_version: 2,
  revision: 1,
  meeting: {
    id,
    title: `Riunione ${id}`,
    date: NOW - 2 * HOUR,
    duration_secs: 1800,
    audio_path: null,
    transcript: '',
    notes: '',
    summary: '',
    key_points: [],
    action_items: [],
    event_id: null,
    ...over,
  },
  request_id: '',
  capture: {
    meeting_id: id,
    state: 'saved',
    elapsed_ms: 0,
    saved_ms: 0,
    microphone_level: 0,
    system_level: 0,
    chunks: [],
    error: null,
  },
  transcripts: [],
  active_transcript: null,
  notes: [],
  legacy_transcript: null,
  summaries: [],
  active_summary: null,
  jobs: [],
  included_in_search: true,
  deleted: false,
  audio_removed: false,
  audio_kept: null,
  has_audio: false,
  has_text: false,
  ...detailOver,
});
const event = (id, start, over = {}) => ({
  id,
  title: `Evento ${id}`,
  start,
  end: start + HOUR,
  location: '',
  attendees: [],
  meeting_link: null,
  conference: '',
  rsvp: 'None',
  notes: '',
  color: [0, 0, 0],
  ...over,
});
const job = (id, kind, state, over = {}) => ({
  id,
  kind,
  state,
  input_revision: 1,
  progress: 0,
  error: null,
  created_at: 1,
  ...over,
});
const withCapture = (detail, state, over = {}) => ({
  ...detail,
  capture: { ...detail.capture, state, ...over },
});
const transcript = (id, texts, over = {}) => ({
  id,
  created_at: 1,
  model: 'm',
  language: 'it',
  complete: true,
  segments: texts.map((text, index) => ({
    id: `${id}-s${index}`,
    start_ms: index * 65_000,
    end_ms: index * 65_000 + 5000,
    text,
    original_text: text,
    source: 'mixed',
    confidence: null,
  })),
  ...over,
});
const summaryVersion = (id, over = {}) => ({
  id,
  created_at: 1,
  transcript_version: null,
  input_revision: 1,
  language: 'it',
  model: 'm',
  summary: 'Si è deciso di partire a marzo.',
  key_points: [],
  decisions: [],
  actions: [],
  complete: true,
  ...over,
});
const ready = { transcription_blocker: null, summary_ready: true };

const frozen = (value) => {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    Object.values(value).forEach(frozen);
  }
  return value;
};

test('merge_puts_a_recording_in_place_of_its_event', () => {
  const ev = event(7, NOW - 2 * HOUR, {
    attendees: ['a@x.it', 'b@x.it', 'c@x.it'],
    location: 'Sala 2',
    meeting_link: 'https://meet/x',
  });
  const rec = meeting(
    1,
    { event_id: 7, title: 'Pianificazione', date: ev.start },
    { has_audio: true, has_text: true },
  );
  const rows = mergeMeetingRows({ meetings: [rec], events: [ev], now: NOW });
  assert.equal(rows.length, 1, 'no double row for a recorded event');
  const [row] = rows;
  assert.equal(row.key, 'm:1');
  assert.equal(row.group, 'past');
  assert.equal(row.status, 'ready');
  assert.equal(row.meetingId, 1);
  assert.equal(row.eventId, 7);
  assert.equal(row.title, 'Pianificazione');
  assert.equal(row.attendees, 3, 'the attendees come from the event');
  assert.equal(row.place, 'Sala 2');
  assert.equal(row.meetingLink, 'https://meet/x');
  assert.equal(row.openable, true);
  assert.equal(row.job, null);
  assert.equal(row.durationSecs, 1800);
  // A recording of an upcoming event still hides the event from Prossime.
  const early = meeting(2, { event_id: 8, date: NOW - 60 });
  const upcoming = event(8, NOW + HOUR);
  const merged = mergeMeetingRows({ meetings: [early], events: [upcoming], now: NOW });
  assert.deepEqual(
    merged.map((r) => r.key),
    ['m:2'],
  );
});

test('merge_keeps_the_row_title_pure_and_the_component_names_an_untitled_meeting', () => {
  // The row carries the event's title when the meeting has none, and an empty one when neither does:
  // the words "Riunione senza nome" belong to the component (they are translated there).
  const ev = event(7, NOW - 2 * HOUR, { title: 'Titolo del calendario' });
  const linked = meeting(1, { event_id: 7, title: '   ', date: ev.start });
  const [fromEvent] = mergeMeetingRows({ meetings: [linked], events: [ev], now: NOW });
  assert.equal(fromEvent.title, 'Titolo del calendario');
  const bare = meeting(2, { title: '' });
  const [nameless] = mergeMeetingRows({ meetings: [bare], events: [], now: NOW });
  assert.equal(nameless.title, '');
});

test('merge_keeps_unrecorded_past_events_grey_and_closed', () => {
  const past = event(3, NOW - 5 * HOUR, { attendees: ['a@x.it'] });
  const [row] = mergeMeetingRows({ meetings: [], events: [past], now: NOW });
  assert.equal(row.key, 'e:3');
  assert.equal(row.group, 'past');
  assert.equal(row.status, 'unrecorded');
  assert.equal(row.openable, false);
  assert.equal(row.meetingId, null);
  assert.equal(row.eventId, 3);
  assert.equal(row.durationSecs, HOUR);
  assert.equal(row.attendees, 1);
  assert.equal(row.startedSecsAgo, null);
  // A meeting without an event has no attendee count to show.
  const [alone] = mergeMeetingRows({ meetings: [meeting(4)], events: [], now: NOW });
  assert.equal(alone.attendees, null);
  assert.equal(alone.eventId, null);
});

test('merge_skips_all_day_events_and_the_far_past_and_future', () => {
  const events = [
    event(1, NOW + HOUR, { end: NOW + HOUR + 23 * HOUR }), // all day: 23 h (a spring-forward day) is out
    event(2, NOW + HOUR, { end: NOW + HOUR + 23 * HOUR - 1 }), // one second under 23 h: kept
    event(9, NOW + HOUR, { end: NOW + HOUR + DAY }), // a plain 24 h day: out
    event(3, NOW + UPCOMING_DAYS * DAY), // starts exactly at the limit: out
    event(4, NOW + UPCOMING_DAYS * DAY - 1), // just inside
    event(5, NOW - PAST_DAYS * DAY - HOUR), // ended before the window opens: out
    event(6, NOW - PAST_DAYS * DAY), // starts exactly at the limit: in
    event(7, NOW - 3 * DAY, { end: NOW - 3 * DAY + 2 * DAY }), // two days long: out
    event(8, NOW - 2 * HOUR, { end: NOW }), // ended right now: past
  ];
  assert.equal(UPCOMING_DAYS, 7);
  assert.equal(PAST_DAYS, 14);
  const keys = mergeMeetingRows({ meetings: [], events, now: NOW })
    .map((r) => r.key)
    .sort();
  assert.deepEqual(keys, ['e:2', 'e:4', 'e:6', 'e:8']);
});

test('merge_leaves_declined_events_out_of_prossime', () => {
  const events = [
    event(1, NOW + HOUR, { rsvp: 'Declined' }), // upcoming, declined: not offered
    event(2, NOW + HOUR, { rsvp: 'Accepted' }),
    event(3, NOW + 2 * HOUR, { rsvp: 'Maybe' }),
    event(4, NOW + 3 * HOUR, { rsvp: 'None' }),
    event(5, NOW - 3 * HOUR, { rsvp: 'Declined' }), // past: it stays the grey "Non registrata" row
    event(6, NOW - 30 * 60, { end: NOW + 30 * 60, rsvp: 'Declined' }), // under way, declined: not offered either
  ];
  const rows = mergeMeetingRows({ meetings: [], events, now: NOW });
  assert.deepEqual(rows.filter((r) => r.group === 'upcoming').map((r) => r.key), ['e:2', 'e:3', 'e:4']);
  assert.deepEqual(rows.filter((r) => r.group === 'past').map((r) => r.key), ['e:5']);
  // A declined event nobody offers does not set the clock either.
  assert.equal(
    nextClockBoundary(mergeMeetingRows({ meetings: [], events: [events[0]], now: NOW }), NOW),
    null,
  );
});

test('merge_skips_events_that_end_before_they_start', () => {
  const bad = [
    event(1, NOW - 2 * HOUR, { end: NOW - 3 * HOUR }), // past, backwards
    event(2, NOW + HOUR, { end: NOW + 30 * 60 }), // future, backwards
    event(3, NOW - HOUR, { end: NOW - 2 * HOUR }),
  ];
  assert.deepEqual(mergeMeetingRows({ meetings: [], events: bad, now: NOW }), []);
  // An event with no length is not bad data; it has no duration to show.
  const [point] = mergeMeetingRows({
    meetings: [],
    events: [event(4, NOW - HOUR, { end: NOW - HOUR })],
    now: NOW,
  });
  assert.equal(point.key, 'e:4');
  assert.equal(point.durationSecs, null);
});

test('merge_handles_no_input_and_an_event_that_is_not_there', () => {
  assert.deepEqual(mergeMeetingRows({ meetings: [], events: [], now: NOW }), []);
  // The event of a recording may be gone from the calendar (or out of the loaded range).
  const orphan = meeting(1, { event_id: 99, title: 'Senza evento' }, { has_text: true });
  const other = event(5, NOW + HOUR);
  const rows = mergeMeetingRows({ meetings: [orphan], events: [other], now: NOW });
  assert.deepEqual(
    rows.map((r) => r.key),
    ['e:5', 'm:1'],
    'Prossime come prima, then Passate',
  );
  const row = rows.find((r) => r.key === 'm:1');
  assert.equal(row.eventId, 99);
  assert.equal(row.title, 'Senza evento');
  assert.equal(row.attendees, null);
  assert.equal(row.place, null);
  assert.equal(row.meetingLink, null);
});

test('merge_marks_a_started_event_with_startedSecsAgo', () => {
  const started = event(1, NOW - 600, { end: NOW + 3000 });
  const later = event(2, NOW + 900);
  const rows = mergeMeetingRows({ meetings: [], events: [later, started], now: NOW });
  const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
  assert.equal(byKey['e:1'].group, 'upcoming');
  assert.equal(byKey['e:1'].status, 'upcoming');
  assert.equal(byKey['e:1'].startedSecsAgo, 600);
  assert.equal(byKey['e:2'].startedSecsAgo, null);
  assert.equal(byKey['e:1'].openable, false);
});

test('merge_assigns_groups_and_statuses_to_recordings', () => {
  const live = withCapture(meeting(1, { date: NOW - 300 }), 'recording');
  const saving = withCapture(meeting(2), 'saving');
  const transcribing = {
    ...meeting(3, {}, { has_audio: true }),
    jobs: [
      job('old', 'transcribe', 'completed', { created_at: 10, progress: 100 }),
      job('a', 'transcribe', 'running', { created_at: 20, progress: 40 }),
      job('b', 'summarize', 'queued', { created_at: 30, progress: 5 }),
    ],
  };
  const interrupted = withCapture(meeting(4), 'interrupted');
  const toTranscribe = meeting(5, {}, { has_audio: true });
  const failed = {
    ...meeting(6, {}, { has_audio: true }),
    jobs: [job('f', 'transcribe', 'failed')],
  };
  const cancelled = {
    ...meeting(7, {}, { has_audio: true }),
    jobs: [job('c', 'transcribe', 'cancelled')],
  };
  const failedButHasText = {
    ...meeting(8, {}, { has_audio: true, has_text: true }),
    jobs: [job('f', 'transcribe', 'failed')],
  };
  const done = meeting(9, {}, { has_audio: false, has_text: true });
  const noAudioNoText = meeting(10);
  const onlySummarizeRunning = {
    ...meeting(11, {}, { has_text: true }),
    jobs: [job('s', 'summarize', 'running', { progress: 70 })],
  };
  const importRunning = {
    ...meeting(12, {}, { has_text: true }),
    jobs: [job('i', 'import', 'running')],
  };
  const empty = meeting(13, {}, { has_audio: true, audio_kept: 'empty_transcript' });
  const emptyButRetrying = {
    ...meeting(14, {}, { has_audio: true, audio_kept: 'empty_transcript' }),
    jobs: [job('r', 'transcribe', 'running', { progress: 20 })],
  };
  const emptyThenFailed = {
    ...meeting(15, {}, { has_audio: true, audio_kept: 'empty_transcript' }),
    jobs: [job('f', 'transcribe', 'failed')],
  };
  const emptyButLegacyText = meeting(
    16,
    {},
    { has_audio: true, has_text: true, audio_kept: 'empty_transcript' },
  );
  const emptyInterrupted = withCapture(
    meeting(17, {}, { has_audio: true, audio_kept: 'empty_transcript' }),
    'interrupted',
  );
  const rows = mergeMeetingRows({
    meetings: frozen([
      live,
      saving,
      transcribing,
      interrupted,
      toTranscribe,
      failed,
      cancelled,
      failedButHasText,
      done,
      noAudioNoText,
      onlySummarizeRunning,
      importRunning,
      empty,
      emptyButRetrying,
      emptyThenFailed,
      emptyButLegacyText,
      emptyInterrupted,
    ]),
    events: [],
    now: NOW,
  });
  const by = Object.fromEntries(rows.map((r) => [r.meetingId, r]));
  assert.deepEqual([by[1].group, by[1].status], ['live', 'recording']);
  assert.deepEqual([by[2].group, by[2].status], ['live', 'recording']);
  assert.deepEqual([by[3].group, by[3].status], ['processing', 'processing']);
  assert.deepEqual(
    by[3].job,
    { kind: 'summarize', progress: 5 },
    'the most recent job gives the progress',
  );
  assert.deepEqual([by[4].group, by[4].status], ['past', 'interrupted']);
  assert.equal(by[5].status, 'to_transcribe');
  assert.equal(by[6].status, 'failed');
  assert.equal(by[7].status, 'failed');
  assert.equal(by[8].status, 'ready', 'a failed job does not matter once there is text');
  assert.equal(by[9].status, 'ready');
  assert.equal(by[10].status, 'ready');
  assert.deepEqual([by[11].group, by[11].job], ['processing', { kind: 'summarize', progress: 70 }]);
  assert.deepEqual(
    [by[12].group, by[12].status],
    ['past', 'ready'],
    'an import is not shown as processing',
  );
  assert.equal(by[13].status, 'empty', 'an empty transcript is not "to transcribe"');
  assert.equal(by[14].status, 'processing', 'a job in progress comes first');
  assert.equal(by[15].status, 'failed', 'a failed job is the newer truth');
  assert.equal(by[16].status, 'ready', 'text from before the versions makes it ready');
  assert.equal(by[17].status, 'interrupted');
  assert.ok(Object.values(by).every((r) => r.openable));
});

test('merge_does_not_mutate_or_read_the_clock', () => {
  const original = Date.now;
  Date.now = () => {
    throw new Error('Date.now() must not be called');
  };
  try {
    const rec = frozen(meeting(1));
    const ev = frozen(event(2, NOW + HOUR));
    assert.equal(mergeMeetingRows({ meetings: [rec], events: [ev], now: NOW }).length, 2);
    assert.equal(detailState(rec, { readiness: ready }).kind, 'no_transcript');
  } finally {
    Date.now = original;
  }
});

test('merge_hides_a_deleted_meeting_and_gives_its_event_back', () => {
  const gone = frozen(meeting(3, { event_id: 9 }, { deleted: true }));
  const back = mergeMeetingRows({ meetings: [gone], events: [event(9, NOW - HOUR)], now: NOW });
  assert.deepEqual(
    back.map((r) => r.key),
    ['e:9'],
  );
  assert.equal(back[0].status, 'unrecorded');
  assert.deepEqual(mergeMeetingRows({ meetings: [gone], events: [], now: NOW }), []);
});

test('next_clock_boundary_is_the_next_moment_a_row_of_prossime_changes', () => {
  const rowsAt = (events, now = NOW) => mergeMeetingRows({ meetings: [], events, now });
  // Nothing to wait for: no rows, only grey past events, only recordings.
  assert.equal(nextClockBoundary([], NOW), null, 'no rows, no deadline');
  assert.equal(
    nextClockBoundary(rowsAt([event(1, NOW - 3 * HOUR)]), NOW),
    null,
    'an event that is over never changes again',
  );
  const recording = mergeMeetingRows({ meetings: [meeting(1)], events: [], now: NOW });
  assert.equal(nextClockBoundary(recording, NOW), null, 'a recording is not on the clock');

  // An event that has not begun: the first second after its start, when the row says "Iniziata".
  assert.equal(nextClockBoundary(rowsAt([event(1, NOW + 40 * 60)]), NOW), NOW + 40 * 60 + 1);
  // Several: the soonest change, whatever the order.
  assert.equal(
    nextClockBoundary(rowsAt([event(1, NOW + 2 * HOUR), event(2, NOW + 600), event(3, NOW + HOUR)]), NOW),
    NOW + 601,
  );
  // One that begins at this very second is not "started" yet: the next second tells.
  assert.equal(nextClockBoundary(rowsAt([event(1, NOW)]), NOW), NOW + 1);
  // One that has begun: the next minute of "Iniziata da N min" (counted from its start), unless it
  // ends before that.
  const begun = rowsAt([event(1, NOW - 100)]);
  assert.equal(begun[0].startedSecsAgo, 100);
  assert.equal(nextClockBoundary(begun, NOW), NOW - 100 + 120, 'at 1:40 in, the minute turns at 2:00');
  assert.equal(
    nextClockBoundary(rowsAt([event(1, NOW - 120)]), NOW),
    NOW - 120 + 180,
    'exactly on a minute: the next one',
  );
  assert.equal(
    nextClockBoundary(rowsAt([event(1, NOW - 100, { end: NOW + 5 })]), NOW),
    NOW + 5,
    'its end comes first',
  );
  // A started event and a later one: the minute of the first.
  assert.equal(
    nextClockBoundary(rowsAt([event(1, NOW - 10), event(2, NOW + HOUR)]), NOW),
    NOW - 10 + 60,
  );
  // The deadline is always ahead of now, and following it ends: a page that re-arms itself on
  // each deadline never spins, and stops when the event is over.
  let now = NOW;
  const events = [event(1, NOW + 90, { end: NOW + 400 })];
  const seen = [];
  for (let step = 0; step < 20; step++) {
    const next = nextClockBoundary(rowsAt(events, now), now);
    if (next === null) break;
    assert.ok(next > now, `${next} is ahead of ${now}`);
    seen.push(next - NOW);
    now = next;
  }
  assert.deepEqual(seen, [91, 150, 210, 270, 330, 390, 400], 'the start, every minute, then the end');
  assert.equal(nextClockBoundary(rowsAt(events, now), now), null, 'then nothing is left to wait for');
  // An event with no length ends where it starts.
  assert.equal(nextClockBoundary(rowsAt([event(1, NOW + 30, { end: NOW + 30 })]), NOW), NOW + 30);
});

test('group_order_and_sorting', () => {
  const meetings = [
    withCapture(meeting(1, { date: NOW - 100 }), 'recording'),
    withCapture(meeting(2, { date: NOW - 900 }), 'paused'),
    {
      ...meeting(3, { date: NOW - 4 * HOUR }, { has_text: true }),
      jobs: [job('j', 'transcribe', 'running', { progress: 10 })],
    },
    meeting(4, { date: NOW - 3 * DAY }, { has_text: true }),
    meeting(5, { date: NOW - 1 * DAY }, { has_text: true }),
  ];
  const events = [
    event(10, NOW + 2 * DAY),
    event(11, NOW + HOUR),
    event(12, NOW - 2 * DAY),
    event(13, NOW - 5 * HOUR),
  ];
  const rows = mergeMeetingRows({ meetings, events, now: NOW });
  const groups = groupRows(rows);
  assert.deepEqual(Object.keys(groups), ['live', 'processing', 'upcoming', 'past']);
  assert.deepEqual(
    groups.live.map((r) => r.key),
    ['m:1', 'm:2'],
    'the newest recording first',
  );
  assert.deepEqual(
    groups.processing.map((r) => r.key),
    ['m:3'],
  );
  assert.deepEqual(
    groups.upcoming.map((r) => r.key),
    ['e:11', 'e:10'],
    'soonest first',
  );
  assert.deepEqual(
    groups.past.map((r) => r.key),
    ['e:13', 'm:5', 'e:12', 'm:4'],
    'newest first, recordings and events mixed',
  );
  // groupRows does not depend on the order it is given.
  const shuffled = groupRows([...rows].reverse());
  for (const group of Object.keys(groups))
    assert.deepEqual(
      shuffled[group].map((r) => r.key),
      groups[group].map((r) => r.key),
    );
  assert.deepEqual(groupRows([]), { live: [], processing: [], upcoming: [], past: [] });
});

test('group_order_is_the_one_the_list_draws', () => {
  assert.deepEqual(GROUP_ORDER, ['live', 'processing', 'upcoming', 'past']);
  assert.deepEqual(Object.keys(groupRows([])), GROUP_ORDER);
});

test('processing_ids_are_the_meetings_the_page_keeps_reading', () => {
  const running = (id, over = {}) => ({
    ...meeting(id, {}, { has_audio: true, ...over }),
    jobs: [job(`j${id}`, 'transcribe', 'running', { progress: 40 })],
  });
  const details = [
    running(9),
    running(3),
    { ...meeting(4), jobs: [job('a', 'summarize', 'queued')] },
    { ...meeting(5), jobs: [job('b', 'transcribe', 'completed'), job('c', 'summarize', 'failed')] },
    { ...meeting(6), jobs: [job('d', 'cancelled-kind', 'running')] },
    withCapture(running(7), 'recording'),
    { ...running(8), deleted: true },
    meeting(2),
  ];
  assert.deepEqual(processingIds(details), [3, 4, 9], 'only a transcription or summary waiting or running');
  assert.deepEqual(processingIds([]), []);
  assert.equal(hasActiveJob(details[0]), true);
  assert.equal(hasActiveJob(details[3]), false, 'a finished or failed job is not in flight');
  assert.equal(hasActiveJob(details[4]), false, 'only transcribe and summarize count');
  // The answer is the same set the list puts in elaborazione.
  const rows = groupRows(mergeMeetingRows({ meetings: details, events: [], now: NOW }));
  assert.deepEqual(
    rows.processing.map((row) => row.meetingId).sort((a, b) => a - b),
    processingIds(details),
  );
});

test('processing_ids_list_a_meeting_once_when_it_is_in_the_list_and_open', () => {
  const running = {
    ...meeting(3, {}, { has_audio: true }),
    jobs: [job('j', 'transcribe', 'running')],
  };
  assert.deepEqual(processingIds([running, running, meeting(2)]), [3]);
});

test('awaits_summary_is_a_finished_transcription_with_no_summary_job_after_it', () => {
  const withJobs = (...jobs) => ({ ...meeting(1), jobs });
  assert.equal(awaitsSummary(withJobs(job('t', 'transcribe', 'completed'))), true);
  assert.equal(
    awaitsSummary(withJobs(job('t', 'transcribe', 'completed'), job('s', 'summarize', 'queued'))),
    false,
    'a summary is already queued: it is a job in flight',
  );
  assert.equal(
    awaitsSummary(withJobs(job('t', 'transcribe', 'completed'), job('s', 'summarize', 'failed'))),
    false,
    'a summary was tried and failed: nothing more is coming',
  );
  assert.equal(
    awaitsSummary(
      withJobs(
        job('s0', 'summarize', 'completed'),
        job('t', 'transcribe', 'completed'),
      ),
    ),
    true,
    'only a summary after the last transcription counts',
  );
  assert.equal(awaitsSummary(withJobs(job('t', 'transcribe', 'running'))), false);
  assert.equal(awaitsSummary(withJobs(job('t', 'transcribe', 'failed'))), false);
  assert.equal(awaitsSummary(withJobs(job('t', 'transcribe', 'cancelled'))), false);
  assert.equal(awaitsSummary(meeting(1)), false, 'never transcribed');
});

test('stopped_meeting_is_the_recording_that_ended_between_two_snapshots', () => {
  const snap = (state, id) => ({ state, meeting_id: id });
  assert.equal(stoppedMeeting(snap('recording', 4), snap('saved', 4)), 4);
  assert.equal(stoppedMeeting(snap('paused', 4), snap('idle', null)), 4);
  assert.equal(stoppedMeeting(snap('saving', 4), snap('saved', 4)), 4);
  assert.equal(stoppedMeeting(snap('recording', 4), snap('recording', 5)), 4, 'another recording');
  assert.equal(stoppedMeeting(snap('recording', 4), snap('paused', 4)), null, 'still the same one');
  assert.equal(stoppedMeeting(snap('idle', null), snap('recording', 4)), null, 'it started');
  assert.equal(stoppedMeeting(snap('saved', 4), snap('idle', null)), null, 'nothing was recording');
  assert.equal(stoppedMeeting(snap('interrupted', 4), snap('saved', 4)), null);
});

test('capture_mark_changes_only_when_a_recording_starts_stops_or_is_another', () => {
  const mark = (state, id) => captureMark({ state, meeting_id: id });
  assert.equal(mark('recording', 4), mark('paused', 4), 'pausing is not a change');
  assert.equal(mark('preparing', 4), mark('saving', 4));
  assert.notEqual(mark('recording', 4), mark('saved', 4), 'a recording that stopped');
  assert.notEqual(mark('idle', null), mark('recording', 4), 'a recording that started');
  assert.notEqual(mark('recording', 4), mark('recording', 5), 'another meeting is being recorded');
  assert.equal(mark('idle', null), mark('idle', null));
  assert.equal(mark('saved', 4), mark('interrupted', 4), 'neither is a live recording');
  assert.notEqual(mark('idle', null), mark('saved', 4), 'the first snapshot about a saved meeting');
});

test('detail_state_covers_every_case_in_the_table', () => {
  const blocked = { transcription_blocker: 'Nessun modello installato', summary_ready: false };

  // live
  for (const state of ['preparing', 'recording', 'paused', 'saving'])
    assert.deepEqual(
      detailState(withCapture(meeting(1), state), { readiness: ready }),
      { kind: 'live' },
      state,
    );

  // processing: the latest queued/running job, live wins over it
  const running = job('r', 'transcribe', 'running', { progress: 30 });
  assert.deepEqual(
    detailState({ ...meeting(1, {}, { has_audio: true }), jobs: [running] }, { readiness: ready }),
    { kind: 'processing', job: running },
  );
  assert.equal(
    detailState({ ...withCapture(meeting(1), 'recording'), jobs: [running] }, { readiness: ready })
      .kind,
    'live',
  );

  // a summarize job alone is only `processing` when there is no transcript to show
  const summarizing = job('sm', 'summarize', 'running');
  assert.deepEqual(
    detailState(
      { ...meeting(1, {}, { has_audio: true }), jobs: [summarizing] },
      { readiness: ready },
    ),
    { kind: 'processing', job: summarizing },
  );

  // recover
  assert.deepEqual(detailState(withCapture(meeting(1), 'interrupted'), { readiness: ready }), {
    kind: 'recover',
  });

  // no_transcript: failed (with the failed job), blocked, never
  const failedJob = job('f', 'transcribe', 'failed', {
    error: 'Modello non trovato',
    created_at: 5,
  });
  const olderFailed = job('o', 'transcribe', 'failed', { created_at: 1 });
  assert.deepEqual(
    detailState(
      { ...meeting(1, {}, { has_audio: true }), jobs: [olderFailed, failedJob] },
      { readiness: blocked },
    ),
    { kind: 'no_transcript', reason: 'failed', message: 'Modello non trovato', failedJob },
    'a failed job wins over a blocker',
  );
  const retried = job('ok', 'transcribe', 'completed', { created_at: 9 });
  const afterRetry = detailState(
    { ...meeting(1, {}, { has_audio: true }), jobs: [failedJob, retried] },
    { readiness: ready },
  );
  assert.deepEqual(
    afterRetry,
    { kind: 'no_transcript', reason: 'never', message: null, failedJob: null },
    'only the latest transcribe job counts',
  );
  assert.deepEqual(detailState(meeting(1, {}, { has_audio: true }), { readiness: blocked }), {
    kind: 'no_transcript',
    reason: 'blocked',
    message: 'Nessun modello installato',
    failedJob: null,
  });
  assert.deepEqual(detailState(meeting(1, {}, { has_audio: true }), { readiness: ready }), {
    kind: 'no_transcript',
    reason: 'never',
    message: null,
    failedJob: null,
  });
  // no_transcript: empty (the audio was kept because the transcript has no words)
  const emptyMeeting = (over = {}) =>
    meeting(1, {}, { has_audio: true, audio_kept: 'empty_transcript', ...over });
  const emptyState = { kind: 'no_transcript', reason: 'empty', message: null, failedJob: null };
  assert.deepEqual(detailState(emptyMeeting(), { readiness: ready }), emptyState);
  assert.deepEqual(
    detailState(emptyMeeting(), { readiness: blocked }),
    emptyState,
    'empty comes before a blocker',
  );
  const newerFailure = job('nf', 'transcribe', 'failed', { error: 'Disco pieno', created_at: 20 });
  assert.deepEqual(
    detailState(
      {
        ...emptyMeeting(),
        jobs: [job('first', 'transcribe', 'completed', { created_at: 3 }), newerFailure],
      },
      { readiness: ready },
    ),
    { kind: 'no_transcript', reason: 'failed', message: 'Disco pieno', failedJob: newerFailure },
    'a newer failed transcription is the truth',
  );
  assert.deepEqual(
    detailState(
      { ...emptyMeeting(), jobs: [job('old', 'transcribe', 'failed', { created_at: 1 }), retried] },
      { readiness: ready },
    ),
    emptyState,
    'an old failure that was retried does not hide the empty transcript',
  );
  assert.equal(
    detailState(emptyMeeting({ audio_removed: true }), { readiness: ready }).reason,
    'never',
    'once the audio is deleted there is nothing left to say',
  );
  assert.equal(
    detailState(emptyMeeting({ audio_kept: 'interrupted_capture' }), { readiness: blocked }).reason,
    'blocked',
    'only an empty transcript has its own reason',
  );

  // an import job is not the page's business: it neither blocks nor shows a bar
  const importing = job('im', 'import', 'running');
  assert.equal(
    detailState({ ...meeting(1, {}, { has_audio: true }), jobs: [importing] }, { readiness: ready })
      .reason,
    'never',
  );
  assert.equal(
    detailState({ ...meeting(1, {}, { has_text: true }), jobs: [importing] }, { readiness: ready })
      .kind,
    'ready',
  );

  for (const state of ['failed', 'cancelled', 'interrupted']) {
    const j = job('x', 'transcribe', state);
    assert.equal(
      detailState({ ...meeting(1, {}, { has_audio: true }), jobs: [j] }, { readiness: ready })
        .reason,
      'failed',
      state,
    );
  }

  // ready: audio notes
  const text = (over = {}) => meeting(1, {}, { has_text: true, ...over });
  const stateOf = (detail, readiness = ready) => detailState(detail, { readiness });
  assert.deepEqual(stateOf(text()).audio, null);
  assert.equal(stateOf(text({ audio_removed: true })).audio, 'removed');
  assert.equal(stateOf(text({ audio_kept: 'empty_transcript' })).audio, 'empty_transcript');
  assert.equal(stateOf(text({ audio_kept: 'interrupted_capture' })).audio, 'interrupted_capture');
  assert.equal(stateOf(text({ audio_kept: 'something_new' })).audio, null);
  assert.equal(
    stateOf(text({ audio_removed: true, audio_kept: 'empty_transcript' })).audio,
    'removed',
  );

  // ready: summary
  assert.deepEqual(stateOf(text()).summary, { kind: 'generate' });
  assert.deepEqual(stateOf(text(), blocked).summary, { kind: 'ai_missing' });
  const withSummary = text({ summaries: [summaryVersion('s1')], active_summary: 's1' });
  assert.deepEqual(
    stateOf(withSummary, blocked).summary,
    { kind: 'text' },
    'a summary wins over everything',
  );
  const legacySummary = meeting(1, { summary: 'Riassunto vecchio' }, { has_text: true });
  assert.deepEqual(
    stateOf(legacySummary).summary,
    { kind: 'text' },
    'a pre-existing summary counts',
  );
  const summaryFailed = job('sf', 'summarize', 'failed', {
    error: 'Chiave non valida',
    created_at: 7,
  });
  assert.deepEqual(stateOf({ ...text(), jobs: [summaryFailed] }, blocked).summary, {
    kind: 'failed',
    message: 'Chiave non valida',
    jobId: 'sf',
  });
  const summaryNoMessage = job('sg', 'summarize', 'failed');
  assert.deepEqual(stateOf({ ...text(), jobs: [summaryNoMessage] }).summary, {
    kind: 'failed',
    message: '',
    jobId: 'sg',
  });
  const summaryRunning = job('sr', 'summarize', 'running', { created_at: 8, progress: 35 });
  assert.deepEqual(
    stateOf({ ...text(), jobs: [summaryFailed, summaryRunning] }).summary,
    { kind: 'pending', job: summaryRunning },
    'a newer running job replaces an older failure',
  );
  const summaryQueued = job('sq', 'summarize', 'queued');
  assert.deepEqual(stateOf({ ...text(), jobs: [summaryQueued] }, blocked).summary, {
    kind: 'pending',
    job: summaryQueued,
  });
  assert.deepEqual(
    stateOf({ ...text(), jobs: [summaryQueued, summaryRunning] }).summary,
    { kind: 'pending', job: summaryRunning },
    'the pending job is the newest one',
  );
  assert.deepEqual(
    stateOf({ ...text(), jobs: [job('t', 'transcribe', 'failed')] }).summary,
    { kind: 'generate' },
    'a transcription failure is not a summary failure',
  );
  const done = stateOf({ ...text(), jobs: [job('t', 'transcribe', 'completed')] });
  assert.equal(done.kind, 'ready');
});

test('detail_state_does_not_mutate', () => {
  const detail = frozen({
    ...meeting(1, {}, { has_text: true }),
    jobs: [job('a', 'summarize', 'failed')],
  });
  assert.equal(detailState(detail, { readiness: frozen(ready) }).kind, 'ready');
});

test('clock_label_switches_to_hours', () => {
  assert.equal(clockLabel(0), '00:00');
  assert.equal(clockLabel(999), '00:00');
  assert.equal(clockLabel(1000), '00:01');
  assert.equal(clockLabel(65_000), '01:05');
  assert.equal(clockLabel(59 * 60_000 + 59_999), '59:59');
  assert.equal(clockLabel(3_600_000), '01:00:00');
  assert.equal(clockLabel(3_600_000 + 65_000), '01:01:05');
  assert.equal(clockLabel(11 * 3_600_000 + 5 * 60_000 + 9000), '11:05:09');
  assert.equal(clockLabel(-5), '00:00');
  assert.equal(clockLabel(Number.NaN), '00:00');
});

test('transcript_rows_falls_back_to_plain_text', () => {
  const versioned = {
    ...meeting(1, {}, { has_text: true }),
    transcripts: [transcript('t1', ['Buongiorno a tutti', '   ', 'Iniziamo'])],
    active_transcript: 't1',
  };
  assert.deepEqual(transcriptRows(versioned), [
    { id: 't1-s0', time: '00:00', text: 'Buongiorno a tutti' },
    { id: 't1-s2', time: '02:10', text: 'Iniziamo' },
  ]);
  // The active version is used, not just any version.
  const two = {
    ...versioned,
    transcripts: [transcript('t0', ['vecchio']), ...versioned.transcripts],
  };
  assert.equal(transcriptRows(two)[0].text, 'Buongiorno a tutti');

  const legacy = meeting(
    2,
    {},
    { has_text: true, legacy_transcript: 'Testo importato\nsu due righe' },
  );
  assert.deepEqual(transcriptRows(legacy), [
    { id: 'plain', time: '', text: 'Testo importato\nsu due righe' },
  ]);
  const projected = meeting(3, { transcript: 'Dal vecchio campo' }, { has_text: true });
  assert.deepEqual(transcriptRows(projected), [
    { id: 'plain', time: '', text: 'Dal vecchio campo' },
  ]);
  // A transcript that is all blank falls back too, like the backend's has_text.
  const blankVersion = {
    ...legacy,
    transcripts: [transcript('t9', ['  ', ''])],
    active_transcript: 't9',
  };
  assert.equal(transcriptRows(blankVersion)[0].text, 'Testo importato\nsu due righe');
  assert.deepEqual(transcriptRows(meeting(4)), []);
});

test('plain_text_helpers_join_the_rows_and_the_summary', () => {
  const detail = {
    ...meeting(1, {}, { has_text: true }),
    transcripts: [transcript('t1', ['Buongiorno', 'Iniziamo'])],
    active_transcript: 't1',
    summaries: [
      summaryVersion('s1', {
        summary: 'Il progetto parte a marzo.',
        key_points: ['Budget approvato', 'Team di tre persone'],
        decisions: [{ text: 'Si parte il 3 marzo', sources: [] }],
        actions: [
          {
            id: 'a1',
            text: 'Preparare il piano',
            owner: 'Anna',
            due: '2027-03-01',
            done: false,
            sources: [],
          },
          { id: 'a2', text: 'Prenotare la sala', owner: null, due: null, done: true, sources: [] },
        ],
      }),
    ],
    active_summary: 's1',
  };
  assert.equal(transcriptPlainText(detail), '00:00 Buongiorno\n01:05 Iniziamo');
  assert.equal(
    transcriptPlainText(meeting(2, {}, { legacy_transcript: 'Solo testo' })),
    'Solo testo',
    'a plain row has no time to print',
  );
  assert.equal(transcriptPlainText(meeting(3)), '');
  assert.equal(
    summaryPlainText(detail),
    [
      'Il progetto parte a marzo.',
      '- Budget approvato\n- Team di tre persone',
      '- Si parte il 3 marzo',
      '- Preparare il piano (Anna, 2027-03-01)\n- Prenotare la sala',
    ].join('\n\n'),
  );
  const legacy = meeting(4, { summary: 'Vecchia sintesi', key_points: ['Uno'] });
  assert.equal(summaryPlainText(legacy), 'Vecchia sintesi\n\n- Uno');
  const withActions = meeting(6, {
    summary: 'Vecchia sintesi',
    key_points: ['Uno'],
    action_items: ['Chiamare il cliente', 'Mandare il verbale'],
  });
  assert.equal(
    summaryPlainText(withActions),
    'Vecchia sintesi\n\n- Uno\n\n- Chiamare il cliente\n- Mandare il verbale',
    'the actions the Sintesi tab shows for a meeting from before the versions are copied too',
  );
  assert.equal(summaryPlainText(meeting(5)), '');
});

test('usable_hits_drops_notes', () => {
  const hit = (kind, over = {}) => ({
    meeting_id: 1,
    title: 't',
    kind,
    text: 'x',
    at_ms: null,
    version: null,
    segment_id: null,
    ...over,
  });
  const hits = [hit('transcript'), hit('notes'), hit('summary'), hit('note'), hit('title')];
  assert.deepEqual(
    usableHits(hits).map((h) => h.kind),
    ['transcript', 'summary', 'title'],
  );
  assert.deepEqual(usableHits([]), []);
});

test('hit_target_opens_the_transcript_only_for_transcript_hits', () => {
  const hit = (kind, segment_id = null) => ({
    meeting_id: 1,
    title: 't',
    kind,
    text: 'x',
    at_ms: 1000,
    version: 'v',
    segment_id,
  });
  assert.deepEqual(hitTarget(hit('transcript', 'seg-4')), {
    tab: 'transcript',
    segmentId: 'seg-4',
  });
  assert.deepEqual(hitTarget(hit('transcript')), { tab: 'transcript', segmentId: null });
  assert.deepEqual(hitTarget(hit('summary', 'seg-4')), { tab: 'summary', segmentId: null });
  assert.deepEqual(hitTarget(hit('decision')), { tab: 'summary', segmentId: null });
  assert.deepEqual(hitTarget(hit('title')), { tab: 'summary', segmentId: null });
});

test('pick_devices_prefers_the_configured_device_then_the_default', () => {
  const device = (id, kind, is_default = false) => ({ id, name: id, kind, is_default });
  const devices = [
    device('mic-a', 'microphone'),
    device('mic-b', 'microphone', true),
    device('in-c', 'input'),
    device('out-a', 'output'),
    device('sys-b', 'system', true),
  ];
  const none = { meeting_microphone: '', meeting_system_audio: '' };
  assert.deepEqual(pickDevices(devices, none), { microphone: 'mic-b', system: 'sys-b' });
  assert.deepEqual(
    pickDevices(devices, { meeting_microphone: 'in-c', meeting_system_audio: 'out-a' }),
    { microphone: 'in-c', system: 'out-a' },
    'a configured device that is present wins',
  );
  assert.deepEqual(
    pickDevices(devices, { meeting_microphone: 'gone', meeting_system_audio: 'gone' }),
    { microphone: 'mic-b', system: 'sys-b' },
    'a configured device that is unplugged falls back to the default',
  );
  assert.deepEqual(
    pickDevices(devices, { meeting_microphone: 'out-a', meeting_system_audio: 'mic-a' }),
    { microphone: 'mic-b', system: 'sys-b' },
    'a device of the wrong kind is not accepted',
  );
  // First microphone when none is the default; no default output means no system audio.
  assert.deepEqual(
    pickDevices([device('m1', 'microphone'), device('m2', 'input'), device('o1', 'output')], none),
    { microphone: 'm1', system: null },
  );
  assert.deepEqual(pickDevices([], none), { microphone: null, system: null });
  assert.deepEqual(pickDevices([device('k', 'camera', true)], none), {
    microphone: null,
    system: null,
  });
});

// Task 11: the capture bus. MeetingRecordingIndicator is the only poller of meeting_capture_status;
// it publishes each snapshot here and the Meetings page listens (src/lib/capture-bus.ts).
const recording = (over = {}) => ({
  ...emptyCapture(),
  meeting_id: 7,
  state: 'recording',
  elapsed_ms: 4000,
  ...over,
});

test('last_capture_defaults_to_idle', async () => {
  // A private copy of the module: the other tests publish into the shared one.
  const fresh = await import('../src/lib/capture-bus.ts?default');
  assert.notEqual(fresh.publishCapture, publishCapture, 'the copy is a separate instance');
  assert.deepEqual(fresh.lastCapture(), emptyCapture());
  assert.equal(fresh.lastCapture().state, 'idle');
  assert.equal(fresh.lastCapture().meeting_id, null);
});

test('capture_bus_delivers_to_subscribers_and_stops_after_unsubscribe', () => {
  const first = [];
  const second = [];
  const offFirst = subscribeCapture((s) => first.push(s));
  const offSecond = subscribeCapture((s) => second.push(s));

  publishCapture(recording({ elapsed_ms: 1000 }));
  assert.deepEqual(first.map((s) => s.elapsed_ms), [1000]);
  assert.deepEqual(second.map((s) => s.elapsed_ms), [1000]);
  assert.equal(lastCapture().elapsed_ms, 1000, 'the last snapshot is kept for late readers');

  offFirst();
  publishCapture(recording({ elapsed_ms: 2000 }));
  assert.deepEqual(first.map((s) => s.elapsed_ms), [1000], 'an unsubscribed listener hears nothing');
  assert.deepEqual(second.map((s) => s.elapsed_ms), [1000, 2000]);
  assert.equal(lastCapture().elapsed_ms, 2000);

  offFirst(); // a second call is harmless
  offSecond();
  publishCapture(recording({ elapsed_ms: 3000 }));
  assert.equal(second.length, 2);
  assert.equal(lastCapture().state, 'recording');
  assert.equal(lastCapture().elapsed_ms, 3000, 'the last snapshot is kept without listeners');
});

test('capture_bus_hands_out_copies_so_a_listener_cannot_change_what_others_read', () => {
  const mine = [];
  const off = subscribeCapture((s) => {
    mine.push(s);
    s.state = 'idle'; // a listener that scribbles on its copy
    s.chunks.push({ source: 'x' });
  });
  const published = recording({ state: 'paused' });
  publishCapture(published);
  off();
  assert.equal(published.state, 'paused', 'the publisher object is not shared');
  assert.deepEqual(published.chunks, []);
  assert.equal(lastCapture().state, 'paused');
  assert.deepEqual(lastCapture().chunks, []);
  const read = lastCapture();
  read.state = 'saved';
  assert.equal(lastCapture().state, 'paused', 'a reader gets its own copy too');
});

test('capture_bus_keeps_delivering_when_a_listener_throws', () => {
  const heard = [];
  const originalError = console.error;
  const logged = [];
  console.error = (...args) => logged.push(args);
  const offBad = subscribeCapture(() => {
    throw new Error('listener bug');
  });
  const offGood = subscribeCapture((s) => heard.push(s.elapsed_ms));
  try {
    assert.doesNotThrow(() => publishCapture(recording({ elapsed_ms: 5000 })));
  } finally {
    console.error = originalError;
    offBad();
    offGood();
  }
  assert.deepEqual(heard, [5000], 'the other listener still hears it');
  assert.equal(logged.length, 1, 'the bug is reported, not swallowed');
  assert.equal(lastCapture().elapsed_ms, 5000);
});
