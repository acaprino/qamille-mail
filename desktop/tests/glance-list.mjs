// The inbox list at a glance (spec 2026-11-07 §4.2): minor groups closed with a trace of what
// they hold, two weights, an amber priority row, hover-only checkboxes, a select bar that waits
// for the first tick, and a headline that says what needs the reader. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';

const port = 28172;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));
// A second unread Priority message, so one Priority row stays unselected when the app opens the first.
const fixture = structuredClone(sample);
const firstPriority = fixture.emails.find((e) => e.folder === 'Inbox' && !e.read && e.category === 'Priority');
fixture.emails.push({
  ...firstPriority, id: 98001, subject: 'Second priority message', message_id: '<second-priority@test>',
  thread_id: 't98001', date: firstPriority.date - 3600, body: 'Another message that needs an answer today.',
});

// The oracle is the fixture, not the app. The Inbox is mail in the Inbox that is neither done nor
// snoozed; a row is Priority when it is unread, not pinned and its category is Priority. The app
// opens the first message when it starts, and an opened message is no longer unread.
const inboxOf = (data) => data.emails.filter((e) => e.folder === 'Inbox' && !e.done);
assert.ok(inboxOf(fixture).every((e) => !e.snoozed_until), 'the fixture has no snoozed Inbox mail');
const needs = (data, openedSubject) => {
  const unread = inboxOf(data).filter((e) => !e.read && e.subject !== openedSubject);
  return { unread: unread.length, priority: unread.filter((e) => !e.pinned && e.category === 'Priority').length };
};
const sentence = ({ unread, priority }) =>
  unread === 0 ? 'All read' : priority > 0 ? `${priority} priority · ${unread} to read` : `${unread} to read`;
assert.ok(inboxOf(fixture).some((e) => !e.read && e.category === 'Notification'), 'the fixture needs an unread notification');
assert.ok(needs(fixture, '').priority >= 2, 'the fixture needs two unread priority messages');

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];

const open = async (data = fixture) => {
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
  page.on('pageerror', (error) => errors.push(error.message));
  {
    await page.route(/\/src\/lib\/sample\.json/, (route) =>
      route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(data)};` }),
    );
  }
  await page.goto(url);
  await expect(page.locator('.mail-row').first()).toBeVisible();
  // The first message opens by itself; its subject is what the oracle has to leave out.
  await expect(page.locator('.message-title')).toBeVisible();
  page.opened = await page.locator('.message-title').innerText();
  return page;
};
// The colour a token resolves to, as the browser writes it in computed styles.
const token = (page, name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, name);
const heading = (page, name) => page.locator('.group-heading', { hasText: name });
const headline = async (page) => (await page.locator('.list-heading .muted').innerText()).replace(/\s+/g, ' ').trim();

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await open();

  // Minor groups start closed, and a closed group never hides that it holds something unread.
  for (const name of ['Notifications', 'Newsletters', 'Seen']) {
    await expect(heading(page, name)).toHaveAttribute('aria-expanded', 'false');
  }
  await expect(heading(page, 'Priority')).toHaveAttribute('aria-expanded', 'true');
  await expect(heading(page, 'Notifications')).toHaveClass(/has-unread/);
  await expect(heading(page, 'Seen')).not.toHaveClass(/has-unread/);
  const blue = await token(page, '--color-accent');
  assert.equal(await heading(page, 'Notifications').locator('.group-count').evaluate((el) => getComputedStyle(el).color), blue, 'the number of a closed group with unread mail is blue');
  assert.notEqual(await heading(page, 'Seen').locator('.group-count').evaluate((el) => getComputedStyle(el).color), blue);
  console.log('PASS Minor groups start closed and keep a trace');

  // What the reader opened stays open when they leave the mail and come back.
  await heading(page, 'Notifications').click();
  await expect(heading(page, 'Notifications')).toHaveAttribute('aria-expanded', 'true');
  await page.locator('.sidebar nav button', { hasText: 'Calendar' }).click();
  await expect(page.locator('.mail-list')).toHaveCount(0);
  await page.locator('.sidebar nav button', { hasText: 'Inbox' }).click();
  await expect(heading(page, 'Notifications')).toHaveAttribute('aria-expanded', 'true');
  await heading(page, 'Notifications').click();
  await expect(heading(page, 'Notifications')).toHaveAttribute('aria-expanded', 'false');
  console.log('PASS A group opened by the reader stays open after leaving and coming back');

  // The headline says what needs the reader, not how much mail there is.
  await expect.poll(() => headline(page)).toBe(sentence(needs(fixture, page.opened)));
  const total = await page.evaluate(async () => (await (await import('/src/lib/api.ts')).call('list_mail', { query: { folder: 'Sent', search: '', offset: 0, limit: 60, threads: false } })).total);
  await page.locator('.sidebar nav button', { hasText: /^Sent/ }).click();
  await expect(page.locator('.list-heading h1')).toHaveText('Sent');
  assert.equal(await headline(page), `${total} ${total === 1 ? 'message' : 'messages'}`, 'other folders keep their total');
  await page.locator('.sidebar nav button', { hasText: 'Inbox' }).click();
  await expect(page.locator('.mail-row').first()).toBeVisible();
  console.log('PASS The Inbox headline counts what needs the reader');

  // Two weights: what needs you is ink and bold, the rest steps back.
  const weight = (locator) => locator.evaluate((el) => Number(getComputedStyle(el).fontWeight));
  const unreadRow = page.locator('.mail-row.unread').first();
  // The message just opened keeps its Priority card until the reader moves on, so a plain read
  // row is the pinned one.
  const readRow = page.locator('.mail-row:not(.unread):not([data-section="priority"])').first();
  assert.ok((await weight(unreadRow.locator('.mail-meta strong'))) >= 700, 'an unread sender is bold');
  assert.equal(await weight(readRow.locator('.mail-meta strong')), 500, 'a read sender is regular');
  const ink = await token(page, '--color-ink');
  assert.equal(await unreadRow.locator('.mail-meta strong').evaluate((el) => getComputedStyle(el).color), ink);
  // Compatto (mockup): the sender is ink in both states; the weight, the dot and the accent time tell them apart.
  assert.equal(await readRow.locator('.mail-meta strong').evaluate((el) => getComputedStyle(el).color), ink, 'a read sender is ink too');
  assert.equal(await readRow.locator('.unread-dot').count(), 0, 'a read row has no dot');
  const row = page.locator('.mail-row[data-section="priority"]:not(.selected)').first();
  await page.mouse.move(0, 0);
  assert.equal(await row.evaluate((el) => getComputedStyle(el).backgroundColor), await token(page, '--priority-wash'), 'the priority row has the amber wash');
  assert.ok((await row.evaluate((el) => getComputedStyle(el).boxShadow)).includes(await token(page, '--priority-mark')), 'and the amber edge');
  console.log('PASS Two weights and an amber priority row');

  // Rows already read do not repeat their text.
  await heading(page, 'Seen').click();
  await expect(page.locator('.mail-row[data-section="seen"]').first()).toBeVisible();
  assert.equal(await page.locator('.mail-row[data-section="seen"] .mail-preview').first().evaluate((el) => getComputedStyle(el).display), 'none');
  await heading(page, 'Seen').click();
  console.log('PASS Already-read rows show no preview line');

  // Checkboxes take the gutter of the unread dot on hover, and the select bar waits for the first tick.
  const first = page.locator('.mail-row').first();
  await page.mouse.move(0, 0);
  assert.equal(await first.locator('.mail-check').evaluate((el) => getComputedStyle(el).opacity), '0', 'no checkbox at rest');
  await expect(page.locator('.selection-toolbar')).toHaveCount(0);
  await first.locator('.mail-open').hover();
  assert.equal(await first.locator('.mail-check').evaluate((el) => getComputedStyle(el).opacity), '1', 'the checkbox shows under the pointer');
  assert.equal(await first.locator('.sender-avatar').count(), 0, 'rows have no avatar');
  const unreadFirst = page.locator('.mail-row.unread').first();
  await unreadFirst.locator('.mail-open').hover();
  assert.equal(await unreadFirst.locator('.unread-dot').evaluate((el) => getComputedStyle(el).visibility), 'hidden', 'the checkbox takes the place of the dot');
  await first.locator('.mail-open').hover();
  await first.locator('.mail-checkbox').click();
  const bar = page.locator('.selection-toolbar');
  await expect(bar).toBeVisible();
  await expect(bar).toContainText('1 selected');
  await expect(bar.getByRole('button', { name: 'Print selected' })).toBeVisible();
  await expect(bar.getByRole('button', { name: 'Download .mbox' })).toBeVisible();
  await page.mouse.move(0, 0);
  assert.equal(await first.locator('.mail-check').evaluate((el) => getComputedStyle(el).opacity), '1', 'a ticked row keeps its checkbox');
  await bar.getByRole('button', { name: 'Clear' }).click();
  await expect(page.locator('.selection-toolbar')).toHaveCount(0);
  console.log('PASS Checkboxes appear on hover and the select bar waits for the first tick');
  await page.close();

  // With nothing priority left the headline drops that part; with nothing unread it says so.
  const priorityRead = structuredClone(fixture);
  for (const e of priorityRead.emails) if (e.category === 'Priority') e.read = true;
  const quiet = await open(priorityRead);
  const quietNeeds = needs(priorityRead, quiet.opened);
  assert.equal(quietNeeds.priority, 0);
  await expect.poll(() => headline(quiet)).toBe(sentence(quietNeeds));
  assert.ok(!(await headline(quiet)).includes('priority'), 'no empty priority count');
  await quiet.close();
  const allRead = structuredClone(fixture);
  for (const e of allRead.emails) e.read = true;
  const done = await open(allRead);
  await expect.poll(() => headline(done)).toBe('All read');
  await done.close();
  console.log('PASS The headline drops an empty priority count and says when all is read');

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
