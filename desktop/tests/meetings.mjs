// The simple Meetings page on the browser preview (Playwright, it-IT, 1440x900). Synthetic data
// only: this script never opens a real audio device. Task 12 is the list: groups, "Registra ora"
// and the search. Task 13 is the detail (tabs and every state; each scenario starts from a clean
// page, see `fresh`). The refresh (Task 14) appends its own section.
//
// The preview's clock is fixed (Playwright `page.clock.setFixedTime`), so the dates are the same on
// every day and the jobs of the preview (which are a function of the clock) stand still until the
// test moves the clock on. T0 is Monday 9 November 2026, 10:00, local time.
//
// Task 14 is the refresh: nothing runs by itself on a page at rest, the list is read again when a
// recording starts or stops elsewhere, one 1-second timer follows the meetings being processed, and
// the Assistant and the recording indicator open a meeting from outside. The meeting commands do not
// go through the preview's `__previewCalls`, so the test counts them itself: it adds a line to
// `meetingCall` as the module is served (below), and tells the page's calls from its own.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { spawn } from 'node:child_process';

if (process.env.QAMILLE_CDP)
  throw new Error('Meeting preview tests cannot run against a native window.');
const previewPort = 20000 + Math.floor(Math.random() * 20000);
const previewUrl = process.env.QAMILLE_PREVIEW_URL || `http://127.0.0.1:${previewPort}`;
const server = process.env.QAMILLE_PREVIEW_URL
  ? null
  : spawn(
      process.execPath,
      [
        path.join(import.meta.dirname, '../node_modules/vite/bin/vite.js'),
        '--host',
        '127.0.0.1',
        '--port',
        String(previewPort),
        '--strictPort',
      ],
      { cwd: path.join(import.meta.dirname, '..'), windowsHide: true, stdio: 'pipe' },
    );
let serverOutput = '';
server?.stdout.on('data', (data) => {
  serverOutput += data.toString();
});
server?.stderr.on('data', (data) => {
  serverOutput += data.toString();
});
for (let attempt = 0; attempt < 40; attempt++) {
  if (server && server.exitCode !== null) throw new Error(`Preview exited: ${serverOutput}`);
  try {
    // Strip ANSI decorations before looking for the readiness marker: newer
    // vite colorizes "Local:" even on piped output, and the plain substring
    // would never match.
    const plain = serverOutput.replace(/\u001b\[[0-9;]*m/g, '');
    if ((!server || plain.includes('Local:')) && (await fetch(previewUrl)).ok) break;
  } catch {
    /* The owned preview server is starting. */
  }
  if (attempt === 39) {
    server?.kill();
    throw new Error(`Preview unavailable: ${serverOutput}`);
  }
  await new Promise((resolve) => setTimeout(resolve, 250));
}
const browser = await chromium.launch({ channel: 'msedge', headless: true }).catch((error) => {
  server?.kill();
  throw error;
});
const context = await browser.newContext({
  locale: 'it-IT',
  viewport: { width: 1440, height: 900 },
  acceptDownloads: true,
});
const page = await context.newPage();
page.setDefaultTimeout(15000);
// Every meeting command is logged as `{ command, args, test }` (`test`: the call came from this
// script, see `previewApi`), and `window.__meetingDelay[command]` holds that command's answer back
// by that many milliseconds. The command has already run when the wait starts, like a slow backend:
// an answer held back is OLDER than what the page learns meanwhile.
await page.route(/\/src\/lib\/meetings\.ts(\?.*)?$/, async (route) => {
  const response = await route.fetch();
  const source = await response.text();
  const head = 'export async function meetingCall(command, args = {}) {';
  assert.ok(source.includes(head), 'meetingCall is where the call counter expects it');
  await route.fulfill({
    response,
    body: source.replace(
      head,
      `${head}
  (window.__meetingLog ??= []).push({ command, args, test: Boolean(window.__fromTest) });
  const answer = await __meetingCallInner(command, args);
  const waitMs = window.__meetingDelay?.[command];
  if (waitMs) await new Promise((resolve) => setTimeout(resolve, waitMs));
  return answer;
}
async function __meetingCallInner(command, args = {}) {`,
    ),
  });
});
// The browser is not the desktop, so the list offers no "Partecipa e registra". A scenario that
// needs the button sets `joinOffered` before it loads the page: the list is then served as if it
// were on the desktop (and the join itself fails in the preview, which has no desktop to open it).
let joinOffered = false;
await page.route(/\/src\/components\/meetings\/MeetingList\.tsx(\?t=\d+)?$/, async (route) => {
  if (!joinOffered) return route.continue();
  const response = await route.fetch();
  const source = await response.text();
  const head = 'import { desktop } from "/src/lib/api.ts";';
  assert.ok(source.includes(head), 'the list reads `desktop` where the join switch expects it');
  await route.fulfill({
    response,
    body: source.replace(head, 'import { desktop as __realDesktop } from "/src/lib/api.ts"; const desktop = true;'),
  });
});
const errors = [];
page.on('pageerror', (error) => errors.push(error.stack || error.message));
const steps = [];
// MEETINGS_STEPS=a,b runs only those scenarios (each one starts from a clean page with `fresh`).
const onlySteps = process.env.MEETINGS_STEPS?.split(',');
async function step(name, work) {
  if (onlySteps && !onlySteps.includes(name)) return;
  await work();
  steps.push(name);
  console.log(`PASS ${name}`);
}

// ─── The clock and the data ───────────────────────────────────────────────────────────────────
const MIN = 60;
const HOUR = 3600;
const DAY = 86_400;
const local = (month, day, hour = 0, minute = 0) =>
  Math.floor(new Date(2026, month - 1, day, hour, minute).getTime() / 1000);
const T0 = local(11, 9, 10);
const at = (secondsAfterT0) => new Date((T0 + secondsAfterT0) * 1000);

async function previewApi(command, args = {}) {
  return page.evaluate(
    async ({ command, args }) => {
      const api = await import('/src/lib/meetings.ts');
      if (!api.meetingPreview) throw new Error('Synthetic preview required.');
      // The call counter reads this flag as the call starts: it is the script's, not the page's.
      window.__fromTest = true;
      const result = api.meetingCall(command, args);
      window.__fromTest = false;
      return result;
    },
    { command, args },
  );
}
async function appCall(command, args = {}) {
  return page.evaluate(
    async ({ command, args }) => (await import('/src/lib/api.ts')).call(command, args),
    { command, args },
  );
}
async function saveEvent(event) {
  return appCall('save_event', {
    event: {
      id: 0,
      title: '',
      start: 0,
      end: 0,
      location: '',
      attendees: [],
      meeting_link: null,
      conference: '',
      rsvp: 'None',
      notes: '',
      color: [47, 111, 237],
      ...event,
    },
  });
}
const people = (count) => Array.from({ length: count }, (_, i) => `ospite${i + 1}@example.com`);
const MEET = 'https://meet.google.com/abc-defg-hij';
let nextRequest = 0;
/** Records and stops a meeting (the preview has one capture at a time). Returns the meeting. */
async function record(title, { eventId = null, stop = true } = {}) {
  const detail = await previewApi('meeting_start', {
    request: {
      request_id: `test-${++nextRequest}`,
      title,
      event_id: eventId,
      microphone: 'demo-mic',
      system: 'demo-system',
    },
  });
  if (stop) await previewApi('meeting_stop');
  return detail;
}

// ─── The page ─────────────────────────────────────────────────────────────────────────────────
const workspaceNav = () => page.getByRole('navigation', { name: 'Spazio di lavoro' });
const openMeetings = async () => {
  await workspaceNav().getByRole('button', { name: 'Meeting', exact: true }).click();
  await expect(page.locator('section.meetings-page')).toBeVisible();
  await expect(page.locator('.meetings-page [data-list-ready]')).toBeAttached();
};
const leaveMeetings = () => page.getByRole('button', { name: /^Posta in arrivo\s*\d*$/ }).click();
/** Away and back: the page reads the meetings and the calendar once, when it opens. */
const reopenMeetings = async () => {
  await leaveMeetings();
  await openMeetings();
};
const groupLabels = () => page.locator('.meetings-page .group-label').allTextContents();
const group = (name) => page.locator(`.meetings-page [data-group="${name}"]`);
const rowOf = (title) =>
  page.locator('.meetings-page [data-row-key]').filter({
    has: page.locator('.item-title', { hasText: new RegExp(`^${title}$`) }),
  });
const titlesIn = (name) => group(name).locator('.item-title').allTextContents();
const detailPane = () => page.locator('.meetings-page .detail-pane');
const recordNow = () => page.getByRole('button', { name: 'Registra ora', exact: true });
const searchBox = () => page.getByRole('searchbox', { name: 'Cerca nelle trascrizioni' });

// ─── The detail (Task 13) ─────────────────────────────────────────────────────────────────────
// Each scenario starts from a page of its own (the preview keeps its state in the module, so a
// reload is a clean slate) and builds its meetings through the preview's own pipeline.
async function fresh() {
  await page.clock.setFixedTime(at(0));
  await page.goto(previewUrl);
  await workspaceNav().getByRole('button', { name: 'Meeting', exact: true }).waitFor();
  await page.clock.setFixedTime(at(0));
  for (const event of await appCall('list_events')) await appCall('delete_event', { id: event.id });
}
/** Records for `seconds` of the (frozen) clock and stops: the job chain starts at T0 + seconds. */
async function recordFor(title, seconds = 120, { eventId = null } = {}) {
  await page.clock.setFixedTime(at(0));
  const detail = await previewApi('meeting_start', {
    request: {
      request_id: `test-${++nextRequest}`,
      title,
      event_id: eventId,
      microphone: 'demo-mic',
      system: 'demo-system',
    },
  });
  await page.clock.setFixedTime(at(seconds));
  await previewApi('meeting_stop');
  return { id: detail.meeting.id, stopAt: seconds };
}
/** Opens the meeting from its row and waits for its title in the detail. */
async function openRow(title) {
  await rowOf(title).locator('.item-main').click();
  await expect(detailPane().getByRole('heading', { name: title, level: 2 })).toBeVisible();
}
const tabOf = (name) => detailPane().getByRole('tab', { name, exact: true });
const readMeeting = (id) => previewApi('meeting_read', { id });

// ─── The refresh (Task 14) ────────────────────────────────────────────────────────────────────
const callMark = () => page.evaluate(() => (window.__meetingLog ??= []).length);
/** What the page asked of the backend since `mark`: neither this script's calls nor the indicator's poll. */
async function pageCalls(mark) {
  const calls = await page.evaluate((from) => (window.__meetingLog ?? []).slice(from), mark);
  return calls.filter(
    (call) =>
      !call.test &&
      call.command !== 'meeting_capture_status' &&
      !(call.command === 'meeting_list_page' && call.args?.state === 'interrupted'),
  );
}
const countOf = (calls, command) => calls.filter((call) => call.command === command).length;
/** The recording as the capture bus has it: what the indicator last read. */
const busNow = () =>
  page.evaluate(async () => (await import('/src/lib/meetings.ts')).lastCapture());
/** What the Assistant's source button and the indicator do: leave the request for the page. */
const requestOpen = (hit) =>
  page.evaluate(async (hit) => (await import('/src/lib/meetings.ts')).requestMeetingOpen(hit), hit);
const sourceHit = (over) => ({
  meeting_id: 0,
  title: '',
  kind: 'transcript',
  text: '',
  at_ms: null,
  version: null,
  segment_id: null,
  ...over,
});
const toMeetings = () => workspaceNav().getByRole('button', { name: 'Meeting', exact: true }).click();
async function startRecording(title) {
  return previewApi('meeting_start', {
    request: {
      request_id: `test-${++nextRequest}`,
      title,
      event_id: null,
      microphone: 'demo-mic',
      system: 'demo-system',
    },
  });
}

try {
  await page.clock.setFixedTime(at(0));
  await page.goto(previewUrl);
  await workspaceNav().getByRole('button', { name: 'Meeting', exact: true }).waitFor();
  // The sample calendar has a few events of its own, dated when the sample was made: start clean.
  for (const event of await appCall('list_events')) await appCall('delete_event', { id: event.id });

  await step('list_shows_the_four_groups_in_order_and_hides_empty_ones', async () => {
    // Only the sample meeting exists: one group, the other three are not drawn at all.
    await openMeetings();
    assert.deepEqual(await groupLabels(), ['Passate']);
    await expect(rowOf('Weekly sync')).toHaveCount(1);
    await leaveMeetings();

    // The calendar: three events to come (one already begun), a day-long one and one a month away
    // (both left out), and a lunch nobody recorded.
    await saveEvent({
      title: 'Colloquio candidata Bianchi',
      start: T0 - 5 * MIN,
      end: T0 + 55 * MIN,
      attendees: people(3),
      meeting_link: MEET,
    });
    await saveEvent({
      title: 'Sprint planning',
      start: T0 + 4 * HOUR + 30 * MIN,
      end: T0 + 5 * HOUR + 30 * MIN,
      attendees: people(6),
      meeting_link: MEET,
    });
    await saveEvent({
      title: 'Demo per Orsini Srl',
      start: T0 + 2 * DAY,
      end: T0 + 2 * DAY + HOUR,
      attendees: people(4),
      location: 'Sala Navigli',
    });
    await saveEvent({ title: 'Giornata fuori sede', start: local(11, 10), end: local(11, 11) });
    await saveEvent({ title: 'Evento lontano', start: T0 + 10 * DAY, end: T0 + 10 * DAY + HOUR });
    await saveEvent({
      title: 'Pranzo con Marco',
      start: local(11, 5, 13),
      end: local(11, 5, 14),
      location: 'Da Gino',
    });
    await saveEvent({ title: 'Evento vecchio', start: T0 - 20 * DAY, end: T0 - 20 * DAY + HOUR });
    const standup = await saveEvent({
      title: 'Stand-up del team',
      start: T0 - DAY - 30 * MIN,
      end: T0 - DAY,
      attendees: people(5),
    });

    // The recordings. The engine is "blocked" while the first two are made, so they stay as audio
    // waiting to be transcribed (and not in elaborazione); the third is let through.
    await previewApi('meeting_preview_readiness', { transcription_blocker: 'Nessun motore (prova)' });
    await record('Stand-up del team', { eventId: standup.id });
    const interrupted = await record('Chiamata fornitore imballaggi');
    await previewApi('meeting_preview_interrupt', { id: interrupted.meeting.id });
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await record('Revisione budget Q4');
    await record('Allineamento prodotto', { stop: false });
    // Half way through the transcription of the one that was stopped.
    await page.clock.setFixedTime(at(1.7));

    await openMeetings();
    assert.deepEqual(await groupLabels(), ['In corso', 'In elaborazione', 'Prossime', 'Passate']);

    // In corso: the recording, a red dot and the time it has been going.
    assert.deepEqual(await titlesIn('live'), ['Allineamento prodotto']);
    const live = rowOf('Allineamento prodotto');
    await expect(live.locator('.item-sub')).toHaveText('Registrazione in corso');
    await expect(live.locator('.item-dot')).toBeVisible();
    await expect(live.locator('.item-elapsed')).toHaveText(/^\d{2}:\d{2}$/);
    await expect(live.locator('.item-when')).toHaveText('10:00');

    // In elaborazione: the phase and its percentage, on a thin bar.
    assert.deepEqual(await titlesIn('processing'), ['Revisione budget Q4']);
    const processing = rowOf('Revisione budget Q4');
    await expect(processing.locator('.item-sub')).toHaveText('Trascrizione 50%');
    await expect(processing.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '50');

    // Prossime: soonest first; the one that has begun says so; the day-long event and the one a
    // month away are not there. A place, or the call service, and the guests.
    assert.deepEqual(await titlesIn('upcoming'), [
      'Colloquio candidata Bianchi',
      'Sprint planning',
      'Demo per Orsini Srl',
    ]);
    await expect(rowOf('Colloquio candidata Bianchi').locator('.item-sub')).toHaveText(
      'Iniziata da 5 min · Google Meet · 3 partecipanti',
    );
    await expect(rowOf('Colloquio candidata Bianchi').locator('.item-when')).toHaveText('09:55');
    await expect(rowOf('Sprint planning').locator('.item-sub')).toHaveText(
      'Google Meet · 6 partecipanti',
    );
    await expect(rowOf('Demo per Orsini Srl').locator('.item-sub')).toHaveText(
      'Sala Navigli · 4 partecipanti',
    );
    await expect(rowOf('Demo per Orsini Srl').locator('.item-when')).toHaveText(/mer\.? 11/i);
    await expect(rowOf('Giornata fuori sede')).toHaveCount(0);
    await expect(rowOf('Evento lontano')).toHaveCount(0);
    // "Partecipa e registra" opens the call and records: the desktop does it, like the Calendar,
    // so the browser preview offers no button that cannot work.
    await expect(page.getByRole('button', { name: 'Partecipa e registra' })).toHaveCount(0);

    // Passate: newest first. A recording that is waiting for its transcript says so; an
    // interrupted one says it must be recovered, in red; an old sample meeting shows its length.
    const past = await titlesIn('past');
    assert.deepEqual(past.slice(0, 2).sort(), ['Chiamata fornitore imballaggi', 'Stand-up del team']);
    assert.deepEqual(past.slice(2), ['Pranzo con Marco', 'Weekly sync', 'Pricing review']);
    await expect(rowOf('Stand-up del team').locator('.item-tag')).toHaveText('Da trascrivere');
    await expect(rowOf('Chiamata fornitore imballaggi').locator('.item-tag')).toHaveText(
      'Da recuperare',
    );
    await expect(rowOf('Chiamata fornitore imballaggi').locator('.item-tag')).toHaveClass(/danger/);
    await expect(rowOf('Weekly sync').locator('.item-sub')).toHaveText('29 min');
    await expect(rowOf('Weekly sync').locator('.item-when')).toHaveText(/1\s+nov/i);
    await expect(rowOf('Pricing review').locator('.item-sub')).toHaveText('15 min');
    await expect(rowOf('Evento vecchio')).toHaveCount(0);
  });

  await step('a_calendar_event_with_a_recording_is_one_row', async () => {
    // "Stand-up del team" exists as an event and as a recording linked to it: one row, the
    // recording, which can be opened; the event does not appear a second time, grey.
    await expect(rowOf('Stand-up del team')).toHaveCount(1);
    await expect(rowOf('Stand-up del team')).toHaveAttribute('data-row-key', /^m:/);
    await expect(rowOf('Stand-up del team')).not.toHaveClass(/unrecorded/);
    await expect(rowOf('Stand-up del team').locator('.item-main')).toHaveCount(1);
    await expect(page.locator('.meetings-page [data-row-key^="e:"]')).toHaveCount(4);
    // The event of the same day nobody recorded stays a row of its own.
    await expect(rowOf('Pranzo con Marco')).toHaveAttribute('data-row-key', /^e:/);
  });

  await step('unrecorded_past_events_are_grey_and_do_not_open', async () => {
    const lunch = rowOf('Pranzo con Marco');
    await expect(lunch).toHaveClass(/unrecorded/);
    await expect(lunch.locator('.item-sub')).toHaveText('Non registrata');
    await expect(lunch.locator('.item-when')).toHaveText(/5\s+nov/i);
    // Nothing to press: it is not a button, and a click leaves the right-hand side as it was.
    await expect(lunch.getByRole('button')).toHaveCount(0);
    await lunch.locator('.item-title').click();
    await expect(page.locator('.meetings-page [aria-current="true"]')).toHaveCount(0);
    await expect(detailPane()).toContainText('Seleziona una riunione');
    await expect(detailPane()).toHaveAttribute('data-key', '');
    // Grey: the title is not the colour of the title of a row that opens.
    const colour = (row) => row.locator('.item-title').evaluate((el) => getComputedStyle(el).color);
    assert.notEqual(await colour(lunch), await colour(rowOf('Weekly sync')));
    // An event still to come does not open either.
    await rowOf('Sprint planning').locator('.item-title').click();
    await expect(page.locator('.meetings-page [aria-current="true"]')).toHaveCount(0);
    // A recording does: the row is the current one and the shell holds its key.
    await rowOf('Weekly sync').locator('.item-main').click();
    await expect(rowOf('Weekly sync').locator('.item-main')).toHaveAttribute('aria-current', 'true');
    await expect(detailPane()).toHaveAttribute('data-key', /^m:\d+$/);
  });

  await step('record_now_popover_starts_a_recording_and_the_row_moves_to_in_corso', async () => {
    // Stop the live recording of the seed (the engine is blocked: no elaborazione) and open the
    // page again: there is nothing In corso any more, so that group is not drawn.
    await previewApi('meeting_preview_readiness', { transcription_blocker: 'Nessun motore (prova)' });
    await previewApi('meeting_stop');
    await leaveMeetings();
    await openMeetings();
    await expect(group('live')).toHaveCount(0);
    await expect(recordNow()).toBeEnabled();

    // The popover: an optional name, where the sound comes from, Annulla and one "Registra".
    await recordNow().click();
    const dialog = page.getByRole('dialog', { name: 'Registra ora' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel('Nome (facoltativo)')).toBeVisible();
    await expect(dialog).toContainText('Microfono e audio del computer, dispositivi predefiniti');
    await expect(page.getByRole('button', { name: 'Registra', exact: true })).toHaveCount(1);
    await dialog.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(group('live')).toHaveCount(0);
    assert.equal((await previewApi('meeting_capture_status')).state, 'saved');

    // Without a name the meeting gets the fallback title; the row is In corso, on top, and open.
    await recordNow().click();
    await page.getByRole('button', { name: 'Registra', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(group('live')).toHaveCount(1);
    assert.equal((await groupLabels())[0], 'In corso');
    assert.deepEqual(await titlesIn('live'), ['Riunione di esempio']);
    await expect(rowOf('Riunione di esempio').locator('.item-dot')).toBeVisible();
    await expect(rowOf('Riunione di esempio').locator('.item-main')).toHaveAttribute(
      'aria-current',
      'true',
    );
    assert.equal((await previewApi('meeting_capture_status')).state, 'recording');
    // One recording at a time: the button waits until it is over.
    await expect(recordNow()).toBeDisabled();

    // With a name, started with Enter. The first one, stopped, moves down to Passate.
    await previewApi('meeting_stop');
    await expect(recordNow()).toBeEnabled();
    await recordNow().click();
    const name = dialog.getByLabel('Nome (facoltativo)');
    await name.fill('Chiamata con il fornitore');
    await name.press('Enter');
    await expect(dialog).toHaveCount(0);
    assert.deepEqual(await titlesIn('live'), ['Chiamata con il fornitore']);
    assert.ok((await titlesIn('past')).includes('Riunione di esempio'));
    await previewApi('meeting_stop');
  });

  await step('search_replaces_the_list_with_results_and_ignores_notes', async () => {
    // The transcription that was half way through finishes; a note of another meeting mentions the
    // same word, and notes are not on the page, so a hit in them leads nowhere.
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await page.clock.setFixedTime(at(10));
    const meetings = await previewApi('meeting_list');
    const byTitle = (title) => meetings.find((item) => item.meeting.title === title);
    const standup = await previewApi('meeting_read', { id: byTitle('Stand-up del team').meeting.id });
    await previewApi('meeting_save_notes', {
      id: standup.meeting.id,
      text: 'Ricordare: il prototipo va mostrato al cliente.',
      expectedRevision: standup.revision,
    });
    const budgetId = byTitle('Revisione budget Q4').meeting.id;
    await leaveMeetings();
    await openMeetings();
    assert.deepEqual(await groupLabels(), ['Prossime', 'Passate']);

    await searchBox().fill('Prototipo');
    const results = page.locator('.meetings-page .hit-item');
    await expect(results).toHaveCount(2);
    // The list is gone: only the results, under their own heading.
    await expect(page.locator('.meetings-page [data-group]')).toHaveCount(0);
    assert.deepEqual(await groupLabels(), ['2 risultati per «Prototipo»']);
    // Meeting, the moment (mm:ss) and the fragment with the match marked, whatever the case.
    await expect(results.nth(0).locator('.hit-title')).toHaveText('Revisione budget Q4');
    await expect(results.nth(0).locator('.hit-where')).toHaveText('00:00');
    await expect(results.nth(0).locator('mark')).toHaveText('prototipo');
    await expect(results.nth(0).locator('.hit-fragment')).toContainText(
      'Confermiamo il prototipo per venerdì',
    );
    await expect(results.nth(1).locator('.hit-where')).toHaveText('Sintesi');
    await expect(results.nth(1).locator('mark')).toHaveText('prototipo');
    await expect(page.locator('.meetings-page')).not.toContainText('Ricordare');

    // A click opens the meeting: the transcript at that line, or the summary.
    await results.nth(0).click();
    await expect(results.nth(0)).toHaveAttribute('aria-current', 'true');
    await expect(results.nth(1)).not.toHaveAttribute('aria-current', 'true');
    await expect(detailPane()).toHaveAttribute('data-key', `m:${budgetId}`);
    await expect(detailPane()).toHaveAttribute('data-tab', 'transcript');
    await expect(detailPane()).toHaveAttribute('data-segment', 'segment-1');
    await results.nth(1).click();
    await expect(detailPane()).toHaveAttribute('data-tab', 'summary');
    await expect(detailPane()).toHaveAttribute('data-segment', '');

    // Only a note says it: no result, and the page says so instead of listing nothing.
    await searchBox().fill('ricordare');
    await expect(page.locator('.meetings-page .group-label')).toHaveText(
      'Nessun risultato per «ricordare»',
    );
    await expect(results).toHaveCount(0);
    // Markup is text: nothing the user typed is drawn as HTML.
    await searchBox().fill('<b>x</b>');
    await expect(page.locator('.meetings-page .group-label')).toHaveText(
      'Nessun risultato per «<b>x</b>»',
    );
    await expect(page.locator('.meetings-page .group-label b')).toHaveCount(0);

    // Emptying the field brings the list back, with the meeting that was opened still the current one.
    await searchBox().fill('');
    await expect(page.locator('.meetings-page [data-group]')).toHaveCount(2);
    assert.deepEqual(await groupLabels(), ['Prossime', 'Passate']);
    await expect(rowOf('Revisione budget Q4').locator('.item-main')).toHaveAttribute(
      'aria-current',
      'true',
    );
  });

  await step('show_more_meetings_follows_next_offset', async () => {
    // A page is fifty meetings; the rest comes with "Mostra altre riunioni" and the link goes
    // away when there is no more.
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      for (let i = 0; i < 55; i++) await api.meetingCall('meeting_import', {});
    });
    const total = (await previewApi('meeting_list')).length;
    assert.ok(total > 50, `${total} meetings`);
    await reopenMeetings();
    const meetingRows = page.locator('.meetings-page [data-row-key^="m:"]');
    await expect(meetingRows).toHaveCount(50);
    const more = page.getByRole('button', { name: 'Mostra altre riunioni', exact: true });
    await expect(more).toBeVisible();
    await more.click();
    await expect(meetingRows).toHaveCount(total);
    await expect(more).toHaveCount(0);
  });

  // ─── Task 13: the detail ────────────────────────────────────────────────────────────────────
  const SUMMARY = 'Il team conferma il prototipo e prepara la revisione del progetto.';
  const ACTION = 'Preparare la revisione del progetto.';
  const SPOKEN = 'Confermiamo il prototipo per venerdì. Maria prepara la revisione del progetto.';
  const NO_ENGINE = 'Installa il modello di trascrizione in Impostazioni.';

  await step('a_past_meeting_opens_on_the_summary_and_the_transcript_tab_shows_times', async () => {
    await fresh();
    const event = await saveEvent({
      title: 'Revisione budget Q4',
      start: T0,
      end: T0 + HOUR,
      attendees: people(3),
    });
    const { stopAt } = await recordFor('Revisione budget Q4', 120, { eventId: event.id });
    await page.clock.setFixedTime(at(stopAt + 10)); // transcribed, summarized, audio deleted
    await openMeetings();
    await openRow('Revisione budget Q4');
    const pane = detailPane();

    // The header: when, how long, who (from the linked event).
    const meta = pane.locator('.detail-meta');
    await expect(meta).toContainText('Lunedì 9 novembre');
    await expect(meta).toContainText('10:00');
    await expect(meta).toContainText('2 min');
    await expect(meta).toContainText('ospite1, ospite2, ospite3');

    // Two tabs, opened on the summary; no player, notes, versions or export any more.
    const tabs = pane.getByRole('tablist', { name: 'Contenuto della riunione' });
    await expect(tabs.getByRole('tab')).toHaveText(['Sintesi', 'Trascrizione']);
    await expect(tabOf('Sintesi')).toHaveAttribute('aria-selected', 'true');
    await expect(tabOf('Trascrizione')).toHaveAttribute('aria-selected', 'false');
    const panel = pane.getByRole('tabpanel');
    await expect(panel).toContainText(SUMMARY);
    await expect(panel.getByRole('heading', { name: 'Punti chiave' })).toBeVisible();
    await expect(panel).toContainText('Prototipo previsto per venerdì.');
    await expect(panel.getByRole('checkbox', { name: ACTION })).not.toBeChecked();
    await expect(panel).toContainText('Maria');
    await expect(pane).toContainText("L'audio è stato eliminato dopo la trascrizione.");
    await expect(pane.getByRole('slider')).toHaveCount(0);
    await expect(pane.getByRole('tab', { name: /Appunti|Notes/ })).toHaveCount(0);

    // The transcript: one line per segment, with its time, read-only.
    await tabOf('Trascrizione').click();
    await expect(tabOf('Trascrizione')).toHaveAttribute('aria-selected', 'true');
    const line = pane.locator('[data-segment-id="segment-1"]');
    await expect(line.locator('time')).toHaveText('00:00');
    await expect(line).toContainText(SPOKEN);
    await expect(line).not.toHaveAttribute('aria-current', 'true');
    await expect(pane.getByRole('textbox')).toHaveCount(0);
    await expect(pane.getByRole('button', { name: 'Copia', exact: true })).toBeVisible();
    // The caption promises the times, so it is there only when the lines have them.
    const CAPTION = 'Testo unico, con l’ora di ogni passaggio';
    await expect(pane).toContainText(CAPTION);

    // A meeting from before the versions: its plain text, no times; and again it opens on the summary.
    await openRow('Weekly sync');
    await expect(tabOf('Sintesi')).toHaveAttribute('aria-selected', 'true');
    await expect(pane).toContainText('Team aligned on the launch plan; proposal due Thursday.');
    await expect(pane).toContainText('Designs are about 80% complete');
    await expect(pane.locator('.detail-meta')).toContainText('29 min');
    await tabOf('Trascrizione').click();
    await expect(pane.locator('time')).toHaveCount(0);
    await expect(pane).toContainText("Alex: Let's go through the launch plan.");
    await expect(pane).not.toContainText(CAPTION);
  });

  await step('summary_actions_toggle_and_copy_gives_plain_text', async () => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: previewUrl });
    await fresh();
    const { id, stopAt } = await recordFor('Allineamento', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await openMeetings();
    await openRow('Allineamento');
    const pane = detailPane();
    const action = pane.getByRole('checkbox', { name: ACTION });

    // Ticking an action saves it against the current revision.
    const before = await readMeeting(id);
    await pane.getByText(ACTION, { exact: true }).click();
    await expect(action).toBeChecked();
    let after = await readMeeting(id);
    assert.equal(after.summaries[0].actions[0].done, true);
    assert.equal(after.revision, before.revision + 1);

    // Somebody else changed the meeting in the meantime: nothing is overwritten, the page reads
    // the meeting again, says so, and the next click goes through.
    await previewApi('meeting_update', {
      id,
      title: 'Titolo cambiato altrove',
      eventId: null,
      includedInSearch: true,
      expectedRevision: after.revision,
    });
    await pane.getByText(ACTION, { exact: true }).click();
    await expect(page.getByText('La riunione è cambiata: l’ho riletta.')).toBeVisible();
    await expect(pane.getByRole('heading', { name: 'Titolo cambiato altrove', level: 2 })).toBeVisible();
    await expect(action).toBeChecked();
    assert.equal((await readMeeting(id)).summaries[0].actions[0].done, true);
    await pane.getByText(ACTION, { exact: true }).click();
    await expect(action).not.toBeChecked();
    assert.equal((await readMeeting(id)).summaries[0].actions[0].done, false);

    // Copy gives plain text: the summary, then key points, decisions and actions as bullet lists.
    // (Windows hands the lines back with CRLF.)
    const clipboard = async () =>
      (await page.evaluate(() => navigator.clipboard.readText())).replaceAll('\r\n', '\n');
    await pane.getByRole('button', { name: 'Copia', exact: true }).click();
    await expect(page.getByText('Copiato', { exact: true })).toBeVisible();
    assert.equal(
      await clipboard(),
      [
        SUMMARY,
        '- Prototipo previsto per venerdì.',
        '- Confermare il prototipo per venerdì.',
        `- ${ACTION} (Maria)`,
      ].join('\n\n'),
    );
    // The transcript copies with its times.
    await tabOf('Trascrizione').click();
    await pane.getByRole('button', { name: 'Copia', exact: true }).click();
    assert.equal(await clipboard(), `00:00 ${SPOKEN}`);
    // A clipboard that refuses is told, not ignored.
    await page.evaluate(() => {
      navigator.clipboard.writeText = () => Promise.reject(new Error('denied'));
    });
    await pane.getByRole('button', { name: 'Copia', exact: true }).click();
    await expect(page.getByText('Non sono riuscito a copiare', { exact: false })).toBeVisible();
  });

  await step('search_hit_opens_the_transcript_on_the_highlighted_segment', async () => {
    await fresh();
    await previewApi('meeting_preview_long_transcript');
    const { stopAt } = await recordFor('Pianificazione trimestre', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await openMeetings();

    await searchBox().fill('argomento 31');
    const hits = page.locator('.meetings-page .hit-item');
    await expect(hits).toHaveCount(1);
    await expect(hits.first().locator('.hit-where')).toHaveText('10:00');
    await hits.first().click();
    const pane = detailPane();
    await expect(tabOf('Trascrizione')).toHaveAttribute('aria-selected', 'true');
    // The line is marked, and the transcript was scrolled to it (40 lines do not fit).
    const line = pane.locator('[data-segment-id="segment-31"]');
    await expect(line).toHaveAttribute('aria-current', 'true');
    await expect(line.locator('time')).toHaveText('10:00');
    await expect(line).toBeInViewport();
    await expect(pane.locator('[data-segment-id="segment-1"]')).not.toBeInViewport();
    await expect(pane.locator('[data-segment-id][aria-current="true"]')).toHaveCount(1);

    // A result in the summary opens the summary, with nothing marked; the transcript opened by hand
    // afterwards starts from the top.
    await searchBox().fill('conferma il prototipo');
    // (The old results may stay on screen until the new ones are in: wait for the new one.)
    await expect(hits.first().locator('.hit-where')).toHaveText('Sintesi');
    await hits.first().click();
    await expect(tabOf('Sintesi')).toHaveAttribute('aria-selected', 'true');
    await expect(pane.getByRole('tabpanel')).toContainText(SUMMARY);
    await tabOf('Trascrizione').click();
    await expect(pane.locator('[data-segment-id][aria-current="true"]')).toHaveCount(0);
    await expect(pane.locator('[data-segment-id="segment-1"]')).toBeInViewport();
  });

  await step('rename_and_delete_ask_before_acting', async () => {
    await fresh();
    const event = await saveEvent({ title: 'Evento collegato', start: T0, end: T0 + HOUR });
    const { id, stopAt } = await recordFor('Vecchio nome', 120, { eventId: event.id });
    await page.clock.setFixedTime(at(stopAt + 10));
    await openMeetings();
    await openRow('Vecchio nome');
    const pane = detailPane();
    const more = pane.getByRole('button', { name: 'Altre azioni', exact: true });
    const menu = () => page.getByRole('dialog', { name: 'Azioni sulla riunione' });

    // The menu has exactly the two actions.
    await more.click();
    await expect(menu().getByRole('button')).toHaveText(['Rinomina', 'Elimina']);
    await menu().getByRole('button', { name: 'Rinomina' }).click();
    const rename = page.getByRole('dialog', { name: 'Rinomina riunione' });
    const name = rename.getByLabel('Nome', { exact: true });
    await expect(name).toHaveValue('Vecchio nome');
    // An empty name is refused: the dialog stays, nothing is saved.
    await name.fill('   ');
    await expect(rename.getByRole('button', { name: 'Salva', exact: true })).toBeDisabled();
    await name.press('Enter');
    await expect(rename).toBeVisible();
    assert.equal((await readMeeting(id)).meeting.title, 'Vecchio nome');
    // Esc leaves it as it was.
    await name.press('Escape');
    await expect(rename).toHaveCount(0);
    assert.equal((await readMeeting(id)).meeting.title, 'Vecchio nome');

    // A new name, with Enter. The link to the event and the search flag are sent back as they were.
    const revision = (await readMeeting(id)).revision;
    await more.click();
    await menu().getByRole('button', { name: 'Rinomina' }).click();
    await name.fill('Revisione budget finale');
    await name.press('Enter');
    await expect(rename).toHaveCount(0);
    await expect(pane.getByRole('heading', { name: 'Revisione budget finale', level: 2 })).toBeVisible();
    await expect(rowOf('Revisione budget finale')).toHaveCount(1);
    await expect(rowOf('Vecchio nome')).toHaveCount(0);
    const renamed = await readMeeting(id);
    assert.equal(renamed.meeting.title, 'Revisione budget finale');
    assert.equal(renamed.meeting.event_id, event.id);
    assert.equal(renamed.included_in_search, true);
    assert.equal(renamed.revision, revision + 1);

    // Delete asks first; "Annulla" keeps the meeting.
    await more.click();
    await menu().getByRole('button', { name: 'Elimina' }).click();
    const confirm = page.getByRole('dialog', { name: 'Elimina riunione' });
    await expect(confirm).toContainText(
      'Eliminare «Revisione budget finale»? La registrazione e il testo vengono rimossi. Non si può annullare.',
    );
    await confirm.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    assert.ok((await previewApi('meeting_list')).some((item) => item.meeting.id === id));
    await expect(pane.getByRole('heading', { name: 'Revisione budget finale', level: 2 })).toBeVisible();
    // "Elimina" removes it: the list loses the row and the right-hand side is empty again.
    await more.click();
    await menu().getByRole('button', { name: 'Elimina' }).click();
    await confirm.getByRole('button', { name: 'Elimina', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    await expect(pane).toContainText('Seleziona una riunione');
    await expect(pane).toHaveAttribute('data-key', '');
    await expect(rowOf('Revisione budget finale')).toHaveCount(0);
    assert.ok(!(await previewApi('meeting_list')).some((item) => item.meeting.id === id));
  });

  await step('live_panel_pauses_and_stops', async () => {
    await fresh();
    await record('Allineamento prodotto', { stop: false });
    await openMeetings();
    await openRow('Allineamento prodotto');
    const pane = detailPane();
    const live = pane.locator('.live');
    await expect(live).toContainText('In registrazione');
    // Levels come from the capture the indicator already polls (the sample says 0.16 and 0.28).
    await expect(live.getByRole('meter', { name: 'Microfono' })).toHaveAttribute('aria-valuenow', '0.16');
    await expect(live.getByRole('meter', { name: 'Audio del computer' })).toHaveAttribute(
      'aria-valuenow',
      '0.28',
    );
    // Nothing else is offered: no tabs, no menu.
    await expect(pane.getByRole('tablist')).toHaveCount(0);
    await expect(pane.getByRole('button')).toHaveText(['Pausa', 'Termina']);
    await page.clock.setFixedTime(at(65));
    await expect(live.locator('.live-clock')).toHaveText('01:05');

    await live.getByRole('button', { name: 'Pausa', exact: true }).click();
    await expect(live).toContainText('In pausa');
    assert.equal((await previewApi('meeting_capture_status')).state, 'paused');
    await expect(live.getByRole('meter', { name: 'Microfono' })).toHaveAttribute('aria-valuenow', '0');
    await live.getByRole('button', { name: 'Riprendi', exact: true }).click();
    await expect(live).toContainText('In registrazione');
    assert.equal((await previewApi('meeting_capture_status')).state, 'recording');

    // "Termina": saved, and the detail turns into the progress of the transcription by itself.
    await page.clock.setFixedTime(at(120));
    await live.getByRole('button', { name: 'Termina', exact: true }).click();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    assert.equal((await previewApi('meeting_capture_status')).state, 'saved');
    await expect(live).toHaveCount(0);
    assert.deepEqual(await titlesIn('processing'), ['Allineamento prodotto']);
    await expect(group('live')).toHaveCount(0);
  });

  await step('processing_shows_only_progress_and_cancel', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Revisione budget Q4', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7)); // half way through the transcription
    await openMeetings();
    await openRow('Revisione budget Q4');
    const pane = detailPane();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    await expect(pane.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '50');
    await expect(pane).toContainText('Trascrizione 50%');
    // Nothing else: no tabs, no copy, no menu.
    await expect(pane.getByRole('tablist')).toHaveCount(0);
    await expect(pane.getByRole('button')).toHaveText(['Annulla']);

    // "Annulla" stops the job; the page then says it was cancelled and offers to try again.
    await pane.getByRole('button', { name: 'Annulla', exact: true }).click();
    assert.equal((await readMeeting(id)).jobs.at(-1).state, 'cancelled');
    await expect(pane).toContainText('La trascrizione è stata annullata.');
    await expect(pane.getByRole('button', { name: 'Riprova', exact: true })).toBeVisible();
    assert.deepEqual(await titlesIn('processing'), []);
  });

  await step('interrupted_meeting_offers_one_button', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { transcription_blocker: 'Nessun motore (prova)' });
    const { id } = await recordFor('Chiamata fornitore imballaggi', 120);
    await previewApi('meeting_preview_interrupt', { id });
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await openMeetings();
    await openRow('Chiamata fornitore imballaggi');
    const pane = detailPane();
    const state = pane.locator('.state');
    await expect(state.getByRole('heading')).toHaveText('La registrazione si è interrotta');
    await expect(state).toContainText("L'audio salvato fino a quel momento si può recuperare.");
    await expect(pane.locator('.detail-meta')).toContainText('Da recuperare');
    await expect(state.getByRole('button')).toHaveText(['Recupera audio']);
    await expect(pane.getByRole('tablist')).toHaveCount(0);

    // One click: the audio is recovered and the transcription starts by itself.
    await state.getByRole('button', { name: 'Recupera audio', exact: true }).click();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    assert.equal((await readMeeting(id)).capture.state, 'saved');
    assert.deepEqual(await titlesIn('processing'), ['Chiamata fornitore imballaggi']);
  });

  await step('blocked_engine_shows_the_reason_and_a_transcribe_button', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    const { id } = await recordFor('Riunione con il commercialista', 120);
    await openMeetings();
    await openRow('Riunione con il commercialista');
    const pane = detailPane();
    const state = pane.locator('.state');
    await expect(state.getByRole('heading')).toHaveText('Questa riunione non ha ancora una trascrizione');
    await expect(state.locator('.notice')).toContainText(NO_ENGINE);
    await expect(state.getByRole('button')).toHaveText(['Trascrivi', 'Apri Impostazioni › Riunioni']);
    await expect(pane.getByRole('tablist')).toHaveCount(0);

    // "Trascrivi" while the engine is still missing says why instead of doing nothing.
    await state.getByRole('button', { name: 'Trascrivi', exact: true }).click();
    await expect(page.getByText(NO_ENGINE).last()).toBeVisible();
    assert.deepEqual((await readMeeting(id)).jobs, []);

    // The link goes to Settings, on the Meetings section.
    await state.getByRole('button', { name: 'Apri Impostazioni › Riunioni' }).click();
    await expect(page.locator('section.settings-page')).toBeVisible();
    await expect(page.locator('.settings-page button.active')).toHaveText('Riunioni');

    // Back with the engine installed: the same button starts the transcription.
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await workspaceNav().getByRole('button', { name: 'Meeting', exact: true }).click();
    await expect(page.locator('.meetings-page [data-list-ready]')).toBeAttached();
    await openRow('Riunione con il commercialista');
    await expect(pane.locator('.notice')).toHaveCount(0);
    await pane.getByRole('button', { name: 'Trascrivi', exact: true }).click();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    assert.equal((await readMeeting(id)).jobs.at(-1).kind, 'transcribe');
  });

  // "Riprova" after a failed or cancelled transcription is a new transcription with the engine
  // Settings says now: the job run again would restart with the options it failed with, and a fix
  // made in Settings (the model, the language) would change nothing.
  await step('retry_after_a_cancelled_transcription_uses_the_settings_of_now', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Da riprovare', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7)); // half way through the transcription
    await openMeetings();
    await openRow('Da riprovare');
    const pane = detailPane();
    await pane.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(pane).toContainText('La trascrizione è stata annullata.');
    const transcriptions = async () => (await readMeeting(id)).jobs.filter((job) => job.kind === 'transcribe');
    assert.deepEqual((await transcriptions())[0].options, { provider: 'compute', model: 'base', language: 'it' });

    // The engine is fixed in Settings; "Riprova" starts a new job with the new options.
    await previewApi('meeting_preview_transcribe_options', { model: 'small', language: 'en' });
    await pane.getByRole('button', { name: 'Riprova', exact: true }).click();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    const jobs = await transcriptions();
    assert.equal(jobs.length, 2, 'a new job, not the old one run again');
    assert.deepEqual(jobs[0].options, { provider: 'compute', model: 'base', language: 'it' }, 'the cancelled job keeps its own');
    assert.deepEqual(jobs[1].options, { provider: 'compute', model: 'small', language: 'en' }, 'the retry runs with the settings of now');

    // Cancelled again, and the engine is blocked: "Riprova" says why instead of starting, and the
    // reason is shown the way the page shows every backend sentence.
    await pane.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(pane).toContainText('La trascrizione è stata annullata.');
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await pane.getByRole('button', { name: 'Riprova', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Notifiche' }).getByText(NO_ENGINE)).toBeVisible();
    assert.equal((await transcriptions()).length, 2, 'nothing was started');
    await expect(pane).toContainText('La trascrizione è stata annullata.');
  });

  // The rows of the list say what is wrong and what to do about it, without opening the meeting.
  await step('rows_say_why_a_recording_waits_and_a_failed_one_can_be_tried_again', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await recordFor('Senza motore', 120);
    await openMeetings();
    const waiting = rowOf('Senza motore');
    // The engine cannot run: the row gives the reason and a way to Settings (not a second "Da trascrivere").
    await expect(waiting.locator('.item-sub')).toContainText(NO_ENGINE);
    await expect(waiting.locator('.item-tag')).toHaveText('Da trascrivere');
    await waiting.getByRole('button', { name: /^Apri Impostazioni › Riunioni/ }).click();
    await expect(page.locator('section.settings-page')).toBeVisible();
    await expect(page.locator('.settings-page button.active')).toHaveText('Riunioni');
    // Back with the engine installed: the page asks the engine again, the row is the usual one.
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await workspaceNav().getByRole('button', { name: 'Meeting', exact: true }).click();
    await expect(page.locator('.meetings-page [data-list-ready]')).toBeAttached();
    await expect(waiting.locator('.item-sub')).toContainText('audio da trascrivere');
    await expect(waiting.locator('.item-sub')).not.toContainText(NO_ENGINE);
    await expect(waiting.getByRole('button', { name: /Impostazioni/ })).toHaveCount(0);

    // A transcription that was cancelled: "Trascrizione non riuscita" and "Riprova" on the row.
    const { id, stopAt } = await recordFor('Annullata', 120);
    await page.clock.setFixedTime(at(stopAt + 1));
    const jobId = (await readMeeting(id)).jobs.at(-1).id;
    await previewApi('meeting_cancel_job', { id, jobId });
    await reopenMeetings();
    const failed = rowOf('Annullata');
    await expect(failed.locator('.item-sub')).toHaveText('Trascrizione non riuscita');
    await expect(failed.locator('.item-sub')).toHaveClass(/danger/);
    const retry = failed.getByRole('button', { name: /^Riprova la trascrizione/ });
    // The engine is blocked: the reason is told, nothing starts, the row stays as it was.
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await retry.click();
    await expect(page.getByRole('region', { name: 'Notifiche' }).getByText(NO_ENGINE)).toBeVisible();
    assert.equal((await readMeeting(id)).jobs.length, 1);
    await expect(failed.locator('.item-sub')).toHaveText('Trascrizione non riuscita');
    // Fixed: "Riprova" is a new transcription with the settings of now, and the row moves along
    // (the keyboard goes with it).
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await previewApi('meeting_preview_transcribe_options', { language: 'en' });
    await retry.focus();
    await page.keyboard.press('Enter');
    await expect.poll(async () => await titlesIn('processing')).toContain('Annullata');
    const jobs = (await readMeeting(id)).jobs;
    assert.equal(jobs.length, 2);
    assert.equal(jobs.at(-1).options.language, 'en');
    await expect(failed.locator('.item-main')).toBeFocused();
  });

  // The button that was pressed goes away when its action changes what the detail shows ("Trascrivi"
  // becomes the progress, "Annulla" becomes "Riprova"): the keyboard must not fall to the page.
  await step('an_action_that_takes_its_button_away_leaves_the_focus_in_the_detail', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await recordFor('Riunione muta', 120);
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await openMeetings();
    await openRow('Riunione muta');
    const pane = detailPane();
    const transcribe = pane.getByRole('button', { name: 'Trascrivi', exact: true });
    await transcribe.focus();
    await page.keyboard.press('Enter');
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    await expect(pane).toBeFocused();
    // "Annulla" is the same: pressing it brings "Riprova", and the focus stays in the detail.
    const cancel = pane.getByRole('button', { name: 'Annulla', exact: true });
    await cancel.focus();
    await page.keyboard.press('Enter');
    await expect(pane.getByRole('button', { name: 'Riprova', exact: true })).toBeVisible();
    await expect(pane).toBeFocused();
    // A button that stays does not lose the focus to the pane: "Riprova" starts the job and goes away, but
    // a press that fails (the engine is blocked) leaves the focus on the button.
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    const retry = pane.getByRole('button', { name: 'Riprova', exact: true });
    await retry.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('region', { name: 'Notifiche' }).getByText(NO_ENGINE)).toBeVisible();
    await expect(retry).toBeFocused();
  });

  await step('failed_summary_offers_retry', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { summary_ready: false });
    const { id, stopAt } = await recordFor('Verbale', 120);
    await page.clock.setFixedTime(at(stopAt + 10)); // the transcript is there, the summary is not
    await openMeetings();
    await openRow('Verbale');
    const pane = detailPane();
    const panel = pane.getByRole('tabpanel');

    // No assistant: the summary says what to set up, with a link to Settings.
    await expect(panel).toContainText('Configura un assistente in Impostazioni per avere la sintesi');
    await expect(panel.getByRole('button', { name: 'Impostazioni', exact: true })).toBeVisible();
    await tabOf('Trascrizione').click();
    await expect(pane).toContainText(SPOKEN);
    await tabOf('Sintesi').click();

    // Asked for anyway, the job fails and the page shows the reason with "Riprova".
    await previewApi('meeting_summarize_default', { id });
    await page.clock.setFixedTime(at(stopAt + 20));
    await openRow('Verbale');
    await expect(panel.getByRole('alert')).toContainText(
      'Configura un assistente in Impostazioni per avere la sintesi.',
    );
    await expect(panel.getByRole('button', { name: 'Riprova', exact: true })).toBeVisible();

    // The assistant is set up; "Riprova" runs the job again: in the Sintesi tab the phase, a bar
    // and "Annulla", while the transcript stays readable.
    await previewApi('meeting_preview_readiness', { summary_ready: true });
    await panel.getByRole('button', { name: 'Riprova', exact: true }).click();
    await expect(panel.getByRole('heading', { name: 'Sto scrivendo la sintesi' })).toBeVisible();
    await expect(panel.getByRole('progressbar')).toBeVisible();
    await expect(panel.getByRole('button')).toHaveText(['Annulla']);
    await tabOf('Trascrizione').click();
    await expect(pane).toContainText(SPOKEN);
    await tabOf('Sintesi').click();
    // Cancelled, it can be tried again; finished, the summary is there.
    await panel.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(panel.getByRole('button', { name: 'Riprova', exact: true })).toBeVisible();
    await panel.getByRole('button', { name: 'Riprova', exact: true }).click();
    await page.clock.setFixedTime(at(stopAt + 30));
    await openRow('Verbale');
    await expect(panel).toContainText(SUMMARY);
    assert.ok((await readMeeting(id)).active_summary);
  });

  await step('a_transcript_without_a_summary_can_be_generated', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { summary_ready: false });
    const { id, stopAt } = await recordFor('Senza sintesi', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await previewApi('meeting_preview_readiness', { summary_ready: true });
    await openMeetings();
    await openRow('Senza sintesi');
    const panel = detailPane().getByRole('tabpanel');
    await panel.getByRole('button', { name: 'Genera sintesi', exact: true }).click();
    await expect(panel.getByRole('heading', { name: 'Sto scrivendo la sintesi' })).toBeVisible();
    assert.equal((await readMeeting(id)).jobs.at(-1).kind, 'summarize');
    await page.clock.setFixedTime(at(stopAt + 20));
    await openRow('Senza sintesi');
    await expect(panel).toContainText(SUMMARY);
  });

  await step('kept_audio_says_why_and_can_be_deleted', async () => {
    await fresh();
    // A recording with gaps keeps its audio after the transcript: the page says why.
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    const { id } = await recordFor('Chiamata disturbata', 120);
    await previewApi('meeting_preview_interrupt', { id });
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await previewApi('meeting_recover', { id });
    await page.clock.setFixedTime(at(120 + 10));
    await openMeetings();
    await openRow('Chiamata disturbata');
    const pane = detailPane();
    const note = pane.locator('.audio-note');
    await expect(note).toContainText("Registrazione con interruzioni: l'audio è conservato.");
    await expect(pane.getByRole('tab', { name: 'Sintesi', exact: true })).toBeVisible();

    // Deleting it asks first.
    await note.getByRole('button', { name: 'Elimina audio', exact: true }).click();
    const confirm = page.getByRole('dialog', { name: 'Elimina audio' });
    await expect(confirm).toContainText("L'audio di questa riunione verrà eliminato");
    await confirm.getByRole('button', { name: 'Annulla', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    assert.equal((await readMeeting(id)).audio_removed, false);
    await note.getByRole('button', { name: 'Elimina audio', exact: true }).click();
    await confirm.getByRole('button', { name: 'Elimina audio', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    assert.equal((await readMeeting(id)).audio_removed, true);
    await expect(note).toContainText("L'audio è stato eliminato dopo la trascrizione.");
    await expect(note.getByRole('button')).toHaveCount(0);
  });

  await step('empty_transcript_says_so_and_offers_delete_and_retry', async () => {
    await fresh();
    await previewApi('meeting_preview_empty_transcript');
    const first = await recordFor('Silenziosa', 120);
    await page.clock.setFixedTime(at(first.stopAt + 10));
    await openMeetings();
    await openRow('Silenziosa');
    const pane = detailPane();
    const state = pane.locator('.state');
    const WHY = "Trascrizione vuota: l'audio è conservato. Controlla i dispositivi in Impostazioni.";
    await expect(state).toContainText(WHY);
    await expect(state.getByRole('button')).toHaveText(['Elimina audio', 'Trascrivi di nuovo']);
    await expect(rowOf('Silenziosa').locator('.item-sub')).toContainText('trascrizione vuota');

    // "Trascrivi di nuovo" runs the transcription again (the preview's silence was for one run).
    await state.getByRole('button', { name: 'Trascrivi di nuovo', exact: true }).click();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    await page.clock.setFixedTime(at(first.stopAt + 20));
    await openRow('Silenziosa');
    await expect(pane.getByRole('tabpanel')).toContainText(SUMMARY);
    await expect(pane).toContainText("L'audio è stato eliminato dopo la trascrizione.");

    // Another silent one: "Elimina audio" asks, then the audio is gone and there is nothing to retry.
    await previewApi('meeting_preview_empty_transcript');
    const second = await recordFor('Muta', 120);
    await page.clock.setFixedTime(at(second.stopAt + 10));
    await reopenMeetings();
    await openRow('Muta');
    await expect(state).toContainText(WHY);
    await state.getByRole('button', { name: 'Elimina audio', exact: true }).click();
    const confirm = page.getByRole('dialog', { name: 'Elimina audio' });
    await confirm.getByRole('button', { name: 'Elimina audio', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    assert.equal((await readMeeting(second.id)).audio_removed, true);
    await expect(state).toContainText('L’audio è stato eliminato: non c’è più nulla da trascrivere.');
    await expect(state.getByRole('button')).toHaveCount(0);
  });

  await step('an_unreadable_meeting_says_so_and_can_be_retried', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Illeggibile', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await previewApi('meeting_preview_read_error', { id });
    await openMeetings();
    await rowOf('Illeggibile').locator('.item-main').click();
    const pane = detailPane();
    await expect(pane).toContainText('Non riesco a leggere questa riunione.');
    await previewApi('meeting_preview_read_error', { id, enabled: false });
    await pane.getByRole('button', { name: 'Riprova', exact: true }).click();
    await expect(pane.getByRole('heading', { name: 'Illeggibile', level: 2 })).toBeVisible();
  });

  // A real command takes a moment to answer (the preview answers at once, unless told): the
  // control that was used must still have the focus while it waits and when the answer comes, or
  // a keyboard user is thrown back to the top of the page.
  await step('focus_stays_on_the_control_while_a_slow_command_runs', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Allineamento', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await openMeetings();
    await openRow('Allineamento');
    const pane = detailPane();
    const view = pane.locator('.detail-view');
    await previewApi('meeting_preview_delay', { ms: 700 });

    // An action ticked with the keyboard: the box shows it at once and keeps the focus all along. A
    // second press while the answer is on its way is kept and saved after it: the last one wins.
    const action = pane.getByRole('checkbox', { name: ACTION });
    const revision = (await readMeeting(id)).revision;
    await action.focus();
    await page.keyboard.press('Space');
    await expect(view).toHaveAttribute('aria-busy', 'true');
    await expect(action).toBeChecked();
    await expect(action).toBeFocused();
    await page.keyboard.press('Space');
    await expect(action).not.toBeChecked();
    await expect(action).toBeFocused();
    await expect
      .poll(async () => (await readMeeting(id)).revision, { timeout: 10_000 })
      .toBe(revision + 2);
    await expect(view).not.toHaveAttribute('aria-busy', 'true');
    await expect(action).toBeFocused();
    await expect(action).not.toBeChecked();
    assert.equal((await readMeeting(id)).summaries[0].actions[0].done, false, 'two presses, two saves');
    // A single press ends ticked, and the box and the meeting agree.
    await page.keyboard.press('Space');
    await expect(view).toHaveAttribute('aria-busy', 'true');
    await expect(view).not.toHaveAttribute('aria-busy', 'true');
    await expect(action).toBeChecked();
    await expect(action).toBeFocused();
    assert.equal((await readMeeting(id)).summaries[0].actions[0].done, true);
    await previewApi('meeting_preview_delay', { ms: 0 });

    // The same on the live panel: Pausa, with Enter.
    await record('Dal vivo', { stop: false });
    await reopenMeetings();
    await openRow('Dal vivo');
    const live = pane.locator('.live');
    const pause = live.getByRole('button', { name: 'Pausa', exact: true });
    await pause.evaluate((node) => (node.dataset.probe = 'pause'));
    const same = page.locator('[data-probe="pause"]');
    await previewApi('meeting_preview_delay', { ms: 700 });
    await pause.focus();
    await page.keyboard.press('Enter');
    await expect(live).toHaveAttribute('aria-busy', 'true');
    await expect(same).toBeFocused();
    await expect(live).not.toHaveAttribute('aria-busy', 'true');
    await expect(same).toHaveText('Riprendi');
    await expect(same).toBeFocused();
    await previewApi('meeting_preview_delay', { ms: 0 });
    await previewApi('meeting_stop');
  });

  // An answer that comes back after the user went to another meeting still updates the row of the
  // one it belongs to: "Termina", then a click on another meeting, and the first one moves on.
  await step('a_slow_answer_for_a_meeting_left_behind_still_refreshes_its_row', async () => {
    await fresh();
    await record('Allineamento prodotto', { stop: false });
    await openMeetings();
    await openRow('Allineamento prodotto');
    const pane = detailPane();
    assert.deepEqual(await titlesIn('live'), ['Allineamento prodotto']);
    await previewApi('meeting_preview_delay', { ms: 600 });
    await pane.locator('.live').getByRole('button', { name: 'Termina', exact: true }).click();
    await rowOf('Weekly sync').locator('.item-main').click();
    await expect(pane.getByRole('heading', { name: 'Weekly sync', level: 2 })).toBeVisible();
    await expect(group('live')).toHaveCount(0);
    assert.deepEqual(await titlesIn('processing'), ['Allineamento prodotto']);
    await previewApi('meeting_preview_delay', { ms: 0 });
  });

  // ─── Task 14: the refresh, the links from outside, the polish of the list ──────────────────────

  await step('no_polling_at_rest', async () => {
    await fresh();
    const { stopAt } = await recordFor('Riunione ferma', 120);
    await page.clock.setFixedTime(at(stopAt + 10)); // transcribed, summarized, audio deleted
    // The indicator hears the stop (it polls once a second) before the page opens: the page starts
    // from the state it will stay in.
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    await openMeetings();
    await openRow('Riunione ferma');
    await expect(detailPane().getByRole('tabpanel')).toContainText(SUMMARY);

    const from = await callMark();
    const appFrom = await page.evaluate(() => window.__previewCalls.length);
    await page.waitForTimeout(3000);
    assert.deepEqual(await pageCalls(from), [], 'a page at rest asks the backend for nothing');
    // The app-level commands go through another counter. Only the ones that belong to this page
    // count (the calendar and the join); the app has background traffic of its own (the mail
    // warm-up, the syncs, the workspace) that has nothing to do with it and comes whenever it comes.
    const ofThePage = (command) =>
      command.startsWith('meeting_') || command === 'list_events' || command === 'event_join_record';
    assert.deepEqual(
      (await page.evaluate((n) => window.__previewCalls.slice(n), appFrom)).filter(ofThePage),
      [],
      'no command of the page, from a page at rest',
    );
    // The instrument works: the indicator did poll the capture during those three seconds.
    const all = await page.evaluate((n) => window.__meetingLog.slice(n), from);
    assert.ok(countOf(all, 'meeting_capture_status') >= 2, 'the indicator keeps polling on its own');
  });

  await step('an_untitled_meeting_has_a_row_with_a_name', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Da perdere', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await previewApi('meeting_preview_untitle', { id });
    assert.equal((await readMeeting(id)).meeting.title, '');
    await openMeetings();
    // The row says what the header of the detail says: no blank line to press.
    const row = page.locator(`.meetings-page [data-row-key="m:${id}"]`);
    await expect(row.locator('.item-title')).toHaveText('Riunione senza nome');
    await row.getByRole('button').click();
    await expect(detailPane().getByRole('heading', { name: 'Riunione senza nome', level: 2 })).toBeVisible();
  });

  await step('a_recording_started_elsewhere_appears_without_reload', async () => {
    await fresh();
    await openMeetings();
    await expect.poll(async () => (await busNow()).state).toBe('idle');
    await expect(group('live')).toHaveCount(0);
    await expect(recordNow()).toBeEnabled();

    // The reminder window, the Calendar or another window starts a recording: the page is not told,
    // it hears the indicator publish the new state, and reads the list once.
    const from = await callMark();
    await startRecording('Avviata altrove');
    await expect(rowOf('Avviata altrove')).toBeVisible();
    assert.deepEqual(await titlesIn('live'), ['Avviata altrove']);
    await expect(rowOf('Avviata altrove').locator('.item-dot')).toBeVisible();
    await expect(recordNow()).toBeDisabled();
    // The indicator goes on publishing every second: that is no reason to read the list again.
    await page.waitForTimeout(2500);
    const afterStart = await pageCalls(from);
    assert.equal(countOf(afterStart, 'meeting_list_page'), 1, 'one list read for the start');
    assert.equal(countOf(afterStart, 'meeting_read'), 0, 'no meeting was open, none was read');

    // Open it: the live panel. It is stopped elsewhere (a "Termina" the page did not see): the panel
    // turns into the progress of the transcription by itself, and "Registra ora" comes back.
    await rowOf('Avviata altrove').locator('.item-main').click();
    await expect(detailPane().locator('.live')).toBeVisible();
    const beforeStop = await callMark();
    await page.clock.setFixedTime(at(90));
    await previewApi('meeting_stop');
    await expect(
      detailPane().getByRole('heading', { name: 'Sto trascrivendo la riunione' }),
    ).toBeVisible();
    await expect(detailPane().locator('.live')).toHaveCount(0);
    await expect(group('live')).toHaveCount(0);
    assert.deepEqual(await titlesIn('processing'), ['Avviata altrove']);
    await expect(recordNow()).toBeEnabled();
    assert.equal(countOf(await pageCalls(beforeStop), 'meeting_list_page'), 1, 'one list read for the stop');
  });

  await step('processing_row_updates_then_the_timer_stops', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Elaborazione lenta', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7)); // half way through the transcription
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    await openMeetings();
    const row = rowOf('Elaborazione lenta');
    await expect(row.locator('.item-sub')).toHaveText('Trascrizione 50%');
    await openRow('Elaborazione lenta');
    const bar = detailPane().getByRole('progressbar');
    await expect(bar).toHaveAttribute('aria-valuenow', '50');

    // One timer of a second, for this meeting only: the meeting is read, the list is not.
    const from = await callMark();
    await page.waitForTimeout(2600);
    const reads = await pageCalls(from);
    assert.deepEqual([...new Set(reads.map((call) => call.command))], ['meeting_read']);
    assert.ok(reads.length >= 2 && reads.length <= 3, `${reads.length} reads in 2.6 s`);
    assert.ok(reads.every((call) => call.args.id === id));

    // The clock moves on: the row and the open detail follow with no click.
    await page.clock.setFixedTime(at(stopAt + 2.6));
    await expect(row.locator('.item-sub')).toHaveText('Trascrizione 75%');
    await expect(row.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '75');
    await expect(bar).toHaveAttribute('aria-valuenow', '75');
    // The transcript is ready, the summary is being written: still in elaborazione.
    await page.clock.setFixedTime(at(stopAt + 3.5));
    await expect(row.locator('.item-sub')).toHaveText('Sintesi…');
    assert.deepEqual(await titlesIn('processing'), ['Elaborazione lenta']);

    // A window nobody is looking at asks for nothing.
    await page.evaluate(() =>
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => true }),
    );
    const hiddenFrom = await callMark();
    await page.waitForTimeout(2300);
    assert.deepEqual(await pageCalls(hiddenFrom), [], 'a hidden window is not read');
    await page.evaluate(() => {
      delete document.hidden;
    });

    // Everything is done: the open detail turns into the summary, the row leaves elaborazione, the
    // list is read once, and then the timer stops by itself.
    const doneFrom = await callMark();
    await page.clock.setFixedTime(at(stopAt + 20));
    await expect(detailPane().getByRole('tabpanel')).toContainText(SUMMARY);
    await expect(group('processing')).toHaveCount(0);
    assert.ok((await titlesIn('past')).includes('Elaborazione lenta'));
    await expect(row.getByRole('progressbar')).toHaveCount(0);
    await page.waitForTimeout(1500);
    assert.equal(countOf(await pageCalls(doneFrom), 'meeting_list_page'), 1, 'the list is read once');
    const quietFrom = await callMark();
    await page.waitForTimeout(2500);
    assert.deepEqual(await pageCalls(quietFrom), [], 'the timer stopped by itself');
  });

  await step('assistant_source_opens_the_transcript_segment', async () => {
    await fresh();
    await previewApi('meeting_preview_long_transcript');
    const { id, stopAt } = await recordFor('Pianificazione trimestre', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    // Fifty-five newer meetings: this one is on the second page of the list, not on the first.
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      for (let i = 0; i < 55; i++) await api.meetingCall('meeting_import', {});
    });
    const version = (await readMeeting(id)).active_transcript;
    const source = (segment) =>
      sourceHit({
        meeting_id: id,
        title: 'Pianificazione trimestre',
        kind: 'transcript',
        text: `Passiamo all'argomento ${segment}: cosa serve per chiuderlo e chi se ne occupa.`,
        at_ms: (segment - 1) * 20_000,
        version,
        segment_id: `segment-${segment}`,
      });

    // The Assistant's source button asks the page, then the app goes there (App.tsx does both).
    // The page was not open: it takes the request when it opens.
    await requestOpen(source(31));
    await toMeetings();
    await expect(page.locator('.meetings-page [data-list-ready]')).toBeAttached();
    const pane = detailPane();
    await expect(pane.getByRole('heading', { name: 'Pianificazione trimestre', level: 2 })).toBeVisible();
    await expect(pane).toHaveAttribute('data-key', `m:${id}`);
    await expect(pane).toHaveAttribute('data-tab', 'transcript');
    await expect(pane).toHaveAttribute('data-segment', 'segment-31');
    await expect(tabOf('Trascrizione')).toHaveAttribute('aria-selected', 'true');
    // The line is marked and was scrolled to (forty lines do not fit).
    const line = pane.locator('[data-segment-id="segment-31"]');
    await expect(line).toHaveAttribute('aria-current', 'true');
    await expect(line).toBeInViewport();
    await expect(pane.locator('[data-segment-id="segment-1"]')).not.toBeInViewport();
    // The first page did not reach this meeting, and it is selected anyway, with its row.
    await expect(rowOf('Pianificazione trimestre').locator('.item-main')).toHaveAttribute(
      'aria-current',
      'true',
    );

    // Already on the page, on another meeting: the same request moves it, to the new line.
    await group('past').locator('[data-row-key^="m:"]').first().getByRole('button').click();
    await expect(pane).not.toHaveAttribute('data-key', `m:${id}`);
    await requestOpen(source(5));
    await expect(pane.getByRole('heading', { name: 'Pianificazione trimestre', level: 2 })).toBeVisible();
    await expect(pane).toHaveAttribute('data-segment', 'segment-5');
    await expect(pane.locator('[data-segment-id="segment-5"]')).toHaveAttribute('aria-current', 'true');
    await expect(pane.locator('[data-segment-id][aria-current="true"]')).toHaveCount(1);

    // With a search on screen the request clears it, so the list and the open row can be seen.
    await searchBox().fill('argomento');
    await expect(page.locator('.meetings-page .hit-item').first()).toBeVisible();
    await requestOpen(source(12));
    await expect(searchBox()).toHaveValue('');
    await expect(page.locator('.meetings-page .hit-item')).toHaveCount(0);
    await expect(pane).toHaveAttribute('data-segment', 'segment-12');
    await expect(rowOf('Pianificazione trimestre').locator('.item-main')).toHaveAttribute(
      'aria-current',
      'true',
    );
  });

  await step('notes_hit_from_the_indicator_opens_the_summary', async () => {
    await fresh();
    const ready = await recordFor('Revisione budget Q4', 120);
    await page.clock.setFixedTime(at(ready.stopAt + 10));
    // A recording is going on: the indicator floats over every page, the Inbox included.
    const live = await startRecording('Allineamento prodotto');
    const indicator = page.getByRole('button', { name: /Apri registrazione/ });
    await expect(indicator).toBeVisible();
    await indicator.click(); // asks the page for the meeting with a hit of kind "notes", and goes there
    await expect(page.locator('section.meetings-page')).toBeVisible();
    const pane = detailPane();
    await expect(pane).toHaveAttribute('data-key', `m:${live.meeting.id}`);
    await expect(pane).toHaveAttribute('data-tab', 'summary');
    await expect(pane.locator('.live')).toBeVisible();
    await expect(rowOf('Allineamento prodotto').locator('.item-main')).toHaveAttribute(
      'aria-current',
      'true',
    );

    // A hit that is not the transcript (notes, a note, the summary, the title) opens the summary,
    // also when the transcript was the tab being read.
    await openRow('Revisione budget Q4');
    for (const kind of ['notes', 'note', 'summary', 'title']) {
      await tabOf('Trascrizione').click();
      await expect(tabOf('Trascrizione')).toHaveAttribute('aria-selected', 'true');
      await requestOpen(sourceHit({ meeting_id: ready.id, title: 'Revisione budget Q4', kind }));
      await expect(tabOf('Sintesi')).toHaveAttribute('aria-selected', 'true');
      await expect(pane.getByRole('tabpanel')).toContainText(SUMMARY);
      await expect(pane).toHaveAttribute('data-tab', 'summary');
      await expect(pane).toHaveAttribute('data-segment', '');
    }
  });

  await step('search_keeps_its_results_while_typing_and_announces_the_count_once', async () => {
    await fresh();
    const { stopAt } = await recordFor('Revisione budget Q4', 120);
    await page.clock.setFixedTime(at(stopAt + 10));
    await openMeetings();
    // The groups are named groups, not landmarks: a screen reader would otherwise announce a region
    // for every one of them.
    await expect(group('past')).toHaveAttribute('role', 'group');
    await expect(page.getByRole('group', { name: 'Passate', exact: true })).toHaveCount(1);
    await expect(page.getByRole('region', { name: 'Passate', exact: true })).toHaveCount(0);
    await expect(page.locator('.meetings-page section[data-group]')).toHaveCount(0);

    const hits = page.locator('.meetings-page .hit-item');
    const heading = page.locator('.meetings-page .group-label');
    await searchBox().fill('prototipo');
    await expect(hits).toHaveCount(2);
    await expect(heading).toHaveText('2 risultati per «prototipo»');
    // One polite status, and it carries the count; the results are a named group, not a region.
    const status = page.locator('.meetings-page [role="status"]');
    await expect(status).toHaveCount(1);
    await expect(status).toHaveText('2 risultati per «prototipo»');
    await expect(page.getByRole('group', { name: 'Risultati della ricerca', exact: true })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Risultati della ricerca', exact: true })).toHaveCount(0);

    // Typing on while the backend is slow: the results of the words before stay, with those words
    // marked and that count, instead of flashing back to "Cerco…".
    await page.evaluate(() => {
      window.__meetingDelay = { meeting_search: 1500 };
    });
    await searchBox().pressSequentially(' per');
    await page.waitForTimeout(600); // past the debounce: the request is on its way
    await expect(heading).toHaveText('2 risultati per «prototipo»');
    await expect(hits).toHaveCount(2);
    await expect(hits.first().locator('mark')).toHaveText('prototipo');
    await expect(status).toHaveCount(1);
    // The answer for the new words replaces them.
    await expect(heading).toHaveText('1 risultato per «prototipo per»');
    await expect(hits).toHaveCount(1);
    await expect(hits.first().locator('mark')).toHaveText(['prototipo', 'per']);
    await page.evaluate(() => {
      window.__meetingDelay = {};
    });

    // Emptying the field brings the list back; there is no status then.
    await searchBox().fill('');
    await expect(hits).toHaveCount(0);
    await expect(status).toHaveCount(0);
    await expect(group('past')).toBeVisible();
  });

  await step('join_names_the_meeting_and_ignores_a_second_click', async () => {
    joinOffered = true;
    try {
      await fresh(); // the page loads again: this time its list is served as the desktop's
      await saveEvent({
        title: 'Colloquio Rossi',
        start: T0 + HOUR,
        end: T0 + 2 * HOUR,
        attendees: people(2),
        meeting_link: MEET,
      });
      await openMeetings();
      // Many rows, many identical buttons: each one says which meeting it is for, and starts with
      // the words that are on screen.
      const join = page.getByRole('button', { name: 'Partecipa e registra: Colloquio Rossi', exact: true });
      await expect(join).toBeVisible();
      await expect(join).toContainText('Partecipa e registra');

      // The preview has no desktop to open the call, so the join fails, slowly. While it runs the
      // button says it is disabled and a second press (here: two in the same instant) starts nothing.
      await page.evaluate(() => {
        (window.__previewCommandDelay ??= {}).event_join_record = 1200;
      });
      const joins = () =>
        page.evaluate(() => window.__previewCalls.filter((name) => name === 'event_join_record').length);
      await join.evaluate((button) => {
        button.click();
        button.click();
      });
      await expect(join).toBeDisabled();
      assert.equal(await joins(), 1, 'two clicks, one join');
      await expect(join).toBeEnabled();
      assert.equal(await joins(), 1);
      // It works again once the call has ended; pressed with the keyboard it keeps the focus while
      // it waits (a `disabled` button would drop it).
      await join.focus();
      await page.keyboard.press('Enter');
      await expect(join).toBeDisabled();
      await expect(join).toBeFocused();
      await expect.poll(joins).toBe(2);
      await expect(join).toBeEnabled();
      await expect(join).toBeFocused();
    } finally {
      joinOffered = false;
    }
  });

  await step('show_more_meetings_waits_for_the_answer', async () => {
    await fresh();
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      for (let i = 0; i < 55; i++) await api.meetingCall('meeting_import', {});
    });
    await openMeetings();
    const more = page.getByRole('button', { name: 'Mostra altre riunioni', exact: true });
    await expect(more).toBeVisible();
    await page.evaluate(() => {
      window.__meetingDelay = { meeting_list_page: 1200 };
    });
    const from = await callMark();
    await more.focus();
    await page.keyboard.press('Enter');
    // While the next meetings are on their way the button says so, cannot be pressed again, and
    // the keyboard stays on it.
    const waiting = page.getByRole('button', { name: 'Carico altre riunioni…', exact: true });
    await expect(waiting).toBeDisabled();
    await expect(waiting).toBeFocused();
    await waiting.evaluate((button) => {
      button.click();
      button.click();
    });
    await expect(waiting).toHaveCount(0); // answered: there is no more
    await expect(more).toHaveCount(0);
    const asked = (await pageCalls(from)).filter((call) => call.command === 'meeting_list_page');
    assert.deepEqual(
      asked.map((call) => call.args.offset),
      [50],
      'the next page was asked for once',
    );
    await page.evaluate(() => {
      window.__meetingDelay = {};
    });
  });

  // The start of a recording takes a moment (it opens the audio devices): Enter in the name field
  // starts it, a second Enter while it waits starts nothing, and the focus stays in the popover.
  await step('record_now_keeps_the_focus_while_the_start_is_on_its_way', async () => {
    await fresh();
    await openMeetings();
    await previewApi('meeting_preview_delay', { ms: 700 });
    await recordNow().click();
    const dialog = page.getByRole('dialog', { name: 'Registra ora' });
    const name = dialog.getByLabel('Nome (facoltativo)');
    await name.fill('Chiamata lenta');
    const from = await callMark();
    await name.press('Enter');
    const submit = dialog.getByRole('button', { name: 'Registra', exact: true });
    await expect(submit).toHaveAttribute('aria-busy', 'true');
    await expect(submit).toHaveAttribute('aria-disabled', 'true');
    await expect(submit).toHaveJSProperty('disabled', false); // not `disabled`: that would drop the focus
    await name.press('Enter');
    await expect(dialog).toBeVisible();
    await expect(name).toBeFocused();
    await expect(dialog).toHaveCount(0); // started
    const starts = (await pageCalls(from)).filter((call) => call.command === 'meeting_start');
    assert.equal(starts.length, 1, 'two Enter, one recording');
    // The popover gives the focus back to "Registra ora", which now waits for the recording to
    // end: it stays a button the keyboard can be on (not `disabled`, which would drop the focus).
    await expect(recordNow()).toHaveAttribute('aria-disabled', 'true');
    await expect(recordNow()).toHaveJSProperty('disabled', false);
    await expect(recordNow()).toBeFocused();
    await expect.poll(() => titlesIn('live')).toEqual(['Chiamata lenta']); // the list read is slow too
    await previewApi('meeting_preview_delay', { ms: 0 });
    await previewApi('meeting_stop');
  });

  await step('record_now_says_why_it_waits', async () => {
    await fresh();
    await startRecording('Allineamento prodotto');
    await expect.poll(async () => (await busNow()).state).toBe('recording');
    await openMeetings();
    const why = 'Una registrazione è già in corso';
    await expect(recordNow()).toBeDisabled();
    await expect(page.getByText(why, { exact: true })).toBeVisible();
    await expect(recordNow()).toHaveAccessibleDescription(why);
    // Pressed, it opens nothing (`force`: Playwright does not press what says it is disabled).
    await recordNow().click({ force: true });
    await expect(page.getByRole('dialog', { name: 'Registra ora' })).toHaveCount(0);
    await recordNow().focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog', { name: 'Registra ora' })).toHaveCount(0);
    await previewApi('meeting_stop');
    await expect(recordNow()).toBeEnabled();
    await expect(page.getByText(why, { exact: true })).toHaveCount(0);
    await expect(recordNow()).not.toHaveAccessibleDescription(why);
  });

  // ─── Task 14, fix round: the gaps between jobs, the depth of a list read, the timer's end ──────

  // The service queues the next job a moment after the previous one ends: after a stop, the
  // transcription; after a transcription, the summary. A reading that lands in that gap sees a
  // meeting with no job, and must not take that for "nothing more is coming".
  await step('a_summary_queued_late_is_followed_without_a_click', async () => {
    await fresh();
    await previewApi('meeting_preview_chain_delay', { ms: 1500 });
    const { stopAt } = await recordFor('Catena lenta', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7)); // half way through the transcription
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    await openMeetings();
    await openRow('Catena lenta');
    const pane = detailPane();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();

    // The transcription ends at +3.0 s; the summary is queued at +4.5 s. In between the meeting has
    // a transcript and no job: that is what the page shows (the button is there, for a moment).
    await page.clock.setFixedTime(at(stopAt + 3.5));
    await expect(pane.getByRole('button', { name: 'Genera sintesi', exact: true })).toBeVisible();
    // The summary is queued: the page was still looking, and the detail follows with no click.
    await page.clock.setFixedTime(at(stopAt + 4.6));
    await expect(pane.getByRole('heading', { name: 'Sto scrivendo la sintesi' })).toBeVisible();
    assert.deepEqual(await titlesIn('processing'), ['Catena lenta']);
    await page.clock.setFixedTime(at(stopAt + 20));
    await expect(pane.getByRole('tabpanel')).toContainText(SUMMARY);
    await expect(pane.getByRole('button', { name: 'Genera sintesi', exact: true })).toHaveCount(0);
    await expect(group('processing')).toHaveCount(0);
    assert.ok((await titlesIn('past')).includes('Catena lenta'));
    // The last job is over: the list was read once and the timer stopped by itself.
    const quiet = await callMark();
    await page.waitForTimeout(2500);
    assert.deepEqual(await pageCalls(quiet), [], 'the timer stopped by itself');
  });

  // With no assistant no summary will ever be queued, and the page cannot know: it waits a few
  // ticks (the grace), reads the list once, and stops.
  await step('a_transcript_with_no_summary_job_stops_the_timer_after_the_grace', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { summary_ready: false });
    const { id, stopAt } = await recordFor('Senza sintesi', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7));
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    await openMeetings();
    await openRow('Senza sintesi');
    const pane = detailPane();
    const mark = await callMark();
    await page.clock.setFixedTime(at(stopAt + 5)); // the transcription is over
    await expect(pane.getByRole('tablist')).toBeVisible();
    await expect(group('processing')).toHaveCount(0);
    // It goes on reading for the grace (about three seconds) and then the list is read, once.
    await expect
      .poll(async () => countOf(await pageCalls(mark), 'meeting_list_page'), { timeout: 8000 })
      .toBe(1);
    const reads = countOf(await pageCalls(mark), 'meeting_read');
    assert.ok(reads >= 3 && reads <= 6, `${reads} reads in the grace`);
    assert.ok((await pageCalls(mark)).every((call) => call.args.id === undefined || call.args.id === id));
    const quiet = await callMark();
    await page.waitForTimeout(2500);
    assert.deepEqual(await pageCalls(quiet), [], 'the timer stopped by itself');
  });

  // The recording is stopped elsewhere and the indicator's poll lands before the service has queued
  // the transcription: the page reads the list, sees no job, and must keep looking for a moment.
  await step('a_stop_heard_before_the_transcription_is_queued_still_shows_it', async () => {
    await fresh();
    await startRecording('Allineamento prodotto');
    await expect.poll(async () => (await busNow()).state).toBe('recording');
    await openMeetings();
    await openRow('Allineamento prodotto');
    const pane = detailPane();
    await expect(pane.locator('.live')).toBeVisible();
    // The engine is blocked for the stop (so the stop queues nothing, as in the gap), and the
    // transcription is queued 1.5 s after the page has heard the stop: a click on "Trascrivi" in
    // another window, or the service catching up. Here it is the second.
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await page.clock.setFixedTime(at(120));
    await previewApi('meeting_stop');
    await expect(group('live')).toHaveCount(0);
    await expect(pane.getByRole('heading', { name: 'Questa riunione non ha ancora una trascrizione' })).toBeVisible();
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    const id = Number((await detailPane().getAttribute('data-key')).slice(2));
    await previewApi('meeting_transcribe_default', { id }); // the job appears, nobody told the page
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    assert.deepEqual(await titlesIn('processing'), ['Allineamento prodotto']);
    await page.clock.setFixedTime(at(140));
    await expect(pane.getByRole('tabpanel')).toContainText(SUMMARY);
  });

  // A list read that began before a command is older than the command's answer: it must not put
  // back the row without the job the command just started.
  await step('a_slow_list_read_does_not_erase_a_job_a_command_just_started', async () => {
    await fresh();
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await recordFor('Da trascrivere', 120); // saved, no job: the engine was blocked
    await startRecording('Registrazione corrente');
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await openMeetings();
    await openRow('Da trascrivere');
    const pane = detailPane();
    const transcribe = pane.getByRole('button', { name: 'Trascrivi', exact: true });
    await expect(transcribe).toBeVisible();
    // From now on, the presence of the group "In elaborazione" is written down every time it changes.
    await page.evaluate(() => {
      const seen = [];
      const read = () => {
        const on = Boolean(document.querySelector('.meetings-page [data-group="processing"]'));
        if (on !== seen.at(-1)) seen.push(on);
      };
      new MutationObserver(read).observe(document.querySelector('.meetings-page'), {
        subtree: true,
        childList: true,
      });
      read();
      window.__processingSeen = seen;
    });

    // The recording is stopped (the engine blocked again: it queues nothing); the page hears it and
    // reads the list; that answer is slow.
    await previewApi('meeting_preview_readiness', { transcription_blocker: NO_ENGINE });
    await page.evaluate(() => {
      window.__meetingDelay = { meeting_list_page: 2000 };
    });
    const mark = await callMark();
    await previewApi('meeting_stop');
    await expect
      .poll(async () => countOf(await pageCalls(mark), 'meeting_list_page'))
      .toBeGreaterThanOrEqual(1);
    // While it is on its way "Trascrivi" is pressed: the answer has the job, the row follows.
    await previewApi('meeting_preview_readiness', { transcription_blocker: null });
    await transcribe.click();
    await expect(pane.getByRole('heading', { name: 'Sto trascrivendo la riunione' })).toBeVisible();
    assert.deepEqual(await titlesIn('processing'), ['Da trascrivere']);
    // The old answer lands: the row is still in elaborazione, and it was never taken out.
    await page.waitForTimeout(2600);
    assert.deepEqual(await titlesIn('processing'), ['Da trascrivere']);
    assert.deepEqual(
      await page.evaluate(() => window.__processingSeen),
      [false, true],
      'the group "In elaborazione" appeared and was never taken away again',
    );
    await page.evaluate(() => {
      window.__meetingDelay = {};
    });
    // And the job is followed to its end.
    await page.clock.setFixedTime(at(140));
    await expect(pane.getByRole('tabpanel')).toContainText(SUMMARY);
    await expect(group('processing')).toHaveCount(0);
  });

  // "Mostra altre riunioni" loaded a second page: reading the list again does not throw it away.
  await step('a_list_read_keeps_the_pages_loaded_with_show_more', async () => {
    await fresh();
    await startRecording('Registrazione corrente');
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      for (let i = 0; i < 55; i++) await api.meetingCall('meeting_import', {});
    });
    await expect.poll(async () => (await busNow()).state).toBe('recording');
    const all = await previewApi('meeting_list');
    const total = all.length;
    const weekly = all.findIndex((item) => item.meeting.title === 'Weekly sync');
    assert.ok(total > 50 && weekly >= 50, `Weekly sync must be on the second page (${weekly} of ${total})`);
    await openMeetings();
    const meetingRows = page.locator('.meetings-page [data-row-key^="m:"]');
    await expect(meetingRows).toHaveCount(50);
    await page.getByRole('button', { name: 'Mostra altre riunioni', exact: true }).click();
    await expect(meetingRows).toHaveCount(total);
    await openRow('Weekly sync');
    await expect(rowOf('Weekly sync').locator('.item-main')).toHaveAttribute('aria-current', 'true');
    // From now on the number of meeting rows is written down every time it changes.
    await page.evaluate(() => {
      const counts = [];
      const read = () => {
        const n = document.querySelectorAll('.meetings-page [data-row-key^="m:"]').length;
        if (n !== counts.at(-1)) counts.push(n);
      };
      new MutationObserver(read).observe(document.querySelector('.meetings-page'), {
        subtree: true,
        childList: true,
      });
      read();
      window.__rowCounts = counts;
    });

    // The recording is stopped elsewhere: the list is read again, and its transcription goes on.
    const mark = await callMark();
    await page.clock.setFixedTime(at(60));
    await previewApi('meeting_stop');
    await expect(group('live')).toHaveCount(0);
    await expect(group('processing')).toHaveCount(1);
    const lists = (await pageCalls(mark)).filter((call) => call.command === 'meeting_list_page');
    assert.ok(lists.length >= 1);
    assert.ok(
      lists.every((call) => call.args.offset === 0 && call.args.limit === total),
      `the read asked for what was on screen: ${JSON.stringify(lists.map((call) => call.args))}`,
    );
    await expect(meetingRows).toHaveCount(total);
    await expect(page.getByRole('button', { name: 'Mostra altre riunioni', exact: true })).toHaveCount(0);
    await expect(rowOf('Weekly sync').locator('.item-main')).toHaveAttribute('aria-current', 'true');
    assert.deepEqual(await page.evaluate(() => window.__rowCounts), [total], 'no row ever went away');
    // The job finishes: the list is read once more, with the same depth.
    await page.clock.setFixedTime(at(80));
    await expect(group('processing')).toHaveCount(0);
    await expect(meetingRows).toHaveCount(total);
    assert.deepEqual(await page.evaluate(() => window.__rowCounts), [total]);
  });

  // A list read that began before "Mostra altre riunioni" and ends after it is older than what the
  // button added: it must not put the list back to the pages it asked for.
  await step('a_list_read_that_ends_after_show_more_does_not_erase_the_page_it_added', async () => {
    await fresh();
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      for (let i = 0; i < 55; i++) await api.meetingCall('meeting_import', {});
    });
    await openMeetings();
    const meetingRows = page.locator('.meetings-page [data-row-key^="m:"]');
    const more = page.getByRole('button', { name: 'Mostra altre riunioni', exact: true });
    await expect(meetingRows).toHaveCount(50);
    // From now on the number of meeting rows is written down every time it changes.
    await page.evaluate(() => {
      const counts = [];
      const read = () => {
        const n = document.querySelectorAll('.meetings-page [data-row-key^="m:"]').length;
        if (n !== counts.at(-1)) counts.push(n);
      };
      new MutationObserver(read).observe(document.querySelector('.meetings-page'), {
        subtree: true,
        childList: true,
      });
      read();
      window.__rowCounts = counts;
    });

    // A recording starts elsewhere: the list is read again, and that answer is held back for two seconds.
    await page.evaluate(() => {
      window.__meetingDelay = { meeting_list_page: 2000 };
    });
    const mark = await callMark();
    await startRecording('Registrazione corrente');
    await expect
      .poll(async () => countOf(await pageCalls(mark), 'meeting_list_page'), { timeout: 10_000 })
      .toBeGreaterThanOrEqual(1);
    // "Mostra altre riunioni" is answered at once, so its answer comes first.
    await page.evaluate(() => {
      window.__meetingDelay = {};
    });
    await more.click();
    await expect.poll(async () => (await page.evaluate(() => window.__rowCounts)).length).toBeGreaterThan(1);
    const total = (await previewApi('meeting_list')).length;
    await page.waitForTimeout(2600); // the older read ends now
    const counts = await page.evaluate(() => window.__rowCounts);
    assert.ok(
      counts.every((n, i) => i === 0 || n >= counts[i - 1]),
      `the pages that were added stay: ${counts.join(' -> ')}`,
    );
    // The read that was dropped is made again, down to what is on screen: the new recording is in the list.
    await expect(meetingRows).toHaveCount(total);
    await expect(more).toHaveCount(0);
    await expect(group('live')).toHaveCount(1);
  });

  // Deeper than one request can give (100), the list read chains requests; a meeting added between
  // two of them pushes the others down a place, so one comes in both answers and must count once.
  // (The keyed list in the DOM would merge a twin by itself; what shows is that the page asks for one
  // more meeting to make up the depth it wanted, which it does only if it did not count the twin.)
  await step('a_deep_list_read_chains_requests_and_counts_each_meeting_once', async () => {
    await fresh();
    await startRecording('Registrazione corrente');
    await page.evaluate(async () => {
      const api = await import('/src/lib/meetings.ts');
      for (let i = 0; i < 105; i++) await api.meetingCall('meeting_import', {});
    });
    await expect.poll(async () => (await busNow()).state).toBe('recording');
    const total = (await previewApi('meeting_list')).length;
    assert.ok(total > 100, `${total} meetings`);
    await openMeetings();
    const meetingRows = page.locator('.meetings-page [data-row-key^="m:"]');
    const more = page.getByRole('button', { name: 'Mostra altre riunioni', exact: true });
    await more.click();
    await expect(meetingRows).toHaveCount(100);
    await more.click();
    await expect(meetingRows).toHaveCount(total);

    // The list is read again down to that depth: two requests (100, then the rest), the answers held
    // back; between them a meeting is added.
    await page.evaluate(() => {
      window.__meetingDelay = { meeting_list_page: 2000 };
    });
    const mark = await callMark();
    await previewApi('meeting_stop');
    // The indicator hears the stop within a second; the first request is then on its way for two.
    await expect
      .poll(async () => countOf(await pageCalls(mark), 'meeting_list_page'), { intervals: [20] })
      .toBeGreaterThanOrEqual(1);
    await previewApi('meeting_import', {});
    await page.evaluate(() => {
      window.__meetingDelay = {}; // the first answer is held back already: the rest come at once
    });
    const asked = (calls) =>
      calls.filter((call) => call.command === 'meeting_list_page').map((call) => [call.args.offset, call.args.limit]);
    await expect
      .poll(async () => asked(await pageCalls(mark)).length, { timeout: 10000 })
      .toBeGreaterThanOrEqual(3);
    await page.waitForTimeout(500); // and no fourth one
    // 100, then the rest (one of which came in the first answer too); then the one that is missing.
    assert.deepEqual(asked(await pageCalls(mark)), [
      [0, 100],
      [100, total - 100],
      [total, 1],
    ]);
    await expect(meetingRows).toHaveCount(total);
    const keys = await meetingRows.evaluateAll((nodes) => nodes.map((node) => node.dataset.rowKey));
    assert.equal(new Set(keys).size, keys.length, 'every meeting is listed once');
    assert.deepEqual(errors, [], `the list broke: ${errors.join('; ')}`);
  });

  // A reading that was on its way when a newer one came in is older than what is on screen.
  await step('a_slow_read_overtaken_by_a_list_read_is_dropped', async () => {
    await fresh();
    const { stopAt } = await recordFor('Elaborazione lenta', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7)); // 50 %
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    await openMeetings();
    const row = rowOf('Elaborazione lenta');
    await expect(row.locator('.item-sub')).toHaveText('Trascrizione 50%');
    await page.evaluate(() => {
      const seen = [];
      const read = () => {
        const rows = [...document.querySelectorAll('.meetings-page [data-row-key]')];
        const mine = rows.find((r) => r.querySelector('.item-title')?.textContent === 'Elaborazione lenta');
        const text = mine?.querySelector('.item-sub')?.textContent ?? null;
        if (text !== seen.at(-1)) seen.push(text);
      };
      new MutationObserver(read).observe(document.querySelector('.meetings-page'), {
        subtree: true,
        childList: true,
        characterData: true,
      });
      read();
      window.__rowTexts = seen;
    });
    // The next reading of the meeting is held back for 2.5 s: it is answered with 50 %.
    await page.evaluate(() => {
      window.__meetingDelay = { meeting_read: 2500 };
    });
    const mark = await callMark();
    await expect
      .poll(async () => countOf(await pageCalls(mark), 'meeting_read'))
      .toBeGreaterThanOrEqual(1);
    // Meanwhile the clock moves on and the page reads the list (a recording starts elsewhere): 75 %.
    await page.clock.setFixedTime(at(stopAt + 2.6));
    await startRecording('Altra');
    await expect(row.locator('.item-sub')).toHaveText('Trascrizione 75%');
    // The held answer arrives: it is older, it is dropped, and the row never goes back to 50 %.
    await page.waitForTimeout(3000);
    await page.evaluate(() => {
      window.__meetingDelay = {};
    });
    await expect(row.locator('.item-sub')).toHaveText('Trascrizione 75%');
    const texts = await page.evaluate(() => window.__rowTexts);
    assert.deepEqual(texts, ['Trascrizione 50%', 'Trascrizione 75%'], 'the row only went forward');
  });

  // The page is left while a job is on: the timer goes with it.
  await step('the_timer_stops_when_the_page_is_left', async () => {
    await fresh();
    const { stopAt } = await recordFor('Elaborazione lenta', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7));
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    const mark = await callMark();
    await openMeetings();
    await expect
      .poll(async () => countOf(await pageCalls(mark), 'meeting_read'))
      .toBeGreaterThanOrEqual(1); // it is following the meeting
    await leaveMeetings();
    await page.waitForTimeout(300); // an answer that was on its way
    const quiet = await callMark();
    await page.waitForTimeout(2600);
    assert.deepEqual(await pageCalls(quiet), [], 'nobody reads the meeting once the page is gone');
    // Coming back starts it again (the job is still there).
    await openMeetings();
    await expect
      .poll(async () => countOf(await pageCalls(quiet), 'meeting_read'))
      .toBeGreaterThanOrEqual(1);
  });

  // Reads that keep failing: three ticks, one notice, and then the page stops asking.
  await step('a_follower_that_cannot_read_says_so_once_and_stops', async () => {
    await fresh();
    const { id, stopAt } = await recordFor('Elaborazione lenta', 120);
    await page.clock.setFixedTime(at(stopAt + 1.7));
    await expect.poll(async () => (await busNow()).state).toBe('saved');
    await openMeetings(); // nothing is open: only the timer reads the meeting
    await previewApi('meeting_preview_read_error', { id });
    const mark = await callMark();
    const notice = page.getByRole('region', { name: 'Notifiche' }).getByText('File meeting non leggibile (simulazione).');
    await expect(notice).toHaveCount(1, { timeout: 10000 });
    const reads = () => page.evaluate(
      ({ from, id }) =>
        window.__meetingLog.slice(from).filter((c) => !c.test && c.command === 'meeting_read' && c.args.id === id).length,
      { from: mark, id },
    );
    assert.equal(await reads(), 3, 'three failed ticks, then it gave up');
    await page.waitForTimeout(2500);
    assert.equal(await reads(), 3, 'and does not try again');
    await expect(notice).toHaveCount(1);
  });

  assert.deepEqual(errors, [], `Uncaught browser errors: ${errors.join('; ')}`);
  console.log(`${steps.length} meeting preview scenarios passed.`);
} finally {
  await browser.close();
  server?.kill();
}
