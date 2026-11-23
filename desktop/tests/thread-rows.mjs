// A conversation opens in the list as plain rows (spec 2026-11-07 §4.3, ADR-0010): one row per
// message on one edge, who each answers beside the name, a fold per reply, a count that never
// hides what is unread. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { IDS, MARIA, ME, SUBJECT, withConversation } from './support/conversation-fixture.mjs';

const port = 28174;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];

async function open(fixture, viewport = { width: 1440, height: 900 }) {
  const page = await browser.newPage({ locale: 'en-US', viewport });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await page.goto(url);
  await expect(page.locator('.mail-row').first()).toBeVisible();
  return page;
}
const row = (page) => page.locator('.mail-row', { hasText: SUBJECT });
const toggle = (page) => row(page).locator('.thread-toggle');
const reply = (page, id) => page.locator(`.reply-row[data-reply-id="${id}"]`);
const replies = (page) => page.locator('.reply-row');
const text = async (locator) => (await locator.innerText()).replace(/\s+/g, ' ').trim();
const colorOf = (page, name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }, name);
const openConversation = async (page) => {
  if ((await toggle(page).getAttribute('aria-expanded')) !== 'true') await toggle(page).click();
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true');
};

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const fixture = withConversation(sample);

  // ---- closed, then open ----
  let page = await open(fixture);
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
  assert.equal((await text(toggle(page))).replace(/\D/g, ''), '5', 'the arrow carries the total of messages');
  await expect(replies(page)).toHaveCount(0);
  await toggle(page).click();
  await expect(replies(page)).toHaveCount(4);
  const lefts = await replies(page).locator('.reply-head strong').evaluateAll((nodes) => nodes.map((n) => Math.round(n.getBoundingClientRect().left)));
  assert.equal(new Set(lefts).size, 1, `every reply is on the same edge (${lefts})`);
  assert.deepEqual(
    await replies(page).evaluateAll((nodes) => nodes.map((n) => Number(n.dataset.replyId))),
    [IDS.root, IDS.a, IDS.sent, IDS.many],
    'tree order, the sent reply among them',
  );
  assert.equal(await row(page).count(), 1, 'the main row is not repeated among the replies');
  console.log('PASS A closed conversation shows the total and opens with its arrow');

  // ---- who each reply answers ----
  const to = async (id) => text(reply(page, id).locator('.reply-to'));
  await expect.poll(() => to(IDS.a)).toBe('to Maria Rossi +1');
  assert.equal(await to(IDS.root), 'to you', 'the reader is "you"');
  assert.equal(await to(IDS.sent), 'to Luca Bianchi +1');
  assert.equal(await to(IDS.many), 'to Luca Bianchi +11', 'twelve recipients: the one answered and the others counted');
  const tip = await reply(page, IDS.many).locator('.reply-to').getAttribute('title');
  assert.ok(tip.includes('Luca Bianchi') && tip.includes('you') && tip.length < 400, `the tooltip names them (${tip})`);
  console.log('PASS A reply names who it answers');

  // ---- no row grows with many recipients ----
  for (const [width, height] of [[1440, 900], [1000, 640]]) {
    await page.setViewportSize({ width, height });
    await page.waitForTimeout(150);
    const boxes = await replies(page).evaluateAll((nodes) => nodes.map((n) => {
      const b = n.getBoundingClientRect();
      const t = n.querySelector('time').getBoundingClientRect();
      const more = n.querySelector('.to-more')?.getBoundingClientRect();
      const list = n.closest('.thread-replies').getBoundingClientRect();
      return { height: Math.round(b.height * 10) / 10, timeRight: t.right, timeLeft: t.left, moreRight: more?.right ?? 0, moreWidth: more?.width ?? 0, listRight: list.right, listLeft: list.left };
    }));
    assert.equal(new Set(boxes.map((b) => b.height)).size, 1, `${width}px: every reply row has the same height (${boxes.map((b) => b.height)})`);
    for (const b of boxes) {
      assert.ok(b.timeLeft >= b.listLeft && b.timeRight <= b.listRight + 0.5, `${width}px: the date stays inside the list`);
      assert.ok(b.moreRight <= b.listRight + 0.5, `${width}px: the count stays inside the list`);
    }
    assert.ok(boxes[3].moreWidth > 0, `${width}px: "+11" is still there`);
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  console.log('PASS A row does not grow with many recipients');

  // ---- fold a reply ----
  const fold = reply(page, IDS.a).locator('.reply-fold');
  assert.equal(await fold.getAttribute('aria-expanded'), 'true');
  assert.equal(await reply(page, IDS.root).locator('.reply-fold').count(), 1, 'R has A under it');
  assert.equal(await reply(page, IDS.many).locator('.reply-fold').count(), 0, 'B has nothing under it');
  await fold.click();
  await expect(replies(page)).toHaveCount(2);
  assert.deepEqual(await replies(page).evaluateAll((nodes) => nodes.map((n) => Number(n.dataset.replyId))), [IDS.root, IDS.a]);
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  assert.equal((await text(fold)).replace(/\D/g, ''), '2', 'the folded reply says how many it hides');
  assert.equal(await fold.evaluate((el) => getComputedStyle(el).color), await colorOf(page, '--color-accent'), 'blue: one of them is unread');
  await fold.click();
  await expect(replies(page)).toHaveCount(4);
  console.log('PASS Folding a reply hides everything under it and keeps a count');

  // ---- no checkbox, pin or drag on a reply ----
  assert.equal(await replies(page).locator('.mail-checkbox, .row-pin').count(), 0);
  assert.notEqual(await replies(page).first().locator('.reply-open').getAttribute('draggable'), 'true');
  console.log('PASS Reply rows have no checkbox, pin or drag');

  // ---- opening a reply ----
  assert.ok(/has-unread/.test(await (async () => { await toggle(page).click(); const c = await toggle(page).getAttribute('class'); await toggle(page).click(); return c; })()), 'closed: the arrow shows an unread message below');
  await expect(replies(page)).toHaveCount(4);
  await reply(page, IDS.many).locator('.reply-open').click();
  await expect(page.locator('.message-body').first()).toContainText('Adding the rest of the team');
  await expect(page.locator('.message-sender strong')).toHaveText('Maria Rossi');
  await expect(reply(page, IDS.many)).toHaveClass(/selected/);
  await expect(reply(page, IDS.many)).not.toHaveClass(/unread/);
  await expect(row(page)).not.toHaveClass(/selected/);
  await expect(row(page).locator('.unread-dot, .mail-open .unread-dot')).toHaveCount(0);
  assert.equal(await replies(page).locator('.unread-dot').count(), 0, 'nothing unread is left in the conversation');
  await toggle(page).click();
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
  assert.ok(!/has-unread/.test(await toggle(page).getAttribute('class')), 'closed again: the arrow no longer shows an unread message');
  await toggle(page).click();
  await expect(replies(page)).toHaveCount(4);
  await row(page).locator('.mail-open').click();
  await expect(page.locator('.message-body').first()).toContainText('I will send the schedule on Monday');
  await expect(row(page)).toHaveClass(/selected/);
  await expect(replies(page).locator(':scope.selected')).toHaveCount(0);
  console.log('PASS Opening a reply reads it, and the main row takes the reader back');

  // ---- only the reply under the pointer reacts ----
  await page.mouse.move(0, 0);
  const background = (locator) => locator.evaluate((el) => getComputedStyle(el).backgroundColor);
  const mainBefore = await background(row(page));
  const replyBefore = await background(reply(page, IDS.a));
  await reply(page, IDS.a).locator('.reply-open').hover();
  assert.equal(await background(reply(page, IDS.a)), await colorOf(page, '--hover'), 'the reply under the pointer is highlighted');
  assert.notEqual(replyBefore, await colorOf(page, '--hover'));
  assert.equal(await background(row(page)), mainBefore, 'the main row stays as it was');
  await row(page).locator('.mail-open').hover();
  assert.equal(await background(reply(page, IDS.a)), replyBefore, 'and the reply does not follow the main row');
  console.log('PASS Only the reply under the pointer reacts');

  // ---- arrow keys ----
  await row(page).locator('.mail-open').focus();
  await page.keyboard.press('ArrowDown');
  await expect(reply(page, IDS.root)).toHaveClass(/selected/);
  await expect(page.locator('.message-body').first()).toContainText('We need to decide who owns the launch plan');
  await expect(reply(page, IDS.root).locator('.reply-open')).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(reply(page, IDS.a)).toHaveClass(/selected/);
  await page.keyboard.press('ArrowUp');
  await expect(reply(page, IDS.root)).toHaveClass(/selected/);
  await row(page).locator('.mail-open').click();
  await row(page).locator('.mail-open').focus();
  await page.keyboard.press('ArrowLeft');
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
  await expect(replies(page)).toHaveCount(0);
  await page.keyboard.press('ArrowRight');
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true');
  await expect(replies(page)).toHaveCount(4);
  console.log('PASS Arrow keys visit the replies and fold the conversation');
  await page.close();

  // ---- what the reader can do with a message that is only shown (ADR-0010, INV-002) ----
  page = await open(fixture);
  await openConversation(page);
  const toolbar = page.locator('.reader-toolbar');
  const stateActions = [
    toolbar.getByRole('button', { name: 'Archive message', exact: true }),
    toolbar.getByRole('button', { name: 'Snooze until tomorrow', exact: true }),
    toolbar.getByRole('button', { name: /^(Pin|Unpin) message$/ }),
    toolbar.getByRole('button', { name: 'Mark message unread', exact: true }),
    toolbar.getByRole('button', { name: 'Move message to trash', exact: true }),
    toolbar.getByRole('button', { name: 'Done', exact: true }),
  ];
  const changes = () => page.evaluate(() => window.__previewCalls.filter((c) => c === 'mail_action' || c === 'queue_mail_action').length);

  // The user's own reply lives in Sent, outside the Inbox view: it can be read and answered, not managed from here.
  await reply(page, IDS.sent).locator('.reply-open').click();
  await expect(page.locator('.message-body').first()).toContainText('Budget is with Maria');
  for (const button of stateActions) await expect(button).toBeDisabled();
  for (const button of stateActions.slice(0, 5)) {
    assert.equal(await button.getAttribute('title').then((t) => /Sent message: manage it from Sent/.test(t)), true, 'and says why');
  }
  await expect(toolbar.getByRole('button', { name: 'Reply', exact: true })).toBeEnabled();
  await expect(toolbar.getByRole('button', { name: 'Forward', exact: true })).toBeEnabled();
  await toolbar.getByRole('button', { name: 'More message actions' }).click();
  await expect(page.getByRole('menuitem', { name: 'Move to inbox', exact: true })).toBeDisabled();
  await expect(page.getByRole('menuitemcheckbox', { name: 'Set aside', exact: true })).toBeDisabled();
  await expect(page.getByRole('menuitem', { name: 'Remove snooze', exact: true }), 'no snooze to remove on a message that is not snoozed').toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Print email', exact: true })).toBeEnabled();
  await page.keyboard.press('Escape');
  const calls = await changes();
  await page.locator('.reply-row.selected .reply-open').focus();
  await page.keyboard.press('e');
  await page.waitForTimeout(300);
  assert.equal(await changes(), calls, 'E does nothing on a message that is only shown');
  await expect(page.locator('.message-body').first()).toContainText('Budget is with Maria');
  console.log('PASS A sent reply opened from the list cannot be acted on from the reader');

  // A message of the view keeps every action, wherever it is opened from.
  await reply(page, IDS.a).locator('.reply-open').click();
  await expect(page.locator('.message-body').first()).toContainText('I can take the schedule');
  for (const button of stateActions) await expect(button).toBeEnabled();
  await row(page).locator('.mail-open').click();
  await expect(page.locator('.message-body').first()).toContainText('I will send the schedule on Monday');
  for (const button of stateActions) await expect(button).toBeEnabled();
  console.log('PASS A message of the view keeps every action');

  // The conversation is in the list: the reader no longer repeats it.
  await expect(page.getByRole('region', { name: 'Conversation messages' })).toHaveCount(0);
  assert.equal(await page.locator('.reader .conversation, .reader .thread-message').count(), 0);
  console.log('PASS The reader no longer lists the conversation');
  await page.close();

  // ---- new mail keeps the conversation open ----
  page = await open(fixture);
  await openConversation(page);
  const before = await page.evaluate(async () => (await (await import('/src/lib/api.ts')).call('list_mail', { query: { folder: 'Inbox', account_id: null, search: '', offset: 0, limit: 60, threads: true } })).items.find((i) => i.subject === 'Launch plan: who owns what?').conversation_id);
  await page.evaluate((id) => {
    // B becomes the newest message of the conversation: the main row changes, the conversation does not.
    window.__previewEdit(id, { date: 1_793_700_000 });
    window.dispatchEvent(new Event('preview-workspace-changed'));
  }, IDS.many);
  await expect.poll(async () => page.locator('.mail-row', { hasText: SUBJECT }).locator('.mail-meta strong').innerText()).toBe('Maria Rossi');
  const after = await page.evaluate(async () => (await (await import('/src/lib/api.ts')).call('list_mail', { query: { folder: 'Inbox', account_id: null, search: '', offset: 0, limit: 60, threads: true } })).items.find((i) => i.subject === 'Launch plan: who owns what?'));
  assert.equal(after.id, IDS.many, 'the newest message is now the main row');
  assert.equal(after.conversation_id, before, 'the conversation keeps its identity');
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'true');
  await expect(replies(page)).toHaveCount(4);
  console.log('PASS An open conversation survives new mail');
  await page.close();

  // ---- beyond the cap ----
  const long = withConversation(sample, { extraReplies: 31 });
  page = await open(long);
  await openConversation(page);
  await expect(replies(page)).toHaveCount(30);
  const more = page.locator('.reply-more');
  await expect(more).toHaveText('Show 5 more replies');
  assert.equal((await text(toggle(page))).replace(/\D/g, ''), '36', 'the arrow keeps the total (36 messages)');
  await more.click();
  await expect(replies(page)).toHaveCount(35);
  await expect(more).toHaveCount(0);
  assert.equal((await text(toggle(page))).replace(/\D/g, ''), '36', 'and still does');
  console.log('PASS "Show more" loads the rest');
  await page.close();

  // ---- Final review, Important 1: an unread reply beyond the cap still marks the closed arrow ----
  // 35 other messages, 30 listed; the one unread message is the last of the five not listed.
  const lastBeyond = IDS.extra + 29;
  page = await open(withConversation(sample, { extraReplies: 31, read: { [IDS.many]: true, [lastBeyond]: false } }));
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
  await expect(toggle(page)).toHaveClass(/has-unread/);
  assert.equal(await toggle(page).evaluate((el) => getComputedStyle(el).color), await colorOf(page, '--color-accent'), 'the closed arrow is blue');
  await toggle(page).click();
  await expect(replies(page)).toHaveCount(30);
  await page.locator('.reply-more').click();
  await expect(replies(page)).toHaveCount(35);
  await expect(reply(page, lastBeyond)).toHaveClass(/unread/);

  // Final review, Important 2: what "Show more" loaded follows what the reader does.
  await reply(page, lastBeyond).locator('.reply-open').click();
  await expect(page.locator('.message-body').first()).toContainText(`Extra reply number ${29 + 1}`);
  await expect(reply(page, lastBeyond)).not.toHaveClass(/unread/);
  assert.equal(await reply(page, lastBeyond).locator('.unread-dot').count(), 0, 'its dot is gone');
  await toggle(page).click();
  await expect(toggle(page)).toHaveAttribute('aria-expanded', 'false');
  assert.ok(!/has-unread/.test(await toggle(page).getAttribute('class')), 'closed again: nothing unread is left, so the arrow is not blue');
  await toggle(page).click();
  await expect(replies(page)).toHaveCount(35);
  console.log('PASS An unread reply beyond the cap marks the arrow, and a reply read after "Show more" clears it');

  // A message trashed since "Show more" loaded it leaves the list: the extras do not outlive the data.
  await page.evaluate((id) => {
    window.__previewEdit(id, { folder: 'Trash' });
    window.dispatchEvent(new Event('preview-workspace-changed'));
  }, IDS.extra + 27);
  await expect(replies(page)).toHaveCount(30);
  await expect(page.locator('.reply-more')).toHaveText('Show 4 more replies');
  await expect(page.locator(`.reply-row[data-reply-id="${IDS.extra + 27}"]`)).toHaveCount(0);
  await page.locator('.reply-more').click();
  await expect(replies(page)).toHaveCount(34);
  await expect(page.locator(`.reply-row[data-reply-id="${IDS.extra + 27}"]`)).toHaveCount(0);
  console.log('PASS A message trashed after "Show more" does not stay listed');
  await page.close();

  // ---- Final review, Important 3: keyboard triage with a conversation open ----
  page = await open(fixture);
  await openConversation(page);
  const messageBody = () => page.locator('.message-body').first();
  await reply(page, IDS.a).locator('.reply-open').click();
  await expect(messageBody()).toContainText('I can take the schedule');
  // A is a message of the view; the one after it is the user's own Sent reply, which triage skips.
  await reply(page, IDS.a).locator('.reply-open').focus();
  await page.keyboard.press('e');
  await expect(messageBody()).toContainText('Adding the rest of the team', { timeout: 5000 });
  await expect(page.locator('.message-title')).toHaveText(SUBJECT);
  await expect(reply(page, IDS.many)).toHaveClass(/selected/);
  await expect(page.locator('.reader-empty')).toHaveCount(0);
  // B is the last line of the conversation: the next one down the list is the next row.
  await reply(page, IDS.many).locator('.reply-open').focus();
  await page.keyboard.press('e');
  await expect(messageBody()).toContainText('the design review is now at 3 pm', { timeout: 5000 });
  await expect(page.locator('.mail-row.selected', { hasText: 'Design review moved to 3 pm' })).toHaveCount(1);
  console.log('PASS Triage from a reply moves on to the next message of the view');

  // ---- Final review, Minor 1 raised to Important: a Sent reply stays locked once its row is gone ----
  await page.close();
  page = await open(fixture);
  await openConversation(page);
  await reply(page, IDS.sent).locator('.reply-open').click();
  await expect(messageBody()).toContainText('Budget is with Maria');
  const lockedToolbar = page.locator('.reader-toolbar');
  const lockedActions = [
    lockedToolbar.getByRole('button', { name: 'Archive message', exact: true }),
    lockedToolbar.getByRole('button', { name: 'Move message to trash', exact: true }),
    lockedToolbar.getByRole('button', { name: 'Done', exact: true }),
  ];
  for (const button of lockedActions) await expect(button).toBeDisabled();
  await page.evaluate((ids) => {
    // The conversation is moved out of the Inbox (by a drag, a rule, another window): its row goes.
    for (const id of ids) window.__previewEdit(id, { folder: 'Archive' });
    window.dispatchEvent(new Event('preview-workspace-changed'));
  }, [IDS.root, IDS.a, IDS.many, IDS.main]);
  await expect(row(page)).toHaveCount(0);
  await expect(messageBody()).toContainText('Budget is with Maria');
  for (const button of lockedActions) await expect(button).toBeDisabled();
  console.log('PASS A Sent reply stays locked after its conversation leaves the list');
  await page.close();

  // ---- phase 3, Task 6: the native menu of a row (docs/aspetto-nativo.md, rule 33) ----
  // The preview has no native menu: its `show_row_menu` keeps what it was asked to show and answers
  // with the entry the test prepared.
  page = await open(fixture);
  const rowMenu = (choice) =>
    page.evaluate((value) => {
      window.__previewRowMenu = { calls: [], choice: value };
    }, choice);
  const menuCalls = () => page.evaluate(() => window.__previewRowMenu.calls);
  await rowMenu('archive');
  // The app opens the first message at start: the conversation right-clicked is another one.
  await expect(page.locator('.message-title')).toHaveCount(1);
  await expect(row(page)).not.toHaveClass(/selected/);
  const onShow = await page.locator('.message-title').innerText();
  await page.evaluate(() => (window.__previewCalls = []));
  const opener = row(page).locator('.mail-open');
  const pointer = await opener.boundingBox();
  await opener.click({ button: 'right' });
  await expect(row(page)).toHaveCount(0);
  assert.ok(
    await page.evaluate(() => !!document.activeElement?.matches('.mail-row .mail-open')),
    'the focus moves on to the next row, not to the page',
  );
  let shown = await menuCalls();
  assert.equal(shown.length, 1, 'one right-click, one menu');
  const offered = shown[0].items.map((item) => item.value);
  for (const value of ['reply', 'reply-all', 'forward', 'archive', 'moveToTrash', 'markAsDone', 'snooze', 'pin'])
    assert.ok(offered.includes(value), `the row offers ${value}: ${offered}`);
  assert.ok(!offered.includes('moveToInbox'), 'an Inbox row has nothing to move back');
  assert.ok(Math.abs(shown[0].x - (pointer.x + pointer.width / 2)) <= 1, `under the pointer (x ${shown[0].x})`);
  assert.ok(Math.abs(shown[0].y - (pointer.y + pointer.height / 2)) <= 1, `under the pointer (y ${shown[0].y})`);
  assert.equal(
    (await page.evaluate(() => window.__previewCalls)).filter((name) => name === 'queue_mail_action').length,
    4,
    'Archive takes the messages of the conversation the Inbox holds, not the Sent reply',
  );
  assert.equal(await page.locator('.message-title').innerText(), onShow, 'the reader keeps the message it showed');
  assert.ok(!(await page.evaluate(() => window.__previewCalls)).includes('read_mail'), 'the row was not opened');
  console.log('PASS A right-click on a row opens the native menu; Archive archives that conversation without opening it');

  // Two ticked rows: the menu is for both of them, with the group actions only.
  const ticks = page.locator('.mail-row .mail-checkbox');
  await ticks.nth(0).check();
  await ticks.nth(1).check();
  await rowMenu(null);
  await page.locator('.mail-row').nth(1).locator('.mail-open').click({ button: 'right' });
  await expect.poll(async () => (await menuCalls()).length).toBe(1);
  const group = (await menuCalls())[0].items.map((item) => item.value);
  assert.ok(!group.includes('reply') && !group.includes('forward') && group.includes('archive'), `group actions only: ${group}`);
  await expect(ticks.nth(0)).toBeChecked();
  console.log('PASS On ticked rows the menu offers only what applies to all of them');
  await page.close();

  // Shift+F10 on the row with the focus: the menu opens below the row, and the focus comes back to it.
  page = await open(fixture);
  await page.getByRole('tab', { name: 'All', exact: true }).click();
  const focused = page.locator('.mail-row').first();
  const wasUnread = /\bunread\b/.test(await focused.getAttribute('class'));
  await rowMenu(wasUnread ? 'markAsSeen' : 'markAsUnseen');
  // The native menu holds the keyboard while it is open: the focus is not on the row meanwhile.
  await page.evaluate(() => (window.__previewCommandDelay = { show_row_menu: 600 }));
  await focused.locator('.mail-open').focus();
  const below = await focused.boundingBox();
  await page.keyboard.press('Shift+F10');
  await page.locator('.mail-list h1').focus();
  await expect.poll(async () => (await menuCalls()).length).toBe(1);
  shown = await menuCalls();
  assert.ok(Math.abs(shown[0].x - (below.x + 16)) <= 1, `16 px from the left edge of the row (x ${shown[0].x}, row ${below.x})`);
  assert.ok(Math.abs(shown[0].y - (below.y + below.height)) <= 1, `below the row (y ${shown[0].y}, row ${below.y}+${below.height})`);
  if (wasUnread) await expect(focused).not.toHaveClass(/\bunread\b/);
  else await expect(focused).toHaveClass(/\bunread\b/);
  await expect(focused.locator('.mail-open')).toBeFocused();
  console.log('PASS Shift+F10 opens the menu below the focused row, and the focus comes back to the row');
  await page.close();

  // Reply all from the menu of a message that is not open: its recipients are read for the composer.
  page = await open(fixture);
  await page.getByRole('button', { name: 'Emails', exact: true }).click();
  const many = page.locator('.mail-row.unread', { hasText: SUBJECT });
  await expect(many).toHaveCount(1);
  await expect(many).not.toHaveClass(/selected/);
  await rowMenu('reply-all');
  await many.locator('.mail-open').click({ button: 'right' });
  const toField = page.getByRole('combobox', { name: 'To', exact: true });
  await expect(toField).toBeVisible();
  const toChips = await page
    .locator('.field-row')
    .filter({ has: toField })
    .locator('[data-scope="tags-input"][data-part="item-preview"]')
    .evaluateAll((nodes) => nodes.map((n) => n.getAttribute('title')));
  assert.ok(toChips.some((entry) => entry.includes(MARIA[1])), `the sender is in To: ${toChips}`);
  assert.ok(toChips.some((entry) => entry.includes('guest9@example.test')), `the other recipients are in To: ${toChips}`);
  assert.ok(!toChips.some((entry) => entry.includes(ME)), 'the reader is not in To');
  await expect(many).not.toHaveClass(/selected/);
  console.log('PASS Reply all from the menu of a row that is not open writes to everyone');
  await page.close();

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
