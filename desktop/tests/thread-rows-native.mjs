// The restyle and the conversation rows (spec 2026-11-07 §4, ADR-0010 v2) on the BUILT executable,
// not on the browser preview: its fonts under the packaged CSP, the frame, the Inbox headline, the
// Conversations mode and the reply rows served by the real Rust backend.
//
// Needs release/Qamille.exe built with `--features custom-protocol` (AGENTS.md), or
// QAMILLE_NATIVE_EXE. Sample accounts only, on a disposable folder under checks/; the app's
// outgoing HTTP is pointed at a dead proxy (see support/native-app.mjs). Run from desktop/:
//   node tests/thread-rows-native.mjs
import { expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { IDS, SUBJECT, withConversation } from './support/conversation-fixture.mjs';
import { colorOf, launchNative, root } from './support/native-app.mjs';

const sample = JSON.parse(await fs.readFile(path.join(root, 'desktop/src/lib/sample.json'), 'utf8'));
const fixture = withConversation(sample);
// A second unread Priority message, so the headline keeps a priority part after the app opens the first one.
const firstPriority = fixture.emails.find((e) => e.folder === 'Inbox' && !e.read && e.category === 'Priority');
fixture.emails.push({
  ...firstPriority, id: 98001, subject: 'Second priority message', message_id: '<second-priority@test>',
  thread_id: 't98001', date: firstPriority.date - 3600, body: 'Another message that needs an answer today.',
});
// The sample's invoice carries a reminder that is already due: the app brings it back pinned and unread
// a moment after it starts. Reminders are not what this test is about, so none is due.
for (const email of fixture.emails) email.reminder_at = null;
const app = await launchNative('thread-rows-native', { store: fixture });
const { page } = app;
const text = async (locator) => (await locator.innerText()).replace(/\s+/g, ' ').trim();
const row = page.locator('.mail-row', { hasText: SUBJECT });
const toggle = row.locator('.thread-toggle');
const reply = (id) => page.locator(`.reply-row[data-reply-id="${id}"]`);
const replies = page.locator('.reply-row');
const cdp = await page.context().newCDPSession(page);
const resize = async (width, height) => {
  await cdp.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
  await page.waitForTimeout(250);
};
try {
  await expect(page.locator('.mail-row').first()).toBeVisible();
  // The app opens the first row at start, but in the executable that open can be dropped by a list refresh
  // that the start-up events trigger; the reader then stays empty. So the test opens the first row itself.
  await page.locator('.mail-row').first().locator('.mail-open').click();
  await expect(page.locator('.message-title')).toBeVisible();
  await page.evaluate(() => document.fonts.ready);

  // ---- 1. The page is the packaged one, its fonts load under the production CSP ----
  assert.equal(page.url(), 'http://tauri.localhost/', 'the packaged UI, not the dev server');
  const faces = await page.evaluate(() => [...document.fonts].filter((f) => f.status === 'loaded').map((f) => f.family.replace(/"/g, '')));
  assert.ok(faces.includes('IBM Plex Sans'), `the face is loaded by the app (${faces})`);
  const fonts = await app.fontEntries();
  assert.ok(fonts.length >= 1 && fonts.every((r) => r.status === 200 && r.url.startsWith('http://tauri.localhost/fonts/')),
    `fonts come from the app itself: ${JSON.stringify(fonts)}`);
  console.log('PASS IBM Plex Sans is loaded, served by the app itself (the console check follows at the end, on a fresh load)');

  // ---- 2. The frame, at the real window size and at the 1000 px minimum ----
  for (const [width, height] of [[1440, 900], [1000, 640]]) {
    await resize(width, height);
    const frame = await page.evaluate(() => {
      const style = (selector) => getComputedStyle(document.querySelector(selector));
      const sheet = document.querySelector('.workspace-content').getBoundingClientRect();
      return {
        bodyFont: style('body').fontFamily, titleFont: style('.list-heading h1').fontFamily,
        radius: style('.workspace-content').borderTopLeftRadius, border: style('.workspace-content').borderTopWidth,
        right: window.innerWidth - sheet.right, bottom: window.innerHeight - sheet.bottom,
        ground: style('.app-shell').backgroundColor, sidebarEdge: style('.sidebar').borderRightWidth,
        sheetShadow: style('.workspace-content').boxShadow,
        scrollWidth: document.documentElement.scrollWidth, innerWidth: window.innerWidth,
        done: document.querySelector('.done-button').getBoundingClientRect().right,
        reader: document.querySelector('.reader').getBoundingClientRect().right,
      };
    });
    const at = `${width}x${height}`;
    assert.match(frame.bodyFont, /^"?IBM Plex Sans"?/, `${at}: body font ${frame.bodyFont}`);
    assert.match(frame.titleFont, /^"?IBM Plex Sans"?/, `${at}: title font ${frame.titleFont}`);
    assert.equal(frame.radius, '0px');
    assert.equal(frame.border, '1px');
    assert.equal(Math.round(frame.right), 0, `${at}: the sheet reaches the right edge`);
    assert.equal(Math.round(frame.bottom), 0, `${at}: the sheet reaches the bottom edge`);
    assert.equal(frame.ground, 'rgb(246, 247, 249)', `${at}: the ground`);
    assert.equal(frame.sidebarEdge, '0px');
    assert.equal(frame.sheetShadow, 'none', `${at}: no resting shadow`);
    assert.ok(frame.scrollWidth <= frame.innerWidth, `${at}: no sideways scroll`);
    assert.ok(frame.done <= frame.reader, `${at}: Done stays inside the reading pane`);
    console.log(`PASS The frame and the reading pane in the real window at ${at}`);
  }
  await resize(1440, 900);

  // ---- 3. The Inbox headline (Email mode, which is the default) ----
  // The expectation is computed from the messages as the backend holds them, one by one, because which messages
  // are read depends on what the app opened by itself:
  // unread = messages not read; priority = unread, not pinned, category Priority (the Priority group).
  const headline = async () => text(page.locator('.list-heading .muted'));
  const everyMessage = async () => (await app.invoke('list_mail', { query: { folder: 'Inbox', threads: false, limit: 100, offset: 0 } })).items;
  const expected = async () => {
    const all = await everyMessage();
    const unread = all.filter((m) => !m.read);
    return { unread: unread.length, priority: unread.filter((m) => !m.pinned && m.category === 'priority').length };
  };
  const sentence = ({ unread, priority }) => (unread === 0 ? 'All read' : priority > 0 ? `${priority} priority · ${unread} to read` : `${unread} to read`);
  const start = await expected();
  assert.ok(start.unread >= 5 && start.unread <= 7, `a sample inbox with a few unread messages (${start.unread})`);
  await expect.poll(headline, { timeout: 15000 }).toBe(sentence(await expected()));
  assert.ok(!/messages?$/.test(await headline()), 'no total of messages in the headline');
  console.log(`PASS The Inbox headline in Email mode: "${await headline()}", from the unread and priority messages, not a total`);

  // ---- 4. Quiet groups start closed and keep a trace of their unread mail ----
  for (const name of ['Notifications', 'Newsletters', 'Seen']) {
    await expect(page.locator('.group-heading', { hasText: name })).toHaveAttribute('aria-expanded', 'false');
  }
  const notifications = page.locator('.group-heading', { hasText: 'Notifications' });
  assert.equal(await notifications.locator('.group-count').evaluate((el) => getComputedStyle(el).color), await colorOf(page, '--accent-text'),
    'a closed group with unread mail counts in blue');
  console.log('PASS Notifications, Newsletters and Seen start closed; the unread one is marked blue');

  // ---- 5. Conversations mode ----
  const modeButton = page.getByRole('button', { name: 'Threads', exact: true });
  await expect(modeButton).toHaveAttribute('aria-pressed', 'false');
  await modeButton.click();
  await expect(modeButton).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.mail-list')).toHaveAttribute('aria-label', 'Thread list');
  await expect(row).toHaveCount(1);
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  assert.equal((await text(toggle)).replace(/\D/g, ''), '5', 'the arrow carries the total: the main row plus four');
  await expect(replies).toHaveCount(0);
  const now = await expected();
  const priorityRows = await page.locator('.mail-row.unread[data-section="priority"]').count();
  await expect.poll(headline).toBe(sentence({ unread: now.unread, priority: priorityRows }));
  assert.equal(priorityRows, now.priority, 'in Conversations mode too, priority is the unread rows of the Priority group');
  assert.equal(await text(page.locator('nav[aria-label="Mail folders"] button', { hasText: 'Inbox' }).locator('.nav-count')), String(now.unread),
    'the headline and the sidebar badge agree');
  console.log('PASS Conversations mode: one row for the conversation, the total on its arrow, the same headline');

  // ---- 6. The reply rows, from the real backend ----
  const items = await app.invoke('list_mail', { query: { folder: 'Inbox', threads: true, limit: 60, offset: 0 } });
  const main = items.items.find((i) => i.subject === SUBJECT);
  assert.equal(main.id, IDS.main, 'the newest message of the view is the main row');
  assert.deepEqual(main.replies.map((r) => r.id), [IDS.root, IDS.a, IDS.sent, IDS.many], 'tree order, the sent reply among them');
  assert.ok(!main.message_ids.includes(IDS.sent), 'a message that is only shown is not in message_ids (INV-001)');
  await toggle.click();
  await expect(replies).toHaveCount(4);
  assert.deepEqual(await replies.evaluateAll((nodes) => nodes.map((n) => Number(n.dataset.replyId))), [IDS.root, IDS.a, IDS.sent, IDS.many]);
  // Compatto has no avatars (compatto-list.mjs): the sender's name is the edge, the same as the row's.
  assert.equal(await replies.locator('.reply-avatar').count(), 0, 'no avatar on a reply');
  const lefts = await replies.locator('.reply-head strong').evaluateAll((nodes) => nodes.map((n) => Math.round(n.getBoundingClientRect().left)));
  const mainLeft = Math.round((await row.locator('.mail-meta strong').boundingBox()).x);
  assert.equal(lefts.length, 4, 'every reply names its sender');
  assert.deepEqual([...new Set(lefts)], [mainLeft], `every reply sits on the same edge as the row (${lefts} vs ${mainLeft})`);
  assert.equal(await replies.locator('.mail-checkbox, .row-pin').count(), 0, 'no checkbox or pin on a reply');
  const to = (id) => text(reply(id).locator('.reply-to'));
  assert.equal(await to(IDS.root), 'to you');
  await expect.poll(() => to(IDS.a)).toBe('to Maria Rossi +1');
  assert.equal(await to(IDS.sent), 'to Luca Bianchi +1');
  assert.equal(await to(IDS.many), 'to Luca Bianchi +11', 'twelve recipients: the one answered and the others counted');
  for (const [width, height] of [[1440, 900], [1000, 640]]) {
    await resize(width, height);
    const boxes = await replies.evaluateAll((nodes) => nodes.map((n) => {
      const list = n.closest('.thread-replies').getBoundingClientRect();
      const t = n.querySelector('time').getBoundingClientRect();
      const more = n.querySelector('.to-more')?.getBoundingClientRect();
      return { height: Math.round(n.getBoundingClientRect().height * 10) / 10, timeRight: t.right, listRight: list.right, moreRight: more?.right ?? 0, moreWidth: more?.width ?? 0 };
    }));
    assert.equal(new Set(boxes.map((b) => b.height)).size, 1, `${width}px: equal heights (${boxes.map((b) => b.height)})`);
    assert.ok(boxes.every((b) => b.timeRight <= b.listRight + 0.5 && b.moreRight <= b.listRight + 0.5), `${width}px: date and +N inside the list`);
    assert.ok(boxes[3].moreWidth > 0, `${width}px: "+11" is still there`);
  }
  await resize(1440, 900);
  console.log('PASS The reply rows: order, one edge, who each answers, equal heights at 1440 and 1000');

  // ---- 7. Fold a reply ----
  const fold = reply(IDS.a).locator('.reply-fold');
  assert.equal(await reply(IDS.many).locator('.reply-fold').count(), 0);
  await fold.click();
  await expect(replies).toHaveCount(2);
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  assert.equal((await text(fold)).replace(/\D/g, ''), '2', 'it says how many it hides');
  assert.equal(await fold.evaluate((el) => getComputedStyle(el).color), await colorOf(page, '--accent-text'), 'blue: one of them is unread');
  await fold.click();
  await expect(replies).toHaveCount(4);
  console.log('PASS Folding a reply hides what is under it, with a count that is blue for an unread one');

  // ---- 8. Opening a reply reads it, in the backend too ----
  assert.equal((await app.invoke('read_mail', { id: IDS.many })).read, false, 'unread before');
  await reply(IDS.many).locator('.reply-open').click();
  await expect(page.locator('.message-body').first()).toContainText('Adding the rest of the team');
  await expect(reply(IDS.many)).toHaveClass(/selected/);
  await expect(reply(IDS.many)).not.toHaveClass(/unread/);
  await expect.poll(async () => (await app.invoke('read_mail', { id: IDS.many })).read).toBe(true);
  assert.equal(await replies.locator('.unread-dot').count(), 0);
  await expect(page.getByRole('region', { name: 'Conversation messages' })).toHaveCount(0);
  console.log('PASS Opening a reply marks it read in the Rust store; the reader no longer repeats the conversation');

  // ---- 9. A sent reply can be read and answered, not managed from here (INV-002) ----
  await reply(IDS.sent).locator('.reply-open').click();
  await expect(page.locator('.message-body').first()).toContainText('Budget is with Maria');
  const toolbar = page.locator('.reader-toolbar');
  for (const name of ['Archive message', 'Snooze until tomorrow', 'Mark message unread', 'Move message to trash', 'Done']) {
    await expect(toolbar.getByRole('button', { name, exact: true })).toBeDisabled();
  }
  await expect(toolbar.getByRole('button', { name: 'Reply', exact: true })).toBeEnabled();
  await expect(toolbar.getByRole('button', { name: 'Forward', exact: true })).toBeEnabled();
  console.log('PASS A sent reply keeps Reply and Forward and loses the actions that change its state');

  // ---- 10. Keyboard: arrows walk the rows, left and right fold ----
  await row.locator('.mail-open').click();
  await row.locator('.mail-open').focus();
  await page.keyboard.press('ArrowDown');
  await expect(reply(IDS.root)).toHaveClass(/selected/);
  await expect(reply(IDS.root).locator('.reply-open')).toBeFocused();
  await page.keyboard.press('ArrowUp');
  await row.locator('.mail-open').focus();
  await page.keyboard.press('ArrowLeft');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await page.keyboard.press('ArrowRight');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  console.log('PASS Arrow keys reach a reply and fold the conversation');

  await page.screenshot({ path: path.join(app.data, 'thread-rows-native.png') });

  // ---- 11. A fresh load with the listeners on: no CSP violation, no font error in the console ----
  // Last, because the app opens its first row at every start and a reload makes it mark a second message read.
  await page.reload();
  await expect(page.locator('.mail-row').first()).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  assert.deepEqual(await page.evaluate(() => window.__cspViolations), [], 'no CSP violation on a fresh load');
  assert.deepEqual(app.consoleProblems.filter((m) => /font|content security|refused|csp/i.test(m)), [], 'no font or CSP message in the console');
  assert.ok(app.fontResponses.length >= 2 && app.fontResponses.every((r) => r.status === 200), `font responses: ${JSON.stringify(app.fontResponses)}`);
  console.log('PASS A fresh load: no CSP violation and no font message in the console; every font file answered 200');
  assert.deepEqual(app.pageErrors, [], 'no page error');
  console.log(`Artifacts: ${app.data}`);
} catch (error) {
  await page.screenshot({ path: path.join(app.data, 'failure.png') }).catch(() => {});
  console.error(`console problems: ${JSON.stringify(app.consoleProblems)}\npage errors: ${JSON.stringify(app.pageErrors)}`);
  throw error;
} finally {
  await app.stop();
}
