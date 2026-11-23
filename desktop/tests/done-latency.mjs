import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { openAllGroups } from './support/groups.mjs';
// Browser preview only: it slows the mock backend through `window.__previewCommandDelay`
// (src/lib/preview.ts). INV-006: a triage action disables only the element acted on, and
// only until the backend has accepted it; opening the next message never holds the interface.
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({
  locale: 'en-US',
  viewport: { width: 1440, height: 900 },
});
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
page.setDefaultTimeout(15000);
const url = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const subjects = {
  a: 'Q4 proposal: can you review by Thursday?',
  b: 'Design review moved to 3 pm',
  c: 'Dinner on Saturday?',
  d: '[qamille] New issue: Search crashes on empty query',
};
const reader = page.getByRole('region', { name: 'Reading pane' });
const done = reader.getByRole('button', { name: 'Done', exact: true });
const archive = reader.getByRole('button', { name: 'Archive message', exact: true });
const heading = (subject) => reader.getByRole('heading', { name: subject, exact: true });
const row = (subject) => page.locator('.mail-row', { hasText: subject });
const notices = page.getByRole('region', { name: 'Recent email actions' });
const undos = notices.getByRole('button', { name: /^Undo:/ });
const delay = (commands) =>
  page.evaluate((value) => (window.__previewCommandDelay = value), commands);
const calls = () => page.evaluate(() => [...window.__previewCalls]);
const count = (list, name) => list.filter((call) => call === name).length;
// The mock store is in memory: reloading gives the fixture back. The first message opens
// by itself and is marked read, which must be over before a delay is installed.
const ready = async () => {
  await page.goto(url);
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  await expect(heading(subjects.a)).toBeVisible();
  await expect(row(subjects.a)).not.toHaveClass(/unread/);
  await expect(row(subjects.b)).toHaveClass(/unread/);
  // The quiet groups start closed; the messages this test walks through live in them.
  await openAllGroups(page);
};
const results = [];
async function step(name, fn) {
  await fn();
  results.push(name);
  console.log(`PASS ${name}`);
}
try {
  await step('Done does not blank or lock the reader while the next message loads', async () => {
    await ready();
    // Opening the next (unread) message needs read_mail and then mail_action markAsSeen, each
    // 1.5 s here. The first is unavoidable, but the reader keeps its toolbar meanwhile, and the
    // second must not keep the next message's toolbar disabled.
    await delay({ read_mail: 1500, mail_action: 1500 });
    const started = Date.now();
    await done.click();
    // Within 200 ms: the toolbar is there and usable (the backend accepted Done at once), and the
    // message just sent away is still on screen instead of a blank page.
    await expect(archive).toBeEnabled({ timeout: 200 });
    await expect(done).toBeEnabled({ timeout: 200 });
    await expect(heading(subjects.a)).toBeVisible();
    await expect(page.locator('.reader-empty')).toHaveCount(0);
    await expect(heading(subjects.b)).toBeVisible();
    const shown = Date.now() - started;
    await expect(archive).toBeEnabled({ timeout: 200 });
    await expect(done).toBeEnabled({ timeout: 200 });
    assert.ok(shown < 2400, `The next message showed after ${shown} ms, not after both delays`);
  });
  await step('The message just sent away takes no second action while the next one loads', async () => {
    await ready();
    await delay({ read_mail: 1500 });
    const before = count(await calls(), 'queue_mail_action');
    await done.click();
    await expect(archive).toBeEnabled({ timeout: 200 });
    // The message is still on screen with live buttons: pressing them must queue nothing new.
    await expect(heading(subjects.a)).toBeVisible();
    await archive.click();
    await done.click();
    await page.waitForTimeout(300);
    assert.equal(count(await calls(), 'queue_mail_action') - before, 1);
    await expect(undos).toHaveCount(1);
    await expect(heading(subjects.b)).toBeVisible();
  });
  await step('Only the element acted on is disabled while the chain runs', async () => {
    await ready();
    await delay({ read_mail: 1500, mail_action: 1500 });
    const before = count(await calls(), 'queue_mail_action');
    await done.click();
    // Every other control stays usable: no row pin is disabled, and one can be used now.
    await expect(page.locator('.row-pin:disabled')).toHaveCount(0, { timeout: 300 });
    await row(subjects.c).locator('.row-pin').click({ timeout: 300, force: true });
    await expect(undos).toHaveCount(2, { timeout: 500 });
    assert.equal(count(await calls(), 'queue_mail_action') - before, 2);
  });
  await step('Until the backend accepts an action only the message acted on is locked', async () => {
    await ready();
    // The accept is slow, so the busy window is long enough to look into.
    await delay({ queue_mail_action: 1500 });
    const before = count(await calls(), 'queue_mail_action');
    await done.click();
    const acted = [
      done,
      archive,
      reader.getByRole('button', { name: 'Snooze until tomorrow', exact: true }),
      reader.getByRole('button', { name: /^(Pin|Unpin) message$/ }),
      reader.getByRole('button', { name: 'Mark message unread', exact: true }),
      reader.getByRole('button', { name: 'Move message to trash', exact: true }),
      row(subjects.a).locator('.row-pin'),
    ];
    for (const control of acted) await expect(control).toBeDisabled({ timeout: 300 });
    // Another message is untouched: its pin works now and queues its own action.
    const otherPin = row(subjects.c).locator('.row-pin');
    await expect(otherPin).toBeEnabled();
    await otherPin.click({ force: true });
    await expect
      .poll(async () => count(await calls(), 'queue_mail_action') - before, { message: 'Done and the other pin' })
      .toBe(2);
    // The shortcut on the message being acted on queues nothing.
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press('e');
    // Neither does a click that gets past the disabled attribute (a stale DOM): action() itself
    // refuses a message that already has an action waiting for the backend.
    await done.evaluate((button) => (button.disabled = false));
    await done.click();
    await archive.evaluate((button) => (button.disabled = false));
    await archive.click();
    await page.waitForTimeout(300);
    assert.equal(
      count(await calls(), 'queue_mail_action') - before,
      2,
      'Nothing more was queued for the message being acted on',
    );
    await expect(undos).toHaveCount(2);
    await expect(heading(subjects.b)).toBeVisible();
    assert.equal(count(await calls(), 'queue_mail_action') - before, 2);
  });
  await step('Done then Done again on the next message queues each message once', async () => {
    await ready();
    // read_mail is fast, markAsSeen of the next message is slow: it is still running when
    // the second Done is pressed, i.e. the first action's follow-up has not finished.
    await delay({ mail_action: 1500 });
    const before = count(await calls(), 'queue_mail_action');
    await done.click();
    await expect(heading(subjects.b)).toBeVisible();
    await done.click({ timeout: 300 });
    await expect(heading(subjects.c)).toBeVisible();
    await expect(undos).toHaveCount(2);
    await expect(row(subjects.a)).toHaveCount(0);
    await expect(row(subjects.b)).toHaveCount(0);
    await expect(row(subjects.c)).toHaveClass(/selected/);
    // The first action's follow-up ends later: it must not reopen or skip anything.
    await page.waitForTimeout(2000);
    await expect(heading(subjects.c)).toBeVisible();
    await expect(row(subjects.d)).not.toHaveClass(/selected/);
    assert.equal(count(await calls(), 'queue_mail_action') - before, 2);
  });
  await step('Done twice at the end of the list does not step back onto a message just sent away', async () => {
    await ready();
    const last = 'Your flight to Berlin is confirmed';
    const beforeLast = 'Photos from the trip';
    const third = 'Invoice #2291 for October';
    await row(last).locator('.mail-open').click();
    await expect(heading(last)).toBeVisible();
    // The reload is slow, so the row that was just sent away is still listed when the second
    // Done picks the message to show next: it comes after the current one in the list.
    await page.evaluate(() => (window.__previewDelay = 1500));
    const before = count(await calls(), 'queue_mail_action');
    await done.click();
    await expect(heading(beforeLast)).toBeVisible();
    await expect(row(last)).toHaveCount(1);
    await done.click({ timeout: 300 });
    await expect(heading(third)).toBeVisible();
    await page.waitForTimeout(500);
    await expect(heading(third)).toBeVisible();
    assert.equal(count(await calls(), 'queue_mail_action') - before, 2);
    await page.evaluate(() => (window.__previewDelay = 0));
  });
  await step('Pressing Done twice on one message queues it once', async () => {
    await ready();
    await delay({ read_mail: 800 });
    const before = count(await calls(), 'queue_mail_action');
    await done.dblclick();
    await expect(heading(subjects.b)).toBeVisible();
    await expect(undos).toHaveCount(1);
    assert.equal(count(await calls(), 'queue_mail_action') - before, 1);
  });
  await step('The keyboard shortcut archives without waiting for the next message', async () => {
    await ready();
    await delay({ read_mail: 1500, mail_action: 1500 });
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press('e');
    await expect(undos).toHaveCount(1, { timeout: 500 });
    await page.keyboard.press('e');
    await page.waitForTimeout(300);
    await expect(undos).toHaveCount(1);
    await expect(heading(subjects.b)).toBeVisible();
    await page.keyboard.press('e');
    await expect(undos).toHaveCount(2, { timeout: 500 });
  });
  await step('A refused action says why, frees its buttons and reloads the list', async () => {
    await ready();
    const passes = async () =>
      (await page.evaluate(() => window.__previewListQueries)).filter((q) => q.offset === 0)
        .length;
    const passesBefore = await passes();
    await page.evaluate(() => (window.__previewCommandFail = { queue_mail_action: 'Synthetic refusal' }));
    await done.click();
    await expect(page.locator('[role="alert"], [data-scope="toast"][data-part="root"]').filter({ hasText: 'Synthetic refusal' }).first()).toBeVisible();
    await expect(done).toBeEnabled();
    await expect(heading(subjects.a)).toBeVisible();
    await expect(row(subjects.a)).toHaveCount(1);
    await expect.poll(passes).toBeGreaterThan(passesBefore);
    await page.evaluate(() => (window.__previewCommandFail = {}));
  });
  await step('A failing follow-up of an action is reported and the list is reloaded', async () => {
    await ready();
    const passes = async () =>
      (await page.evaluate(() => window.__previewListQueries)).filter((q) => q.offset === 0)
        .length;
    const passesBefore = await passes();
    // Pin is accepted; re-reading the message afterwards fails.
    await page.evaluate(() => (window.__previewCommandFail = { read_mail: 'Synthetic read failure' }));
    await reader.getByRole('button', { name: 'Pin message', exact: true }).click();
    await expect(undos).toHaveCount(1);
    await expect(page.locator('[role="alert"], [data-scope="toast"][data-part="root"]').filter({ hasText: 'Synthetic read failure' }).first()).toBeVisible();
    await expect(reader.getByRole('button', { name: 'Pin message', exact: true })).toBeEnabled();
    // One reload for the action itself and one more for the failure.
    await expect.poll(passes).toBeGreaterThanOrEqual(passesBefore + 2);
    await page.evaluate(() => (window.__previewCommandFail = {}));
  });
  await step('Keyboard focus lands on the next row after Done', async () => {
    await ready();
    await delay({ read_mail: 600 });
    await done.click();
    await expect(heading(subjects.b)).toBeVisible();
    // The Done button is gone: focus goes back to the row that is now open, once the list has
    // been rebuilt by the reload, and it stays there.
    await expect(row(subjects.b).locator('.mail-open')).toBeFocused();
    await page.waitForTimeout(500);
    await expect(row(subjects.b).locator('.mail-open')).toBeFocused();
  });
  await step('A slow re-read of an older action cannot overwrite a newer one', async () => {
    await ready();
    const pinned = reader.getByRole('button', { name: 'Unpin message', exact: true });
    const rowPin = row(subjects.a).locator('.row-pin');
    const readsBefore = count(await calls(), 'read_mail');
    // The re-read of the first action answers last, with the state it saw.
    await delay({ read_mail: [1500, 0] });
    const started = Date.now();
    await reader.getByRole('button', { name: 'Pin message', exact: true }).click();
    // The list reload lands first: the row offers Unpin while the slow re-read is on its way.
    await expect(rowPin).toHaveClass(/pinned/);
    await rowPin.click();
    await expect.poll(async () => count(await calls(), 'read_mail') - readsBefore).toBe(2);
    await page.waitForTimeout(Math.max(0, 1800 - (Date.now() - started)));
    await expect(pinned).toHaveCount(0);
    await expect(reader.getByRole('button', { name: 'Pin message', exact: true })).toBeVisible();
    await expect(rowPin).not.toHaveClass(/pinned/);
  });
  // A conversation of two messages in the Inbox, in a context of its own so the Threads
  // preference does not leak into the steps above.
  const fixture = JSON.parse(
    await fs.readFile(new URL('../src/lib/sample.json', import.meta.url), 'utf8'),
  );
  const root = fixture.emails.find((email) => email.id === 4);
  fixture.emails.push({
    ...structuredClone(root),
    id: 99001,
    message_id: '<done-latency-reply@test>',
    thread_id: root.message_id,
    date: root.date + 60,
    read: true,
    pinned: false,
    body: 'A reply in the same conversation.',
  });
  fixture.next_id = 100000;
  const threadContext = await browser.newContext({
    locale: 'en-US',
    viewport: { width: 1440, height: 900 },
  });
  const tp = await threadContext.newPage();
  tp.on('pageerror', (error) => errors.push(error.message));
  tp.setDefaultTimeout(15000);
  await tp.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
  await tp.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({
      contentType: 'application/javascript',
      body: `export default ${JSON.stringify(fixture)};`,
    }),
  );
  const tReader = tp.getByRole('region', { name: 'Reading pane' });
  const tReaderPin = tReader.getByRole('button', { name: /^(Pin|Unpin) message$/ });
  const tRows = tp.locator('.mail-row');
  // The conversation row says how many messages it holds on its arrow (ADR-0010).
  const tThread = tRows.filter({ has: tp.locator('.thread-toggle', { hasText: '2' }) });
  const tThreadPin = tThread.locator('.row-pin');
  const tOtherPin = tRows.filter({ hasText: subjects.c }).locator('.row-pin');
  const tUndos = tp
    .getByRole('region', { name: 'Recent email actions' })
    .getByRole('button', { name: /^Undo:/ });
  const tQueued = () =>
    tp.evaluate(() => window.__previewCalls.filter((c) => c === 'queue_mail_action').length);
  const threadReady = async () => {
    await tp.goto(url);
    await tp.getByRole('button', { name: /^New message/ }).waitFor();
    await expect(tThread).toHaveCount(1);
    await tThread.locator('.mail-open').click();
    await expect(tThread).toHaveClass(/selected/);
    await expect(tReaderPin).toBeEnabled();
  };
  await step('A conversation is locked as a whole until all its messages are accepted', async () => {
    await threadReady();
    // Each message takes 1.5 s to accept, so the second one starts at 1.5 s and ends at 3 s: the
    // controls have 1.5 s of margin at the moment the second call is seen, not tens of ms.
    await tp.evaluate(() => (window.__previewCommandDelay = { queue_mail_action: 1500 }));
    const before = await tQueued();
    await tThreadPin.click();
    // The reader shows one of the conversation's messages: it is locked too.
    await expect(tThreadPin).toBeDisabled({ timeout: 300 });
    await expect(tReaderPin).toBeDisabled({ timeout: 300 });
    await expect(tOtherPin).toBeEnabled();
    // The second message is being accepted; the first one's controls are still locked.
    await expect.poll(async () => (await tQueued()) - before, { intervals: [25] }).toBe(2);
    await expect(tThreadPin).toBeDisabled();
    await expect(tReaderPin).toBeDisabled();
    await expect(tThreadPin).toBeEnabled({ timeout: 3000 });
    await expect(tReaderPin).toBeEnabled();
    await expect(tUndos).toHaveCount(2);
  });
  await step('A refusal half way through a conversation frees all its messages', async () => {
    await threadReady();
    await tp.evaluate(() => (window.__previewCommandDelay = { queue_mail_action: 600 }));
    const before = await tQueued();
    await tThreadPin.click();
    // The first message has been accepted and the second is being sent: refuse that one (a
    // refusal is decided when the wait of the call ends).
    await expect.poll(async () => (await tQueued()) - before).toBe(2);
    await tp.evaluate(
      () => (window.__previewCommandFail = { queue_mail_action: 'Synthetic refusal' }),
    );
    await expect(tp.locator('[data-scope="toast"][data-part="root"]').filter({ hasText: 'Synthetic refusal' })).toBeVisible();
    await expect(tThreadPin).toBeEnabled();
    await expect(tReaderPin).toBeEnabled();
    await expect(tUndos).toHaveCount(1);
    assert.equal((await tQueued()) - before, 2);
  });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: results.length, errors }));
} catch (error) {
  await page.screenshot({ path: '../checks/done-latency-failure.png' }).catch(() => {});
  throw error;
} finally {
  await browser.close();
}
