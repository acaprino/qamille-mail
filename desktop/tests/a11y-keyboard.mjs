// Keyboard and screen reader names for the conversation rows (spec 2026-11-07 §4.3, ADR-0010):
// one walk with the keyboard alone through the sidebar, a conversation row, its arrow, the reply
// rows and a fold button, recording what an assistive technology is given (`ariaSnapshot`, accessible
// name and description) and whether each stop shows where the focus is. Synthetic data only.
//
// Checks D1 to D6 are the accessibility rules of the conversation rows (an unread message says so in
// words, a disclosure button keeps one name, the opened replies are a named group, the initials are
// not read before the name). They are collected, so a regression lists all of them, and make the
// run exit with 1 at the end.
// Artifacts (the names recorded at each stop): checks/a11y-keyboard/walk.txt
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { IDS, SUBJECT, withConversation } from './support/conversation-fixture.mjs';

const port = 28177;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const out = path.resolve(root, '../checks/a11y-keyboard');
await fs.mkdir(out, { recursive: true });
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));
const fixture = withConversation(sample);
for (const email of fixture.emails) email.reminder_at = null;

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];
const walk = [];
const defects = [];
const defect = (id, ok, message) => {
  console.log(`${ok ? 'PASS' : 'FAIL'} ${id}: ${message}`);
  if (!ok) defects.push(`${id}: ${message}`);
};

const first = async (locator) => ((await locator.ariaSnapshot()).split('\n')[0] || '').replace(/^- /, '').trim();
const row = (page) => page.locator('.mail-row', { hasText: SUBJECT });
const reply = (page, id) => page.locator(`.reply-row[data-reply-id="${id}"]`);

/** Does the focus show? The same box is photographed with the focus on the element and with it taken away. */
async function focusShows(page) {
  const box = await page.evaluate(() => {
    const r = document.activeElement.getBoundingClientRect();
    const x = Math.max(0, r.x - 4), y = Math.max(0, r.y - 4);
    return { x, y, width: Math.min(innerWidth - x, r.width + 8), height: Math.min(innerHeight - y, r.height + 8) };
  });
  const focused = await page.screenshot({ clip: box });
  await page.evaluate(() => { window.__focused = document.activeElement; document.activeElement.blur(); });
  await page.waitForTimeout(180);
  const plain = await page.screenshot({ clip: box });
  await page.evaluate(() => window.__focused.focus({ focusVisible: true }));
  await page.waitForTimeout(120);
  return !focused.equals(plain);
}
/** Where the focus is: its role and name as the browser computes them, and whether it shows. */
async function stop(page, label) {
  const active = page.locator(':focus');
  await expect(active).toHaveCount(1);
  const named = await first(active);
  const description = await active.evaluate((el) => {
    const ids = (el.getAttribute('aria-describedby') || '').split(/\s+/).filter(Boolean);
    return [...ids.map((id) => document.getElementById(id)?.textContent ?? ''), el.getAttribute('title') ?? ''].join(' | ').trim();
  });
  const shows = await focusShows(page);
  const size = await active.evaluate((el) => { const r = el.getBoundingClientRect(); return `${Math.round(r.width)}x${Math.round(r.height)}`; });
  walk.push(`${label.padEnd(34)} ${named.slice(0, 110)}${description ? `   [described: ${description.slice(0, 70)}]` : ''}   focus ${shows ? 'shows' : 'DOES NOT SHOW'}   ${size}`);
  return { named, description, shows, size };
}

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await page.goto(url);
  await expect(page.locator('.mail-row').first()).toBeVisible();
  await expect(page.locator('.message-title')).toBeVisible();
  await page.mouse.move(0, 0);

  // What is exposed before anything is focused: the unread reply, the closed arrow.
  await row(page).locator('.thread-toggle').click();
  await expect(page.locator('.reply-row')).toHaveCount(4);
  const unreadReply = reply(page, IDS.many).locator('.reply-open');
  const readReply = reply(page, IDS.a).locator('.reply-open');
  const unreadSnapshot = await first(unreadReply);
  const readSnapshot = await first(readReply);
  await fs.writeFile(path.join(out, 'aria-snapshot.txt'), [
    '--- the unread reply (B) as the accessibility tree has it', await unreadReply.ariaSnapshot(),
    '--- a read reply (A)', await readReply.ariaSnapshot(),
    '--- the whole opened conversation', await page.locator('.mail-list').ariaSnapshot(),
  ].join('\n'));

  // ---- D1: an unread reply says so ----
  // ThreadReplies.tsx puts aria-label on a span with no role, which assistive technology ignores on its
  // own; the question is whether the button's name, computed from its content, still carries it.
  defect('D1', /unread/i.test(unreadSnapshot) && !/unread/i.test(readSnapshot),
    `an unread reply announces "Unread" and a read one does not. Name of the unread one: "...${unreadSnapshot.slice(-60)}"; of a read one: "...${readSnapshot.slice(-40)}"`);
  // The main row does it through a description, so the contrast is real.
  const mainDescription = await row(page).locator('.mail-open').evaluate((el) => document.getElementById(el.getAttribute('aria-describedby'))?.textContent ?? '');
  assert.ok(/Unread|Pinned|\d{4}/.test(mainDescription), `the main row has a description (${mainDescription})`);

  // ---- D5: a fold that hides an unread reply says so. Nothing has been opened yet, so the unread
  // reply B is still unread; later in the walk every reply has been opened (and so read). ----
  const foldA = reply(page, IDS.a).locator('.reply-fold');
  await foldA.click();
  await expect(foldA).toHaveAttribute('aria-expanded', 'false');
  const foldHidesUnread = `${await first(foldA)} | ${(await foldA.getAttribute('title')) ?? ''}`;
  defect('D5', /\bunread\b/i.test(foldHidesUnread),
    `a folded button that hides an unread reply says so in words, not only in blue (ThreadReplies.tsx, foldTip): "${foldHidesUnread}"`);
  await foldA.click();
  await expect(page.locator('.reply-row')).toHaveCount(4);

  // ---- the walk: sidebar ----
  await page.reload();
  await expect(page.locator('.mail-row').first()).toBeVisible();
  await expect(page.locator('.message-title')).toBeVisible();
  await page.mouse.move(0, 0);
  let sidebarStops = 0;
  let reachedList = false;
  for (let i = 0; i < 70 && !reachedList; i++) {
    await page.keyboard.press('Tab');
    const where = await page.evaluate(() => {
      const el = document.activeElement;
      return { sidebar: !!el.closest('[data-slot="sidebar"]'), list: !!el.closest('.mail-list'), tag: el.tagName };
    });
    if (where.sidebar) {
      sidebarStops++;
      const s = await stop(page, `sidebar stop ${sidebarStops}`);
      assert.ok(s.named.replace(/^\w+\s*/, '').length > 0 || /"/.test(s.named), `sidebar stop ${sidebarStops} has a name (${s.named})`);
      assert.ok(s.shows, `sidebar stop ${sidebarStops} shows its focus (${s.named})`);
    }
    reachedList = where.list;
  }
  assert.ok(sidebarStops >= 5, `the sidebar is reachable with Tab (${sidebarStops} stops)`);
  assert.ok(reachedList, 'Tab reaches the list: no trap in the sidebar');
  console.log(`PASS Tab walks ${sidebarStops} sidebar stops, each with a name and a visible focus, and reaches the list`);

  // ---- the list is one Tab stop; the arrow keys reach the conversation row ----
  const entry = await stop(page, 'list entry (one Tab stop)');
  assert.ok(entry.shows, 'the list entry shows its focus');
  for (let i = 0; i < 12 && !(await row(page).locator('.mail-open').evaluate((el) => el === document.activeElement)); i++) {
    await page.keyboard.press('ArrowDown');
    await page.waitForTimeout(120);
  }
  await expect(row(page).locator('.mail-open')).toBeFocused();
  const main = await stop(page, 'conversation row (main message)');
  assert.ok(main.shows, 'the conversation row shows its focus');
  assert.ok(/Open/.test(main.named) && /Luca Bianchi/.test(main.named) && /Launch plan/.test(main.named), `the row names sender and subject (${main.named})`);

  // ---- Tab: pin, then the arrow ----
  await page.keyboard.press('Tab');
  const pin = await stop(page, 'pin (Tab)');
  assert.match(pin.named, /Pin thread/);
  await page.keyboard.press('Tab');
  const toggle = row(page).locator('.thread-toggle');
  await expect(toggle).toBeFocused();
  const arrow = await stop(page, 'conversation arrow (Tab)');
  assert.ok(arrow.shows, 'the arrow shows its focus');
  // After the reload every conversation is closed, and the unread reply B is hidden under the arrow.
  assert.match(arrow.named, /button "Conversation"/, `the arrow is a named button (${arrow.named})`);
  assert.ok(!/\[expanded\]/.test(arrow.named), 'and it is not expanded');
  assert.match(arrow.description, /5 messages in this thread/, 'its description gives the total');
  defect('D2', /\bunread\b/i.test(arrow.named + arrow.description) || !(await toggle.evaluate((el) => el.classList.contains('has-unread'))),
    `a closed arrow that hides an unread message says so in words, not only in blue (MailList.tsx). Name "${arrow.named}", description "${arrow.description}"`);
  await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('.reply-row')).toHaveCount(4);
  await expect(toggle).toBeFocused();
  const openArrow = await stop(page, 'arrow, open (Enter)');
  assert.match(openArrow.named, /button "Conversation" \[expanded\]/, `now the same name is expanded (${openArrow.named})`);
  console.log('PASS Enter on the arrow opens the conversation (four replies appear) and the focus stays on it');

  // ---- the reply rows: grouping and names ----
  const group = await page.locator('.thread-replies').evaluate((el) => ({ role: el.getAttribute('role'), label: el.getAttribute('aria-label') || el.getAttribute('aria-labelledby') }));
  defect('D3', !!group.role && !!group.label,
    `the opened replies form a named group or list of their own (ThreadReplies.tsx is a plain div: role=${group.role}, name=${group.label})`);

  // ---- arrows into the replies ----
  await page.keyboard.press('Shift+Tab'); await page.keyboard.press('Shift+Tab');
  await expect(row(page).locator('.mail-open')).toBeFocused();
  const names = [];
  for (const id of [IDS.root, IDS.a, IDS.sent, IDS.many]) {
    await page.keyboard.press('ArrowDown');
    await expect(reply(page, id).locator('.reply-open')).toBeFocused();
    const s = await stop(page, `reply ${id} (ArrowDown)`);
    names.push(s.named);
    assert.ok(s.shows, `reply ${id} shows its focus`);
    assert.match(s.named, /^button "Open /, `reply ${id} is a button named from its content (${s.named})`);
    assert.ok(/pressed/.test(s.named), `reply ${id} is the open one: aria-pressed is exposed (${s.named})`);
    assert.equal(await reply(page, id).locator('.reply-open').getAttribute('aria-pressed'), 'true');
  }
  console.log('PASS ArrowDown visits all four replies in tree order; each is a named, pressed button with a visible focus');
  defect('D4', !names.some((n) => /^button "Open [A-Z]{1,2} /.test(n)),
    `the avatar initials must not be read before the name (the span in ThreadReplies.tsx and MailList.tsx is not hidden): "${names[0].slice(0, 60)}"`);

  // ---- the fold button of reply A ----
  await page.keyboard.press('ArrowUp'); // sent
  await page.keyboard.press('ArrowUp'); // A
  await expect(reply(page, IDS.a).locator('.reply-open')).toBeFocused();
  await page.keyboard.press('Tab');
  const fold = reply(page, IDS.a).locator('.reply-fold');
  await expect(fold).toBeFocused();
  const foldLabelOpen = await fold.getAttribute('aria-label');
  const foldOpen = await stop(page, 'fold of A (Tab), open');
  assert.ok(foldOpen.shows, 'the fold button shows its focus');
  assert.match(foldOpen.named, /button "Replies to this message" \[expanded\]/, `named and expanded (${foldOpen.named})`);
  await page.keyboard.press('Enter');
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expect(fold).toBeFocused();
  await expect(page.locator('.reply-row')).toHaveCount(2);
  const foldClosed = await stop(page, 'fold of A, closed (Enter)');
  assert.ok(foldClosed.shows, 'the folded button shows its focus');
  assert.match(foldClosed.named, /button "Replies to this message"/, `the name does not change with the state (${foldClosed.named})`);
  assert.match(foldClosed.description, /Show the 2 hidden replies/, `its description says how many it hides (${foldClosed.description})`);
  const foldLabelClosed = await fold.getAttribute('aria-label');
  defect('D6', foldLabelOpen === foldLabelClosed || (await fold.getAttribute('aria-expanded')) === null,
    `a disclosure button keeps one name and lets aria-expanded carry the state; ThreadReplies.tsx changes the name ("${foldLabelOpen}" / "${foldLabelClosed}") and also sets aria-expanded, so it is announced as "Hide the replies to this message, expanded" or "Show the 2 hidden replies, collapsed". The conversation arrow does the same (MailList.tsx: "Open the conversation" / "Close the conversation")`);
  await page.keyboard.press('Enter');
  await expect(fold).toHaveAttribute('aria-expanded', 'true');
  await expect(page.locator('.reply-row')).toHaveCount(4);
  console.log('PASS Enter on a fold button hides and restores what is under it, the focus stays on the button');

  // ---- leaving the list and opening a reply with Enter ----
  await page.keyboard.press('Shift+Tab');
  await expect(reply(page, IDS.a).locator('.reply-open')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('.message-body').first()).toContainText('I can take the schedule');
  const after = [];
  for (let i = 0; i < 14; i++) {
    await page.keyboard.press('Tab');
    const inList = await page.evaluate(() => !!document.activeElement.closest('.mail-list'));
    const s = await stop(page, `Tab ${i + 1} after reply A${inList ? ' (still in the list)' : ''}`);
    assert.ok(s.shows, `Tab ${i + 1} after the replies shows its focus (${s.named})`);
    after.push({ inList, named: s.named });
    if (!inList) break;
  }
  assert.ok(after.length < 14 && !after.at(-1).inList, `Tab leaves the list within ${after.length} stops: no trap in the replies`);
  const next = after.at(-1);
  console.log(`PASS Enter opens a reply in the reading pane; ${after.length} Tab stops later the focus leaves the list for "${next.named.slice(0, 40)}"`);

  // ---- target size of the two arrows (WCAG 2.2 AA 2.5.8 asks 24x24 or room around), noted, not asserted ----
  const sizes = await page.evaluate(() => ({
    toggle: [...document.querySelectorAll('.thread-toggle')].map((el) => `${Math.round(el.getBoundingClientRect().width)}x${Math.round(el.getBoundingClientRect().height)}`),
    fold: [...document.querySelectorAll('.reply-fold')].map((el) => `${Math.round(el.getBoundingClientRect().width)}x${Math.round(el.getBoundingClientRect().height)}`),
  }));
  const small = [...sizes.toggle, ...sizes.fold].filter((size) => size.split('x').some((side) => Number(side) < 24));
  assert.deepEqual(small, [], `the arrows are at least 24x24 px (WCAG 2.2 AA 2.5.8): ${small.join(', ')}`);
  walk.push('', `target sizes: .thread-toggle ${sizes.toggle.join(', ')}  .reply-fold ${sizes.fold.join(', ')}  (WCAG 2.2 AA 2.5.8 wants 24x24 unless spaced)`);

  // ---- the composer: a reply starts in the text, and sending hands the focus back to the list ----
  await page.reload();
  await expect(page.locator('.mail-row').first()).toBeVisible();
  await expect(page.locator('.message-title')).toBeVisible();
  await page.mouse.move(0, 0);
  await row(page).locator('.mail-open').focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.mail-row.selected')).toHaveCount(1);
  await expect(page.locator('.message-body').first()).toContainText('I will send the schedule on Monday');
  await page.keyboard.press('r');
  const composer = page.getByRole('tabpanel');
  const text = composer.getByRole('textbox', { name: 'Message body' });
  await expect(text).toBeFocused();
  await page.keyboard.type('ciao');
  await expect(text).toHaveText('ciao');
  assert.equal(await composer.getByRole('combobox', { name: 'To', exact: true }).inputValue(), '', 'what was typed did not go into the To field');
  const body = await stop(page, 'composer: the text of a reply');
  assert.ok(body.shows, 'the text of the message shows its focus');
  // A caret alone would make the photographs differ: the ring itself is what must be there.
  const ring = () => page.evaluate(() => { const { outlineStyle, outlineWidth } = getComputedStyle(document.activeElement); return `${outlineStyle} ${outlineWidth}`; });
  assert.equal(await ring(), 'solid 2px', 'the text of the message has the focus ring of every field');
  await composer.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(composer).toHaveCount(0);
  await expect(page.locator('.mail-row.selected .mail-open')).toBeFocused();
  console.log('PASS R opens a reply in the text ("ciao" lands there) and Send puts the focus back on the selected row');

  // The list's own heading takes the focus when nothing is selected; it shows where it is.
  await page.evaluate(() => document.querySelector('.mail-list h1').focus({ focusVisible: true }));
  const heading = await stop(page, 'list heading (the fallback)');
  assert.ok(heading.shows, 'the heading that takes the focus when nothing is selected shows its focus');
  assert.equal(await ring(), 'solid 2px', 'the heading has the focus ring of every control');

  await fs.writeFile(path.join(out, 'walk.txt'), walk.join('\n'));
  assert.deepEqual(errors, [], 'no page error');
} finally {
  await fs.writeFile(path.join(out, 'walk.txt'), walk.join('\n')).catch(() => {});
  await browser?.close();
  server.kill();
}
console.log(`\nWalk recorded in ${path.join(out, 'walk.txt')}`);
if (defects.length) {
  console.error(`\n${defects.length} known defect(s):\n- ${defects.join('\n- ')}`);
  process.exit(1);
}
