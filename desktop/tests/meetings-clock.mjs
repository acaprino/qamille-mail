// The clock of the Meetings list on the browser preview (Playwright, it-IT, 1440x900). Synthetic
// data only. The list is built from the time of day, and nothing re-reads it while the page rests:
// "Iniziata da N min" has to move and an event that ended has to leave Prossime on their own, from
// one timeout armed for the next such moment (lib/meetings-view `nextClockBoundary`). Here the
// page's clock is a fake one that starts on Monday 9 November 2026, 10:00 (local time), and the test
// jumps over the minutes: the page is never reloaded and never asked to read again.
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
server?.stdout.on('data', (data) => (serverOutput += data.toString()));
server?.stderr.on('data', (data) => (serverOutput += data.toString()));
for (let attempt = 0; attempt < 40; attempt++) {
  if (server && server.exitCode !== null) throw new Error(`Preview exited: ${serverOutput}`);
  try {
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
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] }).catch((error) => {
  server?.kill();
  throw error;
});
const context = await browser.newContext({
  locale: 'it-IT',
  viewport: { width: 1440, height: 900 },
});
const page = await context.newPage();
page.setDefaultTimeout(15000);
const errors = [];
page.on('pageerror', (error) => errors.push(error.stack || error.message));

const MIN = 60;
const HOUR = 3600;
const T0 = Math.floor(new Date(2026, 10, 9, 10, 0).getTime() / 1000);
const minutes = (count) => count * 60_000;

const appCall = (command, args = {}) =>
  page.evaluate(
    async ({ command, args }) => (await import('/src/lib/api.ts')).call(command, args),
    { command, args },
  );
const saveEvent = (event) =>
  appCall('save_event', {
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
const group = (name) => page.locator(`.meetings-page [data-group="${name}"]`);
// A row that leaves a group fades out (Motion exit) as a picture without data-row-key
// (MeetingRowItem.tsx): only the rows that are in the group count.
const titlesIn = (name) => group(name).locator('[data-row-key] .item-title').allTextContents();
const rowOf = (title) =>
  page.locator('.meetings-page [data-row-key]').filter({
    has: page.locator('.item-title', { hasText: new RegExp(`^${title}$`) }),
  });
/** Everything the page asked of the backend (the commands of the app; the meeting ones are the page's too). */
const askedOfTheBackend = () =>
  page.evaluate(() => window.__previewCalls.filter((name) => name === 'list_events').length);

try {
  // A fake clock that still runs in real time, so only the jumps below move it by minutes.
  await page.clock.install({ time: new Date(T0 * 1000) });
  await page.goto(previewUrl);
  const nav = page.getByRole('navigation', { name: 'Spazio di lavoro' });
  await nav.getByRole('button', { name: 'Meeting', exact: true }).waitFor();
  for (const event of await appCall('list_events')) await appCall('delete_event', { id: event.id });
  // "Standup" begins in 5 minutes and ends at 10:15; "Pranzo" is for later and must not move.
  await saveEvent({ title: 'Standup', start: T0 + 5 * MIN, end: T0 + 15 * MIN });
  await saveEvent({ title: 'Pranzo', start: T0 + 3 * HOUR, end: T0 + 4 * HOUR });
  await nav.getByRole('button', { name: 'Meeting', exact: true }).click();
  await expect(page.locator('section.meetings-page')).toBeVisible();
  await expect(page.locator('.meetings-page [data-list-ready]')).toBeAttached();

  const standup = rowOf('Standup');
  const sub = standup.locator('.item-sub');
  assert.deepEqual(await titlesIn('upcoming'), ['Standup', 'Pranzo']);
  await expect(sub).toHaveCount(0); // nothing to say before it begins

  const reads = await askedOfTheBackend();

  // 10:06: it began a minute ago. Nothing reloaded the page, nothing was asked of the backend.
  await page.clock.fastForward(minutes(6));
  await expect(sub).toContainText('Iniziata da 1 min');

  // The minute moves on with the time of day.
  await page.clock.fastForward(minutes(4)); // 10:10
  await expect(sub).toContainText('Iniziata da 5 min');
  await page.clock.fastForward(minutes(2)); // 10:12
  await expect(sub).toContainText('Iniziata da 7 min');
  assert.deepEqual(await titlesIn('upcoming'), ['Standup', 'Pranzo']);
  await expect(rowOf('Pranzo').locator('.item-sub')).toHaveCount(0);

  // 10:16: it ended. It leaves Prossime for good, without "Partecipa e registra" (it is over) and
  // as a grey row of Passate.
  await page.clock.fastForward(minutes(4));
  await expect.poll(() => titlesIn('upcoming')).toEqual(['Pranzo']);
  assert.ok((await titlesIn('past')).includes('Standup'), 'the ended event is in Passate');
  await expect(standup.locator('.item-sub')).toHaveText('Non registrata');

  // The next event is still waiting for its own start, hours away.
  await page.clock.fastForward(minutes(180)); // 13:16, Pranzo began at 13:00
  await expect(rowOf('Pranzo').locator('.item-sub')).toContainText('Iniziata da 16 min');

  assert.equal(await askedOfTheBackend(), reads, 'the clock of the list asked the backend for nothing');
  assert.deepEqual(errors, [], `Uncaught browser errors: ${errors.join('; ')}`);
  console.log('PASS the list clock moves "Iniziata da N min" and drops an ended event, by itself');
} finally {
  await browser.close();
  server?.kill();
}
