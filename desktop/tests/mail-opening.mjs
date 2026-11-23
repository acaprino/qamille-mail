// Delayed provider responses exercise the real UI with synthetic mail only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { openAllGroups } from './support/groups.mjs';

const port = 28151;
const url = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: path.resolve(import.meta.dirname, '..'), windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', data => output += data);
server.stderr.on('data', data => output += data);
let browser;
try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    localStorage.setItem('qamille:mail-display', 'threads');
    window.mailReads = [];
    window.mailPrefetches = [];
    window.mailGates = new Map();
    window.delayMail = false;
  });
  await page.route(/\/src\/lib\/preview\.ts(?:\?|$)/, async route => {
    const response = await route.fetch();
    const source = await response.text();
    assert.match(source, /export async function previewCommand\(/);
    await route.fulfill({ response, body: source.replace('export async function previewCommand(', 'async function originalPreviewCommand(') + `
      export async function previewCommand(command, args = {}) {
        if (command === 'prefetch_mail') { window.mailPrefetches.push(args.ids); return; }
        if (window.delayMail && (command === 'read_mail' || command === 'read_mail_thread')) {
          window.mailReads.push([command, args.id]);
          await new Promise((resolve, reject) => window.mailGates.set(command + ':' + args.id, { resolve, reject }));
        }
        return originalPreviewCommand(command, args);
      }
    ` });
  });
  await page.goto(url);
  await expect(page.locator('.message-body')).toBeVisible();
  const rows = page.locator('.mail-row');
  const ids = await page.evaluate(async () => (await (await import('/src/lib/api.ts')).call('list_mail', { query: { folder: 'Inbox', search: '', offset: 0, limit: 60, threads: true } })).items.map(item => item.id));
  assert.ok(ids.length >= 3);
  const idAt = async index => {
    const text = await rows.nth(index).locator('.mail-subject').innerText();
    return page.evaluate(async subject => (await (await import('/src/lib/api.ts')).call('list_mail', { query: { folder: 'Inbox', search: '', offset: 0, limit: 60, threads: true } })).items.find(item => item.subject === subject.trim()).id, text);
  };
  const first = await idAt(1);
  const second = await idAt(2);
  await expect.poll(() => page.evaluate(id => window.mailPrefetches.some(ids => ids.includes(id)), first), { timeout: 1500 }).toBe(true);
  await rows.nth(2).locator('.mail-open').hover();
  await expect.poll(() => page.evaluate(() => window.mailPrefetches.at(-1)?.[0]), { timeout: 1500 }).toBe(second);
  console.log('PASS Visible messages prefetch automatically; hovered message takes priority');
  await page.evaluate(() => { window.delayMail = true; });
  await rows.nth(1).locator('.mail-open').click();
  await expect(rows.nth(1)).toHaveClass(/selected/, { timeout: 1000 });
  const reader = page.getByRole('region', { name: 'Reading pane' });
  await expect(reader).toHaveAttribute('aria-busy', 'true');
  // A message is already open: it stays on screen, with a discreet mark instead of a blank page.
  await expect(reader.getByRole('status', { name: 'Opening message…' })).toBeVisible();
  await expect(page.locator('.message-body')).toBeVisible();
  await expect(page.locator('.reader-empty')).toHaveCount(0);
  await rows.nth(1).locator('.mail-open').click();
  assert.equal(await page.evaluate(id => window.mailReads.filter(([command, value]) => command === 'read_mail' && value === id).length, first), 1, 'Repeated clicks share the pending read');
  await page.evaluate(id => window.mailGates.get('read_mail:' + id).resolve(), first);
  await expect(page.locator('.message-body')).toBeVisible({ timeout: 1000 });
  await expect(reader).toHaveAttribute('aria-busy', 'false');
  console.log('PASS Immediate selection and feedback; message appears without waiting for thread details');

  await rows.nth(2).locator('.mail-open').click();
  await page.getByRole('navigation', { name: 'Mail folders' }).getByRole('button', { name: /^Sent\s*\d*$/ }).click();
  await page.evaluate(ids => { for (const id of ids) for (const command of ['read_mail', 'read_mail_thread']) window.mailGates.get(command + ':' + id)?.resolve(); }, [first, second]);
  await expect(page.locator('.message-body')).toHaveCount(0);
  await expect(reader).toHaveAttribute('aria-busy', 'false');
  console.log('PASS Leaving the folder cancels loading and ignores late provider responses');

  // INV-006: opening a message never blanks the reader and reads as read at once. The mock backend
  // is slowed through `window.__previewCommandDelay` (src/lib/preview.ts); a reload gives the
  // fixture back. "Done" then "Undo" within a second is covered in tests/undo.mjs.
  const subjects = {
    a: 'Q4 proposal: can you review by Thursday?',
    b: 'Design review moved to 3 pm',
    c: 'Dinner on Saturday?',
  };
  const heading = (subject) => reader.getByRole('heading', { name: subject, exact: true });
  const row = (subject) => page.locator('.mail-row', { hasText: subject });
  const inboxCount = async (on = page) => {
    const folders = on.getByRole('navigation', { name: 'Mail folders' });
    const badge = folders.getByRole('button', { name: /^Inbox\s*\d*$/ }).locator('.nav-count');
    return (await badge.count()) ? Number(await badge.innerText()) : 0;
  };
  const calls = (name) =>
    page.evaluate((command) => window.__previewCalls.filter((call) => call === command).length, name);
  const isUnread = async (subject) => (await row(subject).getAttribute('class')).includes('unread');
  const listPasses = () =>
    page.evaluate(() => window.__previewListQueries.filter((query) => query.offset === 0).length);
  // The first message opens by itself and is marked read; its follow-up reload must be over
  // before a step slows the backend down.
  const settle = async (on = page) => {
    let last = -1;
    for (;;) {
      const now = await on.evaluate(() => window.__previewCalls.length);
      if (now === last) return;
      last = now;
      await on.waitForTimeout(250);
    }
  };
  // The reload that follows the backend's answer to the last mail_action has started.
  const reloadedAfterMark = (on = page) =>
    on.evaluate(() => {
      const sent = window.__previewCalls;
      return sent.slice(sent.lastIndexOf('mail_action')).includes('workspace_snapshot');
    });
  const ready = async () => {
    await page.goto(url);
    await expect(heading(subjects.a)).toBeVisible();
    await expect(row(subjects.a)).not.toHaveClass(/unread/);
    await expect(row(subjects.b)).toHaveClass(/unread/);
    // The quiet groups start closed; this test opens messages that live in them.
    await openAllGroups(page);
    await settle();
  };

  await ready();
  const before = await inboxCount();
  assert.ok(before >= 3, 'The fixture has several unread messages');
  const marks = await calls('mail_action');
  await page.evaluate(() => (window.__previewCommandDelay = { read_mail: 1000, mail_action: 1000 }));
  await row(subjects.b).locator('.mail-open').click();
  // Neither the message nor its read mark has come back from the backend yet.
  await expect(row(subjects.b)).not.toHaveClass(/unread/, { timeout: 100 });
  await expect.poll(inboxCount, { timeout: 100 }).toBe(before - 1);
  // A reload that reads the backend before it has the mark (a sync, another action) must not undo it.
  const passes = await listPasses();
  await page.evaluate(() => window.dispatchEvent(new Event('preview-workspace-changed')));
  await expect.poll(listPasses).toBeGreaterThan(passes);
  await page.waitForTimeout(150);
  // Read at this very moment: expect() would wait for the row to turn read once the backend answers.
  assert.equal(await isUnread(subjects.b), false, 'A reload during the wait keeps the row read');
  assert.equal(await inboxCount(), before - 1, 'A reload during the wait keeps the counter');
  await expect(heading(subjects.b)).toBeVisible({ timeout: 2000 });
  await expect.poll(() => calls('mail_action')).toBe(marks + 1);
  // The backend answers a second later, and the reload that follows is the one that must not
  // flip anything back.
  await expect.poll(() => reloadedAfterMark()).toBe(true);
  await settle();
  assert.equal(await isUnread(subjects.b), false, 'The backend confirming does not flip the row back');
  assert.equal(await inboxCount(), before - 1, 'The backend confirming does not change the counter');
  // Opening it again changes nothing: not read twice, not counted twice.
  await page.evaluate(() => (window.__previewCommandDelay = {}));
  await row(subjects.c).locator('.mail-open').click();
  await expect(heading(subjects.c)).toBeVisible();
  await row(subjects.b).locator('.mail-open').click();
  await expect(heading(subjects.b)).toBeVisible();
  await settle();
  assert.equal(await calls('mail_action'), marks + 2, 'Only the two unread messages were marked');
  assert.equal(await inboxCount(), before - 2);
  console.log('PASS An unread message reads as read at once and the counter follows, once');

  await ready();
  await page.evaluate(() => {
    window.__fullPage = false;
    const watch = () => {
      const text = document.querySelector('.reader')?.textContent ?? '';
      if (document.querySelector('.reader-empty') || text.includes('Opening message')) window.__fullPage = true;
    };
    new MutationObserver(watch).observe(document.body, { childList: true, subtree: true, characterData: true });
    window.__previewCommandDelay = { read_mail: 1000 };
  });
  const clicked = Date.now();
  await row(subjects.b).locator('.mail-open').click();
  await expect(reader.getByRole('status', { name: 'Opening message…' })).toBeVisible({ timeout: 300 });
  await expect(heading(subjects.a)).toBeVisible();
  await expect(page.locator('.message-body')).toBeVisible();
  await expect(page.locator('.reader-empty')).toHaveCount(0);
  await expect(heading(subjects.b)).toBeVisible({ timeout: 2000 });
  await expect(heading(subjects.a)).toHaveCount(0);
  await expect(reader.getByRole('status', { name: 'Opening message…' })).toHaveCount(0);
  await page.waitForTimeout(Math.max(0, 1200 - (Date.now() - clicked)));
  assert.equal(await page.evaluate(() => window.__fullPage), false, 'The full-page placeholder never showed');
  console.log('PASS The open message stays until the next one arrives; no full-page placeholder');

  await ready();
  const intact = await inboxCount();
  await page.evaluate(() => {
    window.__previewCommandDelay = { mail_action: 800 };
    window.__previewCommandFail = { mail_action: 'Synthetic markAsSeen failure' };
  });
  await row(subjects.b).locator('.mail-open').click();
  await expect(row(subjects.b)).not.toHaveClass(/unread/, { timeout: 100 });
  await expect.poll(inboxCount, { timeout: 100 }).toBe(intact - 1);
  await expect(page.locator('[role="alert"], [data-scope="toast"]')).toContainText('Synthetic markAsSeen failure');
  await expect(row(subjects.b)).toHaveClass(/unread/);
  await expect.poll(inboxCount).toBe(intact);
  await expect(heading(subjects.b)).toBeVisible();
  await page.evaluate(() => {
    window.__previewCommandDelay = {};
    window.__previewCommandFail = {};
  });
  console.log('PASS A read mark the backend refuses is taken back, counter included, and reported');

  await ready();
  const untouched = await inboxCount();
  const sent = await calls('mail_action');
  // The first answer (the older click) is the slow one.
  await page.evaluate(() => (window.__previewCommandDelay = { read_mail: [1500, 0] }));
  await row(subjects.b).locator('.mail-open').click();
  await row(subjects.c).locator('.mail-open').click();
  await expect(heading(subjects.c)).toBeVisible({ timeout: 1000 });
  await expect(row(subjects.c)).toHaveClass(/selected/);
  await page.waitForTimeout(1800);
  await expect(heading(subjects.c)).toBeVisible();
  await expect(heading(subjects.b)).toHaveCount(0);
  await expect(row(subjects.b)).not.toHaveClass(/selected/);
  // The older message was never read: it is unread again, and only the newer one was marked.
  await expect(row(subjects.b)).toHaveClass(/unread/);
  await expect(row(subjects.c)).not.toHaveClass(/unread/);
  await expect.poll(inboxCount).toBe(untouched - 1);
  await settle();
  assert.equal((await calls('mail_action')) - sent, 1);
  console.log('PASS A slower answer for an older click cannot take the reader back');

  // An older click that gives up must take back only what it marked itself.
  await ready();
  const statement = 'Your monthly statement is ready';
  const typography = 'Typography trends for 2027';
  await expect(row(statement)).not.toHaveClass(/unread/);
  const kept = await inboxCount();
  const noMarks = await calls('mail_action');
  await page.evaluate(() => (window.__previewCommandDelay = { read_mail: [1500, 0] }));
  await row(statement).locator('.mail-open').click();
  await row(typography).locator('.mail-open').click();
  await expect(heading(typography)).toBeVisible({ timeout: 1000 });
  await page.waitForTimeout(1800);
  assert.equal(await isUnread(statement), false, 'A message that was read stays read');
  assert.equal(await inboxCount(), kept);
  assert.equal(await calls('mail_action'), noMarks, 'Nothing was marked');
  console.log('PASS Giving up on a message that was already read leaves it read');

  // A, then B, then A again: the first answer for A comes late, while the mark of the second
  // click for A still waits for the backend. It must not take that mark back.
  await ready();
  const twice = await inboxCount();
  await page.evaluate(() => (window.__previewCommandDelay = { read_mail: [1500, 0, 0], mail_action: 3000 }));
  await row(subjects.b).locator('.mail-open').click();
  await row(subjects.c).locator('.mail-open').click();
  await row(subjects.b).locator('.mail-open').click();
  await expect(heading(subjects.b)).toBeVisible({ timeout: 1000 });
  await page.waitForTimeout(1700);
  assert.equal(await isUnread(subjects.b), false, 'The late first answer does not make it unread again');
  assert.equal(await isUnread(subjects.c), false);
  assert.equal(await inboxCount(), twice - 2);
  console.log('PASS A late answer for an older click does not take back a newer mark of the same message');

  // A search that drops the message on screen while another one is still being read supersedes
  // that read: the reader must not stay on the full-page placeholder.
  await ready();
  await page.evaluate(() => (window.__previewCommandDelay = { read_mail: [4000] }));
  await row(subjects.b).locator('.mail-open').click();
  await expect(reader.getByRole('status', { name: 'Opening message…' })).toBeVisible({ timeout: 1000 });
  await page.getByRole('textbox', { name: 'Search mail' }).fill('Dinner');
  await expect(heading(subjects.a)).toHaveCount(0, { timeout: 1500 });
  await expect(page.locator('.reader-empty', { hasText: 'Opening message' })).toHaveCount(0, { timeout: 1500 });
  await expect(reader).toHaveAttribute('aria-busy', 'false');
  console.log('PASS A search that drops the open message ends the pending read; no stuck placeholder');

  // Scanning down faster than read_mail answers: rows passed over must not look read, and the
  // counter must not dip, once the next open has begun.
  await ready();
  const scanOrder = await page.locator('.mail-row').evaluateAll((nodes) =>
    nodes.map((node) => ({
      subject: node.querySelector('.mail-subject')?.textContent?.trim(),
      unread: node.classList.contains('unread'),
      selected: node.classList.contains('selected'),
    })),
  );
  const scanFrom = scanOrder.findIndex((item) => item.selected);
  const scanned = scanOrder.slice(scanFrom + 1, scanFrom + 4);
  assert.equal(scanned.length, 3);
  assert.ok(scanned.every((item) => item.unread), `Three unread rows follow the open one: ${JSON.stringify(scanned)}`);
  const scanBefore = await inboxCount();
  await page.evaluate(() => (window.__previewCommandDelay = { read_mail: [1500, 1500, 0] }));
  await page.locator('body').click({ position: { x: 700, y: 45 } });
  const samples = [];
  const sample = async () =>
    samples.push({
      counter: await inboxCount(),
      unread: await page.locator('.mail-row.unread').evaluateAll((nodes) =>
        nodes.map((node) => node.querySelector('.mail-subject')?.textContent?.trim()),
      ),
    });
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press('ArrowDown');
    await page.waitForTimeout(40);
  }
  const scanStart = Date.now();
  while (Date.now() - scanStart < 2200) {
    await sample();
    await page.waitForTimeout(100);
  }
  const [passedA, passedB, opened] = scanned.map((item) => item.subject);
  await expect(heading(opened)).toBeVisible();
  const worst = Math.min(...samples.map((item) => item.counter));
  assert.ok(worst >= scanBefore - 1, `The counter dipped to ${worst} from ${scanBefore} while scanning`);
  for (const [index, item] of samples.entries())
    assert.ok(
      item.unread.includes(passedA) && item.unread.includes(passedB),
      `Rows passed over looked read at sample ${index}: ${JSON.stringify(item)}`,
    );
  assert.equal((await inboxCount()), scanBefore - 1, 'Only the opened message counts as read');
  assert.equal(await isUnread(opened), false);
  assert.equal(await isUnread(passedA), true);
  assert.equal(await isUnread(passedB), true);
  console.log('PASS Rows passed over while scanning stay unread and the counter dips at most once');

  // A conversation row cannot say which of its messages is unread, so the message is marked when
  // it arrives; the row reads as read once all its messages do. A context of its own, with a
  // conversation of two messages: the newer one is read, the older one is not.
  const fixture = JSON.parse(await fs.readFile(new URL('../src/lib/sample.json', import.meta.url), 'utf8'));
  const root = fixture.emails.find((email) => email.id === 4);
  fixture.emails.push({
    ...structuredClone(root),
    id: 99001,
    message_id: '<mail-opening-reply@test>',
    thread_id: root.message_id,
    date: root.date + 60,
    read: true,
    pinned: false,
    body: 'A reply in the same conversation.',
  });
  fixture.next_id = 100000;
  const threadContext = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 1000 } });
  const tp = await threadContext.newPage();
  tp.on('pageerror', (error) => errors.push(error.message));
  await tp.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
  await tp.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await tp.goto(url);
  // The conversation row says how many messages it holds on its arrow; opened, the other messages
  // are rows under it (ADR-0010), where an older unread one can be read.
  const tThread = tp.locator('.mail-row', { has: tp.locator('.thread-toggle', { hasText: '2' }) });
  const tUnreadMessage = tp.locator('.reply-row.unread');
  await expect(tThread).toHaveCount(1);
  await tThread.locator('.thread-toggle').click();
  await tThread.locator('.mail-open').click();
  await expect(tThread).toHaveClass(/selected/);
  await expect(tp.locator('.reply-row')).toHaveCount(1);
  // Opening the row shows the newer, read message: the older one keeps the conversation unread.
  await expect(tThread).toHaveClass(/unread/);
  await expect(tUnreadMessage).toHaveCount(1);
  await settle(tp);
  const unreadBefore = await inboxCount(tp);
  const sentBefore = await tp.evaluate(() => window.__previewCalls.filter((call) => call === 'mail_action').length);
  await tp.evaluate(() => (window.__previewCommandDelay = { mail_action: 1000 }));
  await tUnreadMessage.locator('.reply-open').click();
  await expect(tThread).not.toHaveClass(/unread/, { timeout: 100 });
  await expect.poll(() => inboxCount(tp), { timeout: 100 }).toBe(unreadBefore - 1);
  await expect(tp.locator('.reply-row.unread')).toHaveCount(0, { timeout: 100 });
  // The conversation stays on screen, and the list that comes with it does not undo the mark.
  await tp.waitForTimeout(300);
  await expect(tp.locator('.reply-row')).toHaveCount(1);
  await expect(tp.locator('.reply-row.unread')).toHaveCount(0);
  await expect.poll(() => reloadedAfterMark(tp)).toBe(true);
  await settle(tp);
  await expect(tThread).not.toHaveClass(/unread/);
  assert.equal(await inboxCount(tp), unreadBefore - 1);
  assert.equal(
    (await tp.evaluate(() => window.__previewCalls.filter((call) => call === 'mail_action').length)) - sentBefore,
    1,
  );
  console.log('PASS A message of a conversation is marked read once it arrives; the row follows');

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
