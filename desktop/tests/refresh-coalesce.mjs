import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { openAllGroups } from './support/groups.mjs';
// Browser preview only: it drives the mock backend in src/lib/preview.ts, which clamps
// `list_mail` to 1..100 rows per call exactly like the Rust backend (INV-007).
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({
  locale: 'en-US',
  viewport: { width: 1440, height: 900 },
});
const page = await context.newPage();
const errors = [];
const warnings = [];
page.on('pageerror', (error) => errors.push(error.message));
page.on('console', (message) => {
  if (message.type() === 'warning') warnings.push(message.text());
});
page.setDefaultTimeout(15000);
const url = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const ready = async () => {
  await page.goto(url);
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  // The quiet groups start closed; this test counts rows.
  await openAllGroups(page);
};
await ready();
const rows = page.locator('.mail-row');
const loadMore = page.getByRole('button', { name: 'Load more messages', exact: true });
const calls = () => page.evaluate(() => [...window.__previewCalls]);
const queries = () => page.evaluate(() => [...window.__previewListQueries]);
const scrollTop = () => page.locator('.mail-scroll').evaluate((el) => el.scrollTop);
// A refresh pass always starts with a list_mail call at offset 0; "Load more" never does.
const passes = async () => (await queries()).filter((query) => query.offset === 0).length;
// Polls until `count` has stopped increasing for 300 ms (at most 3 s) and returns it.
async function settled(count) {
  let last = await count();
  let since = Date.now();
  const deadline = since + 3000;
  while (Date.now() < deadline) {
    await page.waitForTimeout(50);
    const now = await count();
    if (now !== last) [last, since] = [now, Date.now()];
    else if (Date.now() - since >= 300) return last;
  }
  throw new Error('Refresh passes did not settle within 3 s');
}
// Every list pass takes 250 ms and an event arrives every 100 ms for `span` ms.
const stream = (span) =>
  page.evaluate((ms) => {
    window.__previewDelay = 250;
    window.__streamStart = Date.now();
    window.__streamEnd = window.__streamStart + ms;
    const timer = setInterval(
      () => window.dispatchEvent(new Event('preview-workspace-changed')),
      100,
    );
    setTimeout(() => clearInterval(timer), ms);
  }, span);
const streaming = () => page.evaluate(() => Date.now() < window.__streamEnd - 300);
const subject = 'Q4 proposal: can you review by Thursday?';
const heading = (name) => page.locator('.reader').getByRole('heading', { name, exact: true });
const results = [];
// Typed events (ADR-0007 decision 5): the list is patched row by row, not reloaded.
const announce = (detail) =>
  page.evaluate((d) => window.dispatchEvent(new CustomEvent('app-event', { detail: d })), detail);
const rowIds = () =>
  page
    .locator('.mail-row .visually-hidden[id^="mail-row-"]')
    .evaluateAll((nodes) => nodes.map((node) => Number(node.id.slice('mail-row-'.length))));
// Waits until no backend call has been made for 300 ms (at most 5 s) and returns the call count.
async function quiet() {
  let last = (await calls()).length;
  let since = Date.now();
  while (Date.now() - since < 5000) {
    await page.waitForTimeout(50);
    const now = (await calls()).length;
    if (now !== last) [last, since] = [now, Date.now()];
    else if (Date.now() - since >= 300) return last;
  }
  throw new Error('Backend calls did not settle within 5 s');
}
const callsSince = async (mark) => (await calls()).slice(mark);
const count = (names, name) => names.filter((item) => item === name).length;
const readRowsCalls = () => page.evaluate(() => [...(window.__previewReadRows ?? [])]);
const rowOf = (id) => page.locator('.mail-row', { has: page.locator(`#mail-row-${id}`) });
const edit = (id, fields) =>
  page.evaluate(([target, values]) => window.__previewEdit(target, values), [id, fields]);
const listPasses = async (mark) =>
  (await callsSince(mark)).filter((name) => name === 'list_mail').length;
// A fresh page with the sample Inbox plus `seeded` messages, so that "Load more" has rows to bring.
async function seeded(count) {
  await ready();
  await expect(rows.first()).toBeVisible();
  await page.evaluate((n) => window.__previewSeed(n), count);
  await expect(loadMore).toBeVisible();
  await expect(rows).toHaveCount(60);
  await quiet();
}
const inboxCount = async () =>
  Number(
    (await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).innerText()).replace(/\D+/g, '') || 0,
  );
async function step(name, fn) {
  await fn();
  results.push(name);
  console.log(`PASS ${name}`);
}
try {
  let loaded = 0;
  let scrolled = 0;
  let burst = [];
  await step('Inbox pages are loaded with "Load more"', async () => {
    // The sample fixture has 11 messages; add enough for several pages of 60.
    await page.evaluate(() => window.__previewSeed(250));
    // The seeded messages land in the quiet groups, which start closed.
    await openAllGroups(page);
    await expect(rows).toHaveCount(60);
    await loadMore.click();
    await expect(rows).toHaveCount(120);
    await loadMore.click();
    await expect(rows).toHaveCount(180);
    loaded = await rows.count();
    assert.ok(loaded > 100, 'More rows than one backend call can return are loaded');
    await page.locator('.mail-scroll').evaluate((el) => (el.scrollTop = el.scrollHeight / 2));
    scrolled = await scrollTop();
    assert.ok(scrolled > 0, 'The list is scrolled');
  });
  await step('Three back-to-back change events coalesce into at most two passes', async () => {
    const before = (await queries()).length;
    const callsBefore = (await calls()).length;
    await page.evaluate(() => {
      for (let i = 0; i < 3; i++) window.dispatchEvent(new Event('preview-workspace-changed'));
    });
    const total = await settled(passes);
    const since = (burst = (await queries()).slice(before));
    const names = (await calls()).slice(callsBefore);
    const count = since.filter((query) => query.offset === 0).length;
    assert.ok(count >= 1, `The list is reloaded (${total} passes in all)`);
    assert.ok(count <= 2, `list_mail passes ${count} <= 2`);
    const snapshots = names.filter((name) => name === 'workspace_snapshot').length;
    assert.ok(snapshots <= 2, `workspace_snapshot calls ${snapshots} <= 2`);
  });
  await step('A background refresh keeps the loaded pages and the scroll position', async () => {
    await expect(rows).toHaveCount(loaded);
    assert.equal(await scrollTop(), scrolled, 'scrollTop is unchanged');
    assert.ok(
      burst.every((query) => query.limit >= 1 && query.limit <= 100),
      `No list_mail request exceeds the backend limit of 100 (${JSON.stringify(burst)})`,
    );
    const reread = burst.reduce((sum, query) => sum + query.limit, 0);
    assert.equal(reread % loaded, 0, `Each pass re-reads exactly the loaded rows (${reread})`);
  });
  await step('A background refresh still picks up changes', async () => {
    // The headline no longer shows the total (it says what needs the reader), so the proof that
    // the refresh saw the change is the new message at the top of the group it lands in.
    const topOfSeen = () =>
      page
        .locator('.mail-row[data-section="seen"] .visually-hidden[id^="mail-row-"]')
        .first()
        .evaluate((node) => Number(node.id.slice('mail-row-'.length)));
    const topBefore = await topOfSeen();
    await page.evaluate(() => window.__previewSeed(1));
    await expect.poll(topOfSeen).toBeGreaterThan(topBefore);
    await expect(rows).toHaveCount(loaded);
    // The new row lands above the viewport, so the browser may anchor the scroll by one row.
    assert.ok(Math.abs((await scrollTop()) - scrolled) < 200, 'The list does not jump');
  });
  await step('"Load more" continues right after the refreshed page', async () => {
    await loadMore.click();
    await expect(rows).toHaveCount(loaded + 60);
    const last = (await queries()).at(-1);
    assert.deepEqual(last, { offset: loaded, limit: 60 }, 'No row is skipped or read twice');
    const ids = await page
      .locator('.mail-row .visually-hidden[id^="mail-row-"]')
      .evaluateAll((nodes) => nodes.map((node) => node.id));
    assert.equal(new Set(ids).size, ids.length, 'No row appears twice');
  });
  await step('A typed event keeps the loaded pages and the scroll position', async () => {
    const before = await rowIds();
    const top = await scrollTop();
    assert.ok(before.length > 200, `Many rows are loaded (${before.length})`);
    const mark = await quiet();
    // One row on the page and one that sorts after every loaded row: the page must not shrink.
    const onPage = before[before.length >> 1];
    const beyond = Array.from({ length: 300 }, (_, index) => index + 3).find((id) => !before.includes(id));
    assert.ok(beyond, 'Some stored message is not loaded yet');
    await page.evaluate((id) => window.__previewEdit(id, { read: true, subject: 'Edited elsewhere' }), onPage);
    await announce({ kind: 'email-changed', account_id: 1, ids: [onPage, beyond] });
    await expect(
      page.locator('.mail-row', { has: page.locator(`#mail-row-${onPage}`) }),
    ).toContainText('Edited elsewhere');
    const names = (await callsSince(mark)).filter((name) => ['list_mail', 'read_rows'].includes(name));
    assert.deepEqual(names, ['read_rows'], 'One read, no reload');
    assert.deepEqual(await rowIds(), before, 'No loaded row is dropped or moved');
    assert.equal(await scrollTop(), top, 'scrollTop is unchanged');
    await loadMore.click();
    // The last page brings the oldest messages, which open groups that were not there before and
    // start closed.
    await openAllGroups(page);
    // The sample store holds 262 messages in all: the last page is the rest.
    await expect(rows).toHaveCount(262);
    assert.deepEqual((await queries()).at(-1), { offset: before.length, limit: 60 }, 'Load more continues right after');
  });
  await step('An action is not held up by a stream of refresh events', async () => {
    await ready();
    const reader = page.locator('.reader');
    const done = reader.locator('.done-button');
    await expect(done).toBeEnabled();
    // An action that waited for the refresh chain would stay busy until the end of the stream.
    // It must be over as soon as the backend has accepted it (INV-006).
    await stream(3000);
    await page.waitForTimeout(150);
    const callsBefore = (await calls()).length;
    await reader.getByRole('button', { name: 'Mark message unread', exact: true }).click();
    await expect(done).toBeEnabled({ timeout: 300 });
    await expect
      .poll(async () => (await calls()).slice(callsBefore).includes('queue_mail_action'))
      .toBe(true);
    assert.ok(await streaming(), 'The action was accepted while events were still arriving');
    // The stream is still running: let it end before waiting for the passes to go quiet.
    await page.waitForFunction(() => Date.now() - window.__streamStart > 3200);
    await settled(passes);
    await page.evaluate(() => (window.__previewDelay = 0));
  });
  await step('Undo finishes while refresh events keep arriving', async () => {
    await ready();
    await expect(heading(subject)).toBeVisible();
    await page.locator('.reader .done-button').click();
    const notices = page.getByRole('region', { name: 'Recent email actions' });
    const undo = notices.getByRole('button', { name: /^Undo:/ });
    await expect(undo).toBeVisible();
    await expect(heading(subject)).toHaveCount(0);
    // undoPending() waits for refresh() before it reopens the message. Waiting for a stream
    // that never goes quiet would leave the message closed until the stream ends.
    await stream(5000);
    await page.waitForTimeout(150);
    await undo.click();
    await expect(heading(subject)).toBeVisible({ timeout: 3000 });
    await expect(notices).toHaveCount(0);
    assert.ok(await streaming(), 'Undo finished while events were still arriving');
  });
  await step('Done restores keyboard focus while refresh events keep arriving', async () => {
    await ready();
    await expect(heading(subject)).toBeVisible();
    await stream(5000);
    await page.waitForTimeout(150);
    await page.locator('.reader .done-button').click();
    // Focus is restored once the reload and the opening of the next message are over, and both
    // wait for refresh(): neither may wait for the stream to end.
    const next = page.locator('.mail-row', { hasText: 'Design review moved to 3 pm' });
    await expect(next.locator('.mail-open')).toBeFocused({ timeout: 3000 });
    assert.ok(await streaming(), 'Focus was restored while events were still arriving');
  });
  let sampleIds = [];
  await step('An email-changed event of three ids is read with one read_rows and reloads nothing', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    sampleIds = await rowIds();
    assert.ok(sampleIds.length >= 5, `The sample Inbox has rows (${sampleIds})`);
    const mark = await quiet();
    const three = sampleIds.slice(1, 4);
    await announce({ kind: 'email-changed', account_id: 1, ids: three });
    await expect.poll(async () => count(await callsSince(mark), 'read_rows')).toBe(1);
    await quiet();
    const names = await callsSince(mark);
    assert.equal(count(names, 'read_rows'), 1, `read_rows calls: ${names}`);
    assert.equal(count(names, 'list_mail'), 0, `The list is not reloaded: ${names}`);
    assert.deepEqual((await readRowsCalls()).at(-1), three, 'One call carries the three ids');
    assert.deepEqual(await rowIds(), sampleIds, 'The rows are where they were');
  });
  await step('A row that leaves the folder is removed without a reload', async () => {
    const [, , leaving] = sampleIds;
    const mark = await quiet();
    await page.evaluate((id) => window.__previewEdit(id, { folder: 'Archive' }), leaving);
    await announce({ kind: 'email-changed', account_id: 1, ids: [leaving] });
    await expect(rows).toHaveCount(sampleIds.length - 1);
    await expect(page.locator(`#mail-row-${leaving}`)).toHaveCount(0);
    assert.deepEqual(
      await rowIds(),
      sampleIds.filter((id) => id !== leaving),
      'The others keep their order',
    );
    const names = (await callsSince(mark)).filter((name) => ['list_mail', 'read_rows'].includes(name));
    assert.deepEqual(names, ['read_rows'], 'One read, no reload');
  });
  await step('A row that is not on the page and now belongs to it joins in its place', async () => {
    const [, , returning] = sampleIds;
    const mark = await quiet();
    await page.evaluate((id) => window.__previewEdit(id, { folder: 'Inbox' }), returning);
    await announce({ kind: 'email-changed', account_id: 1, ids: [returning] });
    await expect(rows).toHaveCount(sampleIds.length);
    assert.deepEqual(await rowIds(), sampleIds, 'It is back at its place in the order');
    const names = (await callsSince(mark)).filter((name) => ['list_mail', 'read_rows'].includes(name));
    assert.deepEqual(names, ['read_rows']);
  });
  await step('CountsChanged takes one workspace snapshot and does not reload the list', async () => {
    const unread = await inboxCount();
    assert.ok(unread > 0, 'The sample Inbox has unread messages');
    const mark = await quiet();
    // Message 8 is an unread newsletter in the fixture, not the one the reader holds open.
    await page.evaluate(() => window.__previewEdit(8, { read: true }));
    await announce({ kind: 'counts-changed', account_id: 1 });
    await expect.poll(inboxCount).toBe(unread - 1);
    const names = (await callsSince(mark)).filter((name) => ['list_mail', 'workspace_snapshot'].includes(name));
    assert.deepEqual(names, ['workspace_snapshot'], 'One snapshot, no list reload');
  });
  await step('Undo events change the notices without any backend call', async () => {
    const notices = page.getByRole('region', { name: 'Recent email actions' });
    const mark = await quiet();
    await announce({
      kind: 'undo-changed',
      action: {
        id: 9001,
        operation: { kind: 'mail', email_id: 3, action: 'archive', value: null },
        subject: 'Announced elsewhere',
        created_at_ms: Date.now(),
        expires_at_ms: Date.now() + 20000,
        status: 'pending',
        error: null,
      },
    });
    await expect(notices.getByRole('button', { name: /^Undo: .*Announced elsewhere/ })).toBeVisible();
    await announce({ kind: 'undo-removed', id: 9001 });
    await expect(notices).toHaveCount(0);
    const names = (await callsSince(mark)).filter((name) => !['prefetch_mail', 'read_mail'].includes(name));
    assert.deepEqual(names, [], 'Nothing was asked of the backend');
  });
  await step('A change too big for one read reloads the list instead', async () => {
    const mark = await quiet();
    const many = Array.from({ length: 150 }, (_, index) => 100 + index);
    await announce({ kind: 'email-changed', account_id: 1, ids: many });
    await quiet();
    const names = await callsSince(mark);
    assert.ok(count(names, 'list_mail') >= 1, `The list is reloaded: ${names}`);
    assert.equal(count(names, 'read_rows'), 0, 'No read_rows for a bulk change');
  });
  await step('Rows read for a view the user has left are dropped', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    const [first, second] = await rowIds();
    await quiet();
    await page.evaluate(() => (window.__previewCommandDelay = { read_rows: 700 }));
    await page.evaluate((id) => window.__previewEdit(id, { subject: 'Edited elsewhere' }), second);
    await announce({ kind: 'email-changed', account_id: 1, ids: [first, second] });
    await page.getByRole('button', { name: /^Archive\s*\d*$/ }).click();
    await expect(page.getByRole('heading', { name: 'Archive', exact: true }).first()).toBeVisible();
    await page.waitForTimeout(1200);
    await quiet();
    assert.equal(await rows.count(), 0, 'The Inbox rows did not slip into the Archive list');
    await page.evaluate(() => (window.__previewCommandDelay = {}));
  });
  await step('A read_rows answer that lands after "Load more" still patches its row', async () => {
    await seeded(100);
    const target = (await rowIds())[3];
    // The preview records a command in __previewCalls when it starts, but in __previewReadRows
    // only after its delay: a slow read_rows is seen starting through the first.
    await page.evaluate(() => (window.__previewCommandDelay = { read_rows: 1500 }));
    const mark = (await calls()).length;
    await edit(target, { subject: 'Edited during Load more' });
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect.poll(async () => count(await callsSince(mark), 'read_rows')).toBe(1);
    // The read is in flight: "Load more" bumps the list request and lands first.
    await loadMore.dispatchEvent('click');
    await expect(rows).toHaveCount(111);
    assert.ok(
      !(await rowOf(target).innerText()).includes('Edited during Load more'),
      'The read has not answered yet',
    );
    await expect(rowOf(target)).toContainText('Edited during Load more');
    await quiet();
    assert.equal(await listPasses(mark), 1, 'Only the Load more call, no reload');
    assert.deepEqual((await queries()).at(-1), { offset: 60, limit: 60 }, 'Load more continues at 60');
    await page.evaluate(() => (window.__previewCommandDelay = {}));
  });
  await step('A read_rows answer that lands while "Load more" is still loading patches its row too', async () => {
    await seeded(100);
    const target = (await rowIds())[3];
    await page.evaluate(() => (window.__previewCommandDelay = { read_rows: 1500 }));
    const mark = (await calls()).length;
    await edit(target, { subject: 'Edited while loading more' });
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect.poll(async () => count(await callsSince(mark), 'read_rows')).toBe(1);
    // "Load more" takes 3.5 s: the read answers while it is still loading.
    await page.evaluate(() => (window.__previewDelay = 3500));
    await loadMore.dispatchEvent('click');
    await expect(rowOf(target)).toContainText('Edited while loading more', { timeout: 3000 });
    assert.equal(await rows.count(), 60, 'Load more has not landed yet');
    await expect(rows).toHaveCount(111, { timeout: 6000 });
    await expect(rowOf(target)).toContainText('Edited while loading more');
    await quiet();
    assert.equal(await listPasses(mark), 1, 'Load more was not cancelled by a reload');
    await page.evaluate(() => {
      window.__previewCommandDelay = {};
      window.__previewDelay = 0;
    });
  });
  await step('The newest read of a row wins over an older one that answers late', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    const target = (await rowIds())[2];
    await quiet();
    await page.evaluate(() => (window.__previewCommandDelay = { read_rows: [1500, 0] }));
    const mark = (await calls()).length;
    await edit(target, { subject: 'First edit' });
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect.poll(async () => count(await callsSince(mark), 'read_rows')).toBe(1);
    await edit(target, { subject: 'Second edit' });
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect(rowOf(target)).toContainText('Second edit');
    // The first read answers later, with the first edit: it must not win.
    await expect.poll(async () => (await readRowsCalls()).length).toBe(2);
    await page.waitForTimeout(300);
    await expect(rowOf(target)).toContainText('Second edit');
    await expect(rowOf(target)).not.toContainText('First edit');
    await page.evaluate(() => (window.__previewCommandDelay = {}));
  });
  await step('A failed read_rows reloads the list once and the next event is patched again', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    const target = (await rowIds())[2];
    await quiet();
    await page.evaluate(() => (window.__previewCommandFail = { read_rows: 'backend unavailable' }));
    await edit(target, { subject: 'Edited, read failed' });
    const mark = (await calls()).length;
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect(rowOf(target)).toContainText('Edited, read failed');
    await quiet();
    let names = await callsSince(mark);
    assert.equal(count(names, 'read_rows'), 1, `One failed read: ${names}`);
    assert.ok(count(names, 'list_mail') >= 1, `The list is reloaded: ${names}`);
    await page.evaluate(() => (window.__previewCommandFail = {}));
    await edit(target, { subject: 'Edited again' });
    const next = await quiet();
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect(rowOf(target)).toContainText('Edited again');
    await quiet();
    names = await callsSince(next);
    assert.deepEqual(
      names.filter((name) => ['list_mail', 'read_rows'].includes(name)),
      ['read_rows'],
      'Back to one read, no reload',
    );
  });
  await step('Rows without the snooze flags are not patched in: the list reloads instead', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    const target = (await rowIds())[2];
    await quiet();
    // With the flags, a snoozed message leaves the Inbox through one read and no reload.
    await edit(target, { snoozed_until: Math.floor(Date.now() / 1000) + 3600 });
    let mark = (await calls()).length;
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect(rowOf(target)).toHaveCount(0);
    await quiet();
    assert.deepEqual(
      (await callsSince(mark)).filter((name) => ['list_mail', 'read_rows'].includes(name)),
      ['read_rows'],
    );
    // A backend that leaves the flags out must not bring it back: it is read as "not snoozed".
    warnings.length = 0;
    await page.evaluate(() => (window.__previewReadRowsOmit = ['snoozed_until', 'set_aside', 'reminder_at']));
    mark = (await calls()).length;
    await announce({ kind: 'email-changed', account_id: 1, ids: [target] });
    await expect.poll(async () => count(await callsSince(mark), 'list_mail')).toBeGreaterThanOrEqual(1);
    await quiet();
    assert.equal(count(await callsSince(mark), 'read_rows'), 1);
    await expect(rowOf(target)).toHaveCount(0);
    assert.ok(
      warnings.some((text) => text.includes('without snoozed_until')),
      `The broken answer is reported: ${warnings}`,
    );
    // A row still on the page, answered without the flags, is not patched either.
    const other = (await rowIds())[1];
    await edit(other, { subject: 'Edited, no flags' });
    mark = (await calls()).length;
    await announce({ kind: 'email-changed', account_id: 1, ids: [other] });
    await expect(rowOf(other)).toContainText('Edited, no flags');
    await quiet();
    assert.ok(count(await callsSince(mark), 'list_mail') >= 1, 'It came from a reload');
    await page.evaluate(() => (window.__previewReadRowsOmit = []));
  });
  await step('A payload that is not an event is reported and reloads the list', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    await quiet();
    warnings.length = 0;
    const mark = (await calls()).length;
    await announce({ kind: 'email-changed', account_id: 1, ids: 'not a list' });
    await expect.poll(async () => count(await callsSince(mark), 'list_mail')).toBeGreaterThanOrEqual(1);
    await quiet();
    const names = await callsSince(mark);
    assert.equal(count(names, 'read_rows'), 0, `Nothing was patched: ${names}`);
    assert.ok(
      warnings.some((text) => text.includes('app-event not understood')),
      `The drift is reported: ${warnings}`,
    );
  });
  await step('Rows named before a switch to conversations are not patched into the conversation list', async () => {
    await ready();
    await expect(rows.first()).toBeVisible();
    const target = (await rowIds())[2];
    await quiet();
    const mark = (await calls()).length;
    // The event names a row; before the 25 ms that batch rows are over, the display changes.
    await page.evaluate((id) => {
      window.dispatchEvent(
        new CustomEvent('app-event', { detail: { kind: 'email-changed', account_id: 1, ids: [id] } }),
      );
      document.querySelector('button[aria-label="Threads"]').click();
    }, target);
    await expect(page.getByRole('button', { name: 'Threads', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await quiet();
    assert.equal(count(await callsSince(mark), 'read_rows'), 0, 'A conversation list is read as a whole');
    await page.getByRole('button', { name: 'Emails', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Emails', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await quiet();
  });
  await step('Committing an action announces what changed and reloads nothing', async () => {
    await ready();
    await expect(heading(subject)).toBeVisible();
    await page.locator('.reader .done-button').click();
    const notices = page.getByRole('region', { name: 'Recent email actions' });
    await expect(notices.getByRole('button', { name: /^Undo:/ })).toBeVisible();
    // The reload after the click, and the opening of the next message, are over.
    await quiet();
    const mark = (await calls()).length;
    await expect(notices).toHaveCount(0, { timeout: 8000 });
    await quiet();
    const names = (await callsSince(mark)).filter((name) =>
      ['list_mail', 'read_rows', 'workspace_snapshot'].includes(name),
    );
    assert.deepEqual(
      names.sort(),
      ['read_rows', 'workspace_snapshot'],
      'The commit costs one row read and one snapshot, and no list reload',
    );
  });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: results.length, errors }));
} catch (error) {
  await page.screenshot({ path: '../checks/refresh-coalesce-failure.png' }).catch(() => {});
  throw error;
} finally {
  await browser.close();
}
