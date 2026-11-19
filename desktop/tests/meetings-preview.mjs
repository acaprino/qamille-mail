// Task 11: the preview backend of the simple Meetings page (previewMeeting in src/lib/meetings.ts).
// It imitates what the real backend does after a recording: transcribe by itself, summarize if an
// assistant is set up, delete the audio once the text is safe. Synthetic data only.
// Playwright on the browser preview: set QAMILLE_PREVIEW_URL, without it the tests are skipped.
// meetings.ts imports Tauri, the sample data and the i18n runes, so plain node cannot load it.
//
// The preview moves jobs by the clock, not by timers, so these tests freeze `Date.now` with
// page.clock.setFixedTime and move it by hand: no waiting, and a test of "no hidden timer".
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { chromium } from '@playwright/test';

const previewUrl = process.env.QAMILLE_PREVIEW_URL;
const skip = !previewUrl ? { skip: 'set QAMILLE_PREVIEW_URL to run the preview steps' } : {};

const T0 = Date.UTC(2026, 10, 9, 10, 0, 0);
const WEEKLY_SYNC = 19; // a sample meeting from before the versions: plain text, one audio chunk, no gaps
const PRICING_REVIEW = 18; // the other: a complete transcript version with text, one clean audio chunk
const BLOCKED = 'Installa il modello di trascrizione in Impostazioni.';

let browser;
before(async () => {
  if (previewUrl) browser = await chromium.launch({ channel: 'msedge', headless: true });
});
after(async () => {
  await browser?.close();
});

// A page of its own per test: the preview keeps its state in the module.
async function withApi(run) {
  const context = await browser.newContext({
    locale: 'it-IT',
    viewport: { width: 1280, height: 800 },
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  try {
    await page.goto(previewUrl);
    await page.evaluate(() => import('/src/lib/meetings.ts'));
    await page.clock.setFixedTime(T0);
    const call = (command, args = {}) =>
      page.evaluate(
        async ({ command, args }) => {
          const api = await import('/src/lib/meetings.ts');
          if (!api.meetingPreview) throw new Error('Synthetic preview required.');
          return api.meetingCall(command, args);
        },
        { command, args },
      );
    // The error a command is refused with, as the page would see it.
    const refused = async (command, args = {}) => {
      const outcome = await page.evaluate(
        async ({ command, args }) => {
          const api = await import('/src/lib/meetings.ts');
          try {
            await api.meetingCall(command, args);
            return null;
          } catch (error) {
            return { message: error.message, code: error.code };
          }
        },
        { command, args },
      );
      assert.ok(outcome, `${command} should have been refused`);
      return outcome;
    };
    const at = (ms) => page.clock.setFixedTime(T0 + ms);
    const read = (id) => call('meeting_read', { id });
    // Records for five seconds and stops at T0 + 5000; returns the id and the instant of the stop.
    const record = async (title = 'Prova') => {
      await at(0);
      const detail = await call('meeting_start', {
        request: {
          request_id: `req-${Math.random()}`,
          title,
          microphone: 'demo-mic',
          system: 'demo-system',
        },
      });
      await at(5000);
      await call('meeting_stop');
      return { id: detail.meeting.id, stop: 5000 };
    };
    const job = (detail, kind) => [...detail.jobs].reverse().find((j) => j.kind === kind);
    await run({ page, call, refused, at, read, record, job });
    assert.deepEqual(errors, [], `No page error: ${errors}`);
  } finally {
    await context.close();
  }
}

test('readiness_defaults_to_ready_and_the_test_hook_changes_it', skip, () =>
  withApi(async ({ call }) => {
    assert.deepEqual(await call('meeting_readiness'), {
      transcription_blocker: null,
      summary_ready: true,
    });
    await call('meeting_preview_readiness', {
      transcription_blocker: BLOCKED,
      summary_ready: false,
    });
    assert.deepEqual(await call('meeting_readiness'), {
      transcription_blocker: BLOCKED,
      summary_ready: false,
    });
    // A field left out stays as it is; null lifts the blocker.
    await call('meeting_preview_readiness', { transcription_blocker: null });
    assert.deepEqual(await call('meeting_readiness'), {
      transcription_blocker: null,
      summary_ready: false,
    });
  }),
);

test('stop_runs_transcribe_then_summarize_and_deletes_the_audio', skip, () =>
  withApi(async ({ call, at, read, record, job }) => {
    const { id, stop } = await record('Riunione di prova');

    // Queued first, still holding the audio, with nothing to read yet.
    await at(stop + 100);
    let detail = await read(id);
    const transcribe = job(detail, 'transcribe');
    assert.equal(transcribe.state, 'queued');
    assert.equal(transcribe.progress, 0);
    assert.equal(detail.has_audio, true);
    assert.equal(detail.has_text, false);
    assert.equal(detail.audio_removed, false);
    assert.equal(detail.audio_kept, null);
    assert.equal(detail.capture.state, 'saved');
    assert.equal(detail.capture.chunks.length, 1);

    // Running, with progress that climbs in steps and never reaches 100 before it completes.
    const seen = [];
    for (const ms of [1000, 2000, 2900]) {
      await at(stop + ms);
      detail = await read(id);
      const running = job(detail, 'transcribe');
      assert.equal(running.state, 'running', `running at +${ms} ms`);
      assert.ok(Number.isInteger(running.progress), 'progress is a whole number');
      assert.ok(
        running.progress > 0 && running.progress < 100,
        `progress ${running.progress} at +${ms} ms`,
      );
      assert.equal(running.id, transcribe.id, 'one job, not a new one each time');
      seen.push(running.progress);
    }
    assert.deepEqual(
      seen,
      [...seen].sort((a, b) => a - b),
      'progress never goes back',
    );
    assert.ok(new Set(seen).size > 1, 'progress moves');
    assert.equal(detail.has_text, false, 'no text until it completes');

    // About three seconds later the transcript is saved, the audio is gone, the summary has started.
    await at(stop + 3100);
    detail = await read(id);
    assert.equal(job(detail, 'transcribe').state, 'completed');
    assert.equal(job(detail, 'transcribe').progress, 100);
    assert.equal(detail.transcripts.length, 1);
    assert.equal(detail.transcripts[0].complete, true);
    assert.ok(
      detail.transcripts[0].segments.some((s) => s.text.trim()),
      'the transcript has text',
    );
    assert.equal(detail.active_transcript, detail.transcripts[0].id);
    assert.equal(detail.has_text, true);
    assert.equal(detail.audio_removed, true);
    assert.equal(detail.audio_kept, null);
    assert.equal(detail.has_audio, false);
    assert.deepEqual(detail.capture.chunks, []);
    assert.equal(detail.capture.state, 'saved');
    const summarize = job(detail, 'summarize');
    assert.ok(summarize, 'the summary follows');
    assert.ok(['queued', 'running'].includes(summarize.state));
    assert.equal(detail.active_summary, null);

    // Then the summary is there too, and nothing else was queued.
    await at(stop + 3000 + 1000);
    assert.equal(job(await read(id), 'summarize').state, 'running');
    await at(stop + 6000);
    detail = await read(id);
    assert.equal(job(detail, 'summarize').state, 'completed');
    assert.ok(detail.active_summary);
    assert.ok(detail.summaries[0].summary.trim());
    assert.ok(detail.meeting.summary.trim());
    assert.deepEqual(
      detail.jobs.map((j) => j.kind),
      ['transcribe', 'summarize'],
    );

    // Later reads change nothing, and the audio stays gone (the live capture state is copied into
    // the meeting on every command and must not bring the chunks back).
    await at(stop + 60_000);
    detail = await read(id);
    assert.deepEqual(
      detail.jobs.map((j) => j.state),
      ['completed', 'completed'],
    );
    assert.equal(detail.audio_removed, true);
    assert.equal(detail.has_audio, false);
    assert.deepEqual(detail.capture.chunks, []);
    assert.deepEqual((await call('meeting_capture_status')).chunks, []);
  }),
);

test('jobs_follow_the_clock_not_a_timer', skip, () =>
  withApi(async ({ page, at, read, record, job }) => {
    const { id, stop } = await record();
    await at(stop + 100);
    assert.equal(job(await read(id), 'transcribe').state, 'queued');
    // The clock is frozen: a timer would have fired by now, the clock-driven job has not moved.
    await page.waitForTimeout(1300);
    const frozen = job(await read(id), 'transcribe');
    assert.equal(frozen.state, 'queued');
    assert.equal(frozen.progress, 0);
    // Moving the clock moves it, however long the page was not asking: it catches up in one read.
    await at(stop + 120_000);
    const detail = await read(id);
    assert.deepEqual(
      detail.jobs.map((j) => j.state),
      ['completed', 'completed'],
    );
    assert.equal(detail.audio_removed, true);
  }),
);

test('the_list_page_carries_the_flags_and_the_job_progress', skip, () =>
  withApi(async ({ call, at, record, job }) => {
    const { id, stop } = await record();
    await at(stop + 2000);
    const page = await call('meeting_list_page', { offset: 0, limit: 50 });
    const row = page.items.find((item) => item.meeting.id === id);
    assert.equal(job(row, 'transcribe').state, 'running');
    assert.equal(row.has_audio, true, 'a list row drops the chunks but keeps the flag');
    assert.deepEqual(row.capture.chunks, []);
    assert.equal(row.has_text, false);
    await at(stop + 3500);
    const later = (await call('meeting_list_page', { offset: 0, limit: 50 })).items.find(
      (item) => item.meeting.id === id,
    );
    assert.equal(later.has_text, true);
    assert.equal(later.has_audio, false);
    assert.equal(later.audio_removed, true);
    assert.equal(later.meeting.transcript, '', 'list rows still carry no text');
  }),
);

test('flags_are_computed_from_the_data', skip, () =>
  withApi(async ({ call, at, read }) => {
    const weekly = await read(WEEKLY_SYNC);
    assert.equal(weekly.has_audio, true, 'a chunk is audio');
    assert.equal(weekly.has_text, true, 'the plain text of an old meeting counts');

    await at(0);
    const started = await call('meeting_start', {
      request: { request_id: 'flags', title: 'Flags', microphone: 'demo-mic' },
    });
    assert.equal(started.has_audio, false, 'nothing is on disk while it starts');
    assert.equal(started.has_text, false);
    await at(2000);
    assert.equal((await read(started.meeting.id)).has_audio, false);

    const imported = await call('meeting_import', { path: 'synthetic.wav' });
    assert.equal(imported.has_audio, true, 'an imported file is audio');
    assert.equal(imported.has_text, false);
  }),
);

test('a_blocked_engine_queues_nothing_and_the_default_command_says_why', skip, () =>
  withApi(async ({ call, refused, at, read, record, job }) => {
    await call('meeting_preview_readiness', { transcription_blocker: BLOCKED });
    const { id, stop } = await record();
    await at(stop + 10_000);
    const detail = await read(id);
    assert.deepEqual(detail.jobs, [], 'no job was queued');
    assert.equal(detail.has_audio, true);
    assert.equal(detail.has_text, false);
    assert.equal(detail.audio_removed, false);

    const error = await refused('meeting_transcribe_default', { id });
    assert.equal(error.message, BLOCKED);
    assert.deepEqual((await read(id)).jobs, []);

    // Once it can, the button works: the same chain, one click.
    await call('meeting_preview_readiness', { transcription_blocker: null });
    const queued = await call('meeting_transcribe_default', { id });
    assert.equal(job(queued, 'transcribe').state, 'queued');
    assert.equal((await refused('meeting_transcribe_default', { id })).code, 'job_running');
    await at(stop + 10_000 + 3100);
    const done = await read(id);
    assert.equal(done.has_text, true);
    assert.equal(done.audio_removed, true);
    assert.ok(job(done, 'summarize'));
  }),
);

test(
  'without_an_assistant_the_chain_stops_after_the_transcript_and_summarize_default_fails_with_a_reason',
  skip,
  () =>
    withApi(async ({ call, at, read, record, job }) => {
      await call('meeting_preview_readiness', { summary_ready: false });
      const { id, stop } = await record();
      await at(stop + 10_000);
      let detail = await read(id);
      assert.deepEqual(
        detail.jobs.map((j) => j.kind),
        ['transcribe'],
      );
      assert.equal(detail.has_text, true);
      assert.equal(detail.audio_removed, true);
      assert.equal(detail.active_summary, null);

      // The command does not refuse: the job is queued and ends `failed` with the reason, which is
      // what the page shows (with "Riprova").
      const queued = await call('meeting_summarize_default', { id });
      assert.equal(job(queued, 'summarize').state, 'queued');
      await at(stop + 10_000 + 3000);
      detail = await read(id);
      const failed = job(detail, 'summarize');
      assert.equal(failed.state, 'failed');
      assert.equal(failed.error, 'Configura un assistente in Impostazioni per avere la sintesi.');
      assert.equal(detail.active_summary, null);

      // Setting the assistant up and retrying that job produces it.
      await call('meeting_preview_readiness', { summary_ready: true });
      const retried = await call('meeting_retry_job', { id, jobId: failed.id });
      assert.equal(job(retried, 'summarize').state, 'queued');
      await at(stop + 10_000 + 3000 + 3000);
      detail = await read(id);
      assert.equal(job(detail, 'summarize').state, 'completed');
      assert.ok(detail.active_summary);
    }),
);

test('summarize_default_is_refused_while_recording_and_without_any_text', skip, () =>
  withApi(async ({ call, refused, at, read }) => {
    // The engine is blocked, so stopping leaves audio with no transcript and nothing to summarize.
    await call('meeting_preview_readiness', { transcription_blocker: BLOCKED });
    await at(0);
    const started = await call('meeting_start', {
      request: { request_id: 'busy-1', title: 'In corso', microphone: 'demo-mic', system: 'demo-system' },
    });
    const id = started.meeting.id;
    const busy = await refused('meeting_summarize_default', { id });
    assert.equal(busy.code, 'recording_busy');
    await at(2000);
    await call('meeting_stop');
    const nothing = await refused('meeting_summarize_default', { id });
    assert.equal(nothing.code, 'validation');
    assert.deepEqual((await read(id)).jobs, []);
  }),
);

test('summarize_default_accepts_a_transcript_that_came_out_empty', skip, () =>
  withApi(async ({ call, at, read, record, job }) => {
    await call('meeting_preview_empty_transcript');
    const { id, stop } = await record('Muta');
    await at(stop + 3100);
    assert.equal((await read(id)).has_text, false);
    const queued = await call('meeting_summarize_default', { id });
    assert.equal(job(queued, 'summarize').state, 'queued');
  }),
);

test('the_long_transcript_hook_gives_the_next_transcription_forty_timed_segments', skip, () =>
  withApi(async ({ call, at, read, record }) => {
    await call('meeting_preview_long_transcript');
    const first = await record('Lunga');
    await at(first.stop + 3100);
    const segments = (await read(first.id)).transcripts[0].segments;
    assert.equal(segments.length, 40);
    assert.equal(segments[0].id, 'segment-1');
    assert.equal(segments[0].start_ms, 0);
    assert.equal(segments[30].id, 'segment-31');
    assert.equal(segments[30].start_ms, 600_000);
    assert.match(segments[30].text, /argomento 31\b/);
    // The hook is for one transcription only.
    const second = await record('Normale');
    await at(second.stop + 3100);
    assert.equal((await read(second.id)).transcripts[0].segments.length, 1);
  }),
);

test('an_empty_transcript_keeps_the_audio_until_it_is_discarded', skip, () =>
  withApi(async ({ call, refused, at, read, record, job }) => {
    await call('meeting_preview_empty_transcript');
    const first = await record('Silenziosa');
    await at(first.stop + 3100);
    let detail = await read(first.id);
    assert.equal(job(detail, 'transcribe').state, 'completed');
    assert.equal(detail.transcripts.length, 1);
    assert.equal(detail.transcripts[0].complete, true);
    assert.ok(!detail.transcripts[0].segments.some((s) => s.text.trim()), 'no word in it');
    assert.equal(detail.audio_kept, 'empty_transcript');
    assert.equal(detail.audio_removed, false);
    assert.equal(detail.has_audio, true, 'the audio stays');
    assert.equal(detail.has_text, false);
    assert.equal(detail.capture.chunks.length, 1);
    await at(first.stop + 20_000);
    assert.deepEqual(
      (await read(first.id)).jobs.map((j) => j.kind),
      ['transcribe'],
      'nothing to summarize',
    );

    // The audio can be deleted by hand, once.
    const discarded = await call('meeting_discard_audio', { id: first.id });
    assert.equal(discarded.audio_removed, true);
    assert.equal(discarded.audio_kept, null);
    assert.equal(discarded.has_audio, false);
    assert.deepEqual(discarded.capture.chunks, []);
    assert.equal((await read(first.id)).has_audio, false);
    const again = await refused('meeting_discard_audio', { id: first.id });
    assert.equal(again.message, "Non c'è audio da eliminare.");

    // The hook was for one transcription: the next recording is normal.
    const second = await record('Normale');
    await at(second.stop + 3100);
    detail = await read(second.id);
    assert.equal(detail.has_text, true);
    assert.equal(detail.audio_removed, true);
    assert.equal(detail.audio_kept, null);
  }),
);

test('a_recording_with_gaps_keeps_its_audio_after_recovery', skip, () =>
  withApi(async ({ call, at, read, job }) => {
    // The blocker lets the audio exist without a transcript, as after a crash before any job.
    await call('meeting_preview_readiness', { transcription_blocker: BLOCKED });
    const imported = await call('meeting_import', { path: 'synthetic.wav' });
    const id = imported.meeting.id;
    await call('meeting_preview_interrupt', { id });
    assert.equal((await read(id)).capture.state, 'interrupted');

    // Recovering with the engine still blocked queues nothing.
    await call('meeting_recover', { id });
    assert.deepEqual((await read(id)).jobs, []);

    // Recovering when it can: the transcription starts by itself, as after a stop.
    await call('meeting_preview_interrupt', { id });
    await call('meeting_preview_readiness', { transcription_blocker: null });
    await call('meeting_recover', { id });
    assert.equal(job(await read(id), 'transcribe').state, 'queued');
    await at(3100);
    const detail = await read(id);
    assert.equal(detail.has_text, true);
    assert.equal(detail.audio_kept, 'interrupted_capture', 'the gaps are why the audio stays');
    assert.equal(detail.audio_removed, false);
    assert.equal(detail.has_audio, true);
    assert.ok(job(detail, 'summarize'), 'the text is summarized all the same');
  }),
);

test('discarding_kept_audio_clears_the_gap_flags_and_does_not_change_the_revision', skip, () =>
  withApi(async ({ call, at, read }) => {
    await call('meeting_preview_readiness', { transcription_blocker: BLOCKED });
    const imported = await call('meeting_import', { path: 'synthetic.wav' });
    const id = imported.meeting.id;
    await call('meeting_preview_interrupt', { id });
    await call('meeting_preview_readiness', { transcription_blocker: null });
    await call('meeting_recover', { id });
    await at(6000); // the transcription and the summary that follows it are over
    let detail = await read(id);
    assert.equal(detail.audio_kept, 'interrupted_capture');
    // The recording that was kept for its gaps still carries all of them.
    assert.ok(detail.capture.gaps.length > 0);
    assert.ok(detail.capture.missing_chunks.length > 0);
    assert.ok(detail.capture.warnings.length > 0);
    assert.equal(detail.capture.incomplete, true);
    const revision = detail.revision;

    const discarded = await call('meeting_discard_audio', { id });
    assert.equal(discarded.revision, revision, 'removing the audio is not an edit of the meeting');
    detail = await read(id);
    assert.equal(detail.revision, revision);
    for (const capture of [detail.capture, await call('meeting_capture_status')]) {
      assert.deepEqual(capture.chunks, []);
      assert.deepEqual(capture.missing_chunks, []);
      assert.deepEqual(capture.unverified_chunks, []);
      assert.deepEqual(capture.gaps, []);
      assert.deepEqual(capture.warnings, []);
      assert.equal(capture.incomplete, false);
      assert.equal(capture.error, null);
    }
  }),
);

test('cancelling_the_transcription_stops_the_chain_and_keeps_the_audio', skip, () =>
  withApi(async ({ call, at, read, record, job }) => {
    const { id, stop } = await record();
    await at(stop + 1000);
    const running = job(await read(id), 'transcribe');
    assert.equal(running.state, 'running');
    await call('meeting_cancel_job', { id, jobId: running.id });
    await at(stop + 60_000);
    const detail = await read(id);
    assert.deepEqual(
      detail.jobs.map((j) => [j.kind, j.state]),
      [['transcribe', 'cancelled']],
    );
    assert.equal(detail.transcripts.length, 0);
    assert.equal(detail.has_audio, true);
    assert.equal(detail.audio_removed, false);
    assert.equal(detail.has_text, false);
  }),
);

test('after_the_audio_is_deleted_a_new_transcription_is_refused', skip, () =>
  withApi(async ({ call, refused, at, read, record, job }) => {
    const { id, stop } = await record();
    await at(stop + 10_000);
    const detail = await read(id);
    assert.equal(detail.audio_removed, true);
    const message = "L'audio di questa riunione è stato eliminato dopo la trascrizione.";
    const byDefault = await refused('meeting_transcribe_default', { id });
    assert.equal(byDefault.message, message);
    assert.equal(byDefault.code, 'audio_removed');
    const retried = await refused('meeting_retry_job', { id, jobId: job(detail, 'transcribe').id });
    // Like the service, a retry looks at the job first: a finished one is not tried again at all.
    assert.equal(retried.code, 'validation');
    assert.deepEqual(
      (await read(id)).jobs.map((j) => j.state),
      ['completed', 'completed'],
      'nothing was queued',
    );
  }),
);

test('a_failed_or_cancelled_summary_can_be_retried_by_its_job', skip, () =>
  withApi(async ({ call, at, read, record, job }) => {
    const { id, stop } = await record();
    await at(stop + 3000 + 600);
    const summarize = job(await read(id), 'summarize');
    await call('meeting_cancel_job', { id, jobId: summarize.id });
    await at(stop + 20_000);
    assert.equal(job(await read(id), 'summarize').state, 'cancelled');
    const retried = await call('meeting_retry_job', { id, jobId: summarize.id });
    const again = job(retried, 'summarize');
    assert.notEqual(again.id, summarize.id);
    assert.equal(again.state, 'queued');
    await at(stop + 20_000 + 2000);
    const detail = await read(id);
    assert.equal(job(detail, 'summarize').state, 'completed');
    assert.ok(detail.active_summary);
  }),
);

test('the_delay_hook_holds_every_answer_back_but_not_the_hooks_themselves', skip, () =>
  withApi(async ({ page, call }) => {
    // Time in the page itself: the tests freeze Date.now, performance.now keeps going.
    const timed = (command, args = {}) =>
      page.evaluate(
        async ({ command, args }) => {
          const api = await import('/src/lib/meetings.ts');
          const started = performance.now();
          await api.meetingCall(command, args);
          return performance.now() - started;
        },
        { command, args },
      );
    assert.ok((await timed('meeting_readiness')) < 150, 'no delay by default');
    assert.ok((await timed('meeting_preview_delay', { ms: 400 })) < 150, 'a hook answers at once');
    assert.ok((await timed('meeting_readiness')) >= 390, 'a command waits');
    assert.ok((await timed('meeting_read', { id: WEEKLY_SYNC })) >= 390, 'so does a read');
    await call('meeting_preview_delay', { ms: 0 });
    assert.ok((await timed('meeting_readiness')) < 150, 'zero puts it right');
  }),
);

test('discard_audio_is_refused_when_nothing_was_kept', skip, () =>
  withApi(async ({ refused, read }) => {
    const before = await read(WEEKLY_SYNC);
    const error = await refused('meeting_discard_audio', { id: WEEKLY_SYNC });
    assert.equal(error.message, "Non c'è audio da eliminare.");
    const after = await read(WEEKLY_SYNC);
    assert.equal(after.audio_removed, false);
    assert.equal(after.has_audio, before.has_audio);
  }),
);

test('the_one_time_cleanup_counts_then_deletes_the_audio_of_transcribed_meetings', skip, () =>
  withApi(async ({ call, at, read, record }) => {
    // The sample "Pricing review" has a complete transcript version with text and a clean chunk of
    // 900 s (16 kHz, 16 bit, mono). Weekly sync has only the plain text of a meeting from before the
    // versions: the real rule needs a transcript VERSION, so its audio is not taken.
    const expected = { meetings: 1, bytes: 900 * 32_000 };
    assert.deepEqual(await call('meeting_audio_cleanup_preview'), expected);
    assert.deepEqual(
      await call('meeting_audio_cleanup_preview'),
      expected,
      'looking changes nothing',
    );
    assert.equal((await read(WEEKLY_SYNC)).audio_removed, false);
    assert.equal((await read(PRICING_REVIEW)).audio_removed, false);

    // A meeting whose audio was kept (empty transcript) is not counted.
    await call('meeting_preview_empty_transcript');
    const silent = await record('Silenziosa');
    await at(silent.stop + 3100);
    assert.equal((await read(silent.id)).audio_kept, 'empty_transcript');
    assert.deepEqual(await call('meeting_audio_cleanup_preview'), expected);

    assert.deepEqual(await call('meeting_audio_cleanup'), expected);
    const pricing = await read(PRICING_REVIEW);
    assert.equal(pricing.audio_removed, true);
    assert.equal(pricing.has_audio, false);
    assert.deepEqual(pricing.capture.chunks, []);
    assert.equal(pricing.has_text, true, 'the text stays');
    assert.equal(pricing.meeting.duration_secs, 900, 'the length stays');
    const weekly = await read(WEEKLY_SYNC);
    assert.equal(weekly.audio_removed, false, 'plain text from before the versions is not a transcript');
    assert.equal(weekly.has_audio, true);
    const kept = await read(silent.id);
    assert.equal(kept.has_audio, true, 'a kept audio is only deleted by hand');
    assert.deepEqual(await call('meeting_audio_cleanup_preview'), { meetings: 0, bytes: 0 });
    assert.deepEqual(await call('meeting_audio_cleanup'), { meetings: 0, bytes: 0 });
  }),
);

test('the_recording_indicator_publishes_every_status_it_reads', skip, () =>
  withApi(async ({ page, call }) => {
    // The indicator polls meeting_capture_status every second and is the page's only source.
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      window.__heard = [];
      api.subscribeCapture((snapshot) =>
        window.__heard.push([snapshot.state, snapshot.meeting_id]),
      );
    });
    const started = await call('meeting_start', {
      request: { request_id: 'bus', title: 'Bus', microphone: 'demo-mic', system: 'demo-system' },
    });
    const id = started.meeting.id;
    await page.waitForFunction(
      (id) => window.__heard.some(([s, m]) => s === 'recording' && m === id),
      id,
      {
        timeout: 5000,
      },
    );
    const live = await page.evaluate(async () =>
      (await import('/src/lib/meetings.ts')).lastCapture(),
    );
    assert.equal(live.state, 'recording');
    assert.equal(live.meeting_id, id);

    await call('meeting_stop');
    await page.waitForFunction(() => window.__heard.at(-1)?.[0] === 'saved', null, {
      timeout: 5000,
    });
  }),
);

// ─── Final review: the mock does what the service does ─────────────────────────────────────────

test('a_retry_runs_with_the_options_of_its_job_and_a_new_transcription_with_the_settings_of_now', skip, () =>
  withApi(async ({ call, refused, at, read, record, job }) => {
    const { id, stop } = await record();
    await at(stop + 1000);
    const first = job(await read(id), 'transcribe');
    assert.deepEqual(first.options, { provider: 'compute', model: 'base', language: 'it' });
    assert.equal(first.language, 'it');
    await call('meeting_cancel_job', { id, jobId: first.id });

    // Settings is fixed afterwards (another model, another language).
    const now = await call('meeting_preview_transcribe_options', { model: 'small', language: 'en' });
    assert.deepEqual(now, { provider: 'compute', model: 'small', language: 'en' });

    // The job run again keeps what it started with: the fix does not reach it.
    const retried = job(await call('meeting_retry_job', { id, jobId: first.id }), 'transcribe');
    assert.notEqual(retried.id, first.id);
    assert.deepEqual(retried.options, first.options, 'a retry reuses the stored options');
    await call('meeting_cancel_job', { id, jobId: retried.id });

    // A new transcription reads Settings again.
    const fresh = job(await call('meeting_transcribe_default', { id }), 'transcribe');
    assert.deepEqual(fresh.options, { provider: 'compute', model: 'small', language: 'en' });
    assert.equal(fresh.language, 'en');
    await at(stop + 1000 + 4000);
    const detail = await read(id);
    const version = detail.transcripts.find((item) => item.id === detail.active_transcript);
    assert.equal(version.model, 'small', 'the transcript says what made it');
    assert.equal(version.language, 'en');
    assert.deepEqual(
      detail.jobs.filter((j) => j.kind === 'transcribe').map((j) => j.options.model),
      ['base', 'base', 'small'],
    );

    // Only a job that failed, was interrupted or cancelled can be tried again, and only if it exists.
    const done = job(detail, 'transcribe');
    assert.equal(done.state, 'completed');
    assert.equal((await refused('meeting_retry_job', { id, jobId: done.id })).code, 'validation');
    assert.equal((await refused('meeting_retry_job', { id, jobId: 'nope' })).code, 'not_found');
  }),
);

test('renaming_a_meeting_whose_event_is_gone_works_and_a_new_link_needs_an_event', skip, () =>
  withApi(async ({ page, call, refused, at, read }) => {
    const app = (command, args = {}) =>
      page.evaluate(
        async ({ command, args }) => (await import('/src/lib/api.ts')).call(command, args),
        { command, args },
      );
    const event = await app('save_event', {
      event: {
        id: 0,
        title: 'Revisione',
        start: Math.floor(T0 / 1000),
        end: Math.floor(T0 / 1000) + 3600,
        location: '',
        attendees: [],
        meeting_link: null,
        conference: '',
        rsvp: 'None',
        notes: '',
        color: [47, 111, 237],
      },
    });
    await at(0);
    const started = await call('meeting_start', {
      request: {
        request_id: 'event-gone',
        title: 'Revisione',
        event_id: event.id,
        microphone: 'demo-mic',
        system: 'demo-system',
      },
    });
    await at(5000);
    await call('meeting_stop');
    const id = started.meeting.id;
    assert.equal((await read(id)).meeting.event_id, event.id);

    // The event is deleted from the calendar. The meeting keeps its id for it.
    await app('delete_event', { id: event.id });
    const rename = (over) =>
      call('meeting_update', {
        id,
        title: 'Revisione di novembre',
        eventId: event.id,
        includedInSearch: true,
        ...over,
      });
    const renamed = await rename({ expectedRevision: (await read(id)).revision });
    assert.equal(renamed.meeting.title, 'Revisione di novembre');
    assert.equal(renamed.meeting.event_id, event.id, 'the link is kept as it was');

    // Pointing the meeting at an event that is not there is refused; no link is always fine.
    const missing = await refused('meeting_update', {
      id,
      title: 'Altro',
      eventId: 999_999,
      includedInSearch: true,
    });
    assert.equal(missing.code, 'not_found');
    assert.equal((await read(id)).meeting.title, 'Revisione di novembre', 'nothing changed');
    const unlinked = await rename({ eventId: null });
    assert.equal(unlinked.meeting.event_id, null);
  }),
);

test('a_recovery_does_not_transcribe_again_a_meeting_that_has_a_transcription_or_text', skip, () =>
  withApi(async ({ call, at, read, record, job }) => {
    // A meeting with a transcription job (here finished, with its text) is left alone.
    const { id, stop } = await record();
    await at(stop + 10_000);
    const done = await read(id);
    assert.equal(done.has_text, true);
    const before = done.jobs.length;
    await call('meeting_preview_interrupt', { id });
    await call('meeting_recover', { id });
    await at(stop + 10_000);
    assert.equal((await read(id)).jobs.length, before, 'no transcribe job was queued');

    // Same for one whose transcription was cancelled: the person asks again.
    const second = await record('Annullata');
    await at(second.stop + 1000);
    await call('meeting_cancel_job', { id: second.id, jobId: job(await read(second.id), 'transcribe').id });
    await call('meeting_preview_interrupt', { id: second.id });
    await call('meeting_recover', { id: second.id });
    const jobs = (await read(second.id)).jobs.filter((j) => j.kind === 'transcribe');
    assert.deepEqual(jobs.map((j) => j.state), ['cancelled'], 'the cancelled one stays the only one');

    // A meeting with no job and no text (the engine was blocked at the stop) is transcribed.
    await call('meeting_preview_readiness', { transcription_blocker: BLOCKED });
    const third = await record('Bloccata');
    await at(third.stop + 100);
    assert.equal((await read(third.id)).jobs.length, 0);
    await call('meeting_preview_readiness', { transcription_blocker: null });
    await call('meeting_preview_interrupt', { id: third.id });
    await call('meeting_recover', { id: third.id });
    assert.equal(job(await read(third.id), 'transcribe').state, 'queued');
  }),
);

test('search_matches_every_word_and_finds_the_title_like_the_service', skip, () =>
  withApi(async ({ call }) => {
    const found = (query) => call('meeting_search', { query });
    // The title of a meeting is a hit of its own.
    const title = await found('weekly');
    assert.deepEqual(
      title.map((hit) => [hit.meeting_id, hit.kind]),
      [[WEEKLY_SYNC, 'title']],
    );
    // Every word has to be in the same text, in any order and any case.
    const both = await found('  LAUNCH   plan ');
    assert.deepEqual(
      both.map((hit) => [hit.meeting_id, hit.kind]),
      [
        [WEEKLY_SYNC, 'transcript'],
        [WEEKLY_SYNC, 'summary'],
      ],
      'the plain text of a meeting from before the versions is searched too, and its summary',
    );
    assert.equal(both[0].segment_id, null);
    assert.deepEqual(await found('launch riverbend'), [], 'one word missing: no hit');
    assert.deepEqual(await found('   '), [], 'no words, no hits');

    // The lines of a transcript version: one hit per line that has all the words.
    const lines = await found('pricing copy');
    assert.deepEqual(
      lines.map((hit) => [hit.meeting_id, hit.kind, hit.segment_id]),
      [[PRICING_REVIEW, 'transcript', 'segment-1']],
    );
    assert.equal(lines[0].version, 'sample-transcript');
    // The same word in the title and in the line: both are hits.
    const mixed = await found('pricing');
    assert.deepEqual(
      mixed.map((hit) => hit.kind),
      ['title', 'transcript'],
    );

    // A meeting out of the search gives nothing, whatever it holds.
    await call('meeting_update', {
      id: PRICING_REVIEW,
      title: 'Pricing review',
      eventId: null,
      includedInSearch: false,
    });
    assert.deepEqual(await found('pricing'), []);
  }),
);
