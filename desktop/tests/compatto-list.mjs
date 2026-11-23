// Task 8 of the Compatto restyle (spec 2026-11-08 ark-compatto, section 6): the message list, its
// conversations and the search filters. Values are the mockup's, read as computed styles; behavior
// goes through role and accessible name. Synthetic data only.
//
//   node tests/compatto-list.mjs      (QAMILLE_PREVIEW_URL names a Vite of your own, not 1420)
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { previewUrl, underline } from './support/compatto.mjs';
import { IDS, withConversation } from './support/conversation-fixture.mjs';

const root = path.resolve(import.meta.dirname, '..');
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));

const ACCENT = 'rgb(79, 70, 229)';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];

async function open({ fixture = sample, size = { width: 1440, height: 900 }, locale = 'en-US', threads = false } = {}) {
  const context = await browser.newContext({ locale, viewport: size });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((mode) => localStorage.setItem('qamille:mail-display', mode), threads ? 'threads' : 'emails');
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await page.goto(previewUrl);
  await expect(page.locator('.mail-row').first()).toBeVisible();
  return page;
}
const style = (locator, property) =>
  locator.first().evaluate((node, name) => getComputedStyle(node).getPropertyValue(name), property);

// A conversation whose sender has 80 characters and one of whose messages went to 40 people.
const long = (name) => `${name} ${'x'.repeat(80 - name.length - 1)}`;
function crowded() {
  const fixture = withConversation(sample);
  const forty = Array.from({ length: 40 }, (_, i) => `guest${i + 1}@example.test`);
  for (const email of fixture.emails) {
    if (email.id === IDS.main) email.from_name = long('Luca Bianchi');
    if (email.id === IDS.many) {
      email.from_name = long('Maria Rossi');
      email.to = forty;
    }
    if (email.id === IDS.a) email.from_name = long('Luca Bianchi');
  }
  const main = fixture.emails.find((email) => email.id === IDS.main);
  main.attachments = structuredClone(sample.emails.find((email) => email.attachments?.length).attachments);
  return fixture;
}

try {
  // ---- the heading, the search and the tabs ----
  let page = await open();
  const h1 = page.locator('.mail-list h1');
  assert.equal(await style(h1, 'font-size'), await page.evaluate(() => { const p = document.createElement('i'); p.style.fontSize = 'var(--text-base)'; document.body.append(p); const v = getComputedStyle(p).fontSize; p.remove(); return v; }), 'the view title is --text-base');
  assert.equal(await style(h1, 'font-weight'), '600');
  assert.equal(Math.round((await page.locator('.global-search').boundingBox()).height), 32, 'the search is 32px high');
  const tablist = page.getByRole('tablist', { name: 'Inbox view' });
  await expect(tablist.getByRole('tab')).toHaveText(['Smart', 'All', 'Unread']);
  assert.equal(await tablist.getAttribute('data-slot'), 'tabs-list', 'the tabs are Sigrid\u2019s');
  const chosen = tablist.getByRole('tab', { name: 'Smart', exact: true });
  await expect(chosen).toHaveAttribute('aria-selected', 'true');
  await expect(tablist.getByRole('tab', { name: 'Unread', exact: true })).toHaveAttribute('aria-selected', 'false');
  assert.equal(await tablist.getByRole('tab').count(), 3, 'no "with attachments" tab');
  await tablist.getByRole('tab', { name: 'Unread', exact: true }).click();
  await expect(tablist.getByRole('tab', { name: 'Unread', exact: true })).toHaveAttribute('aria-selected', 'true');
  assert.equal(await page.locator('.group-heading').count(), 0, 'Unread is a flat list, as before');
  await tablist.getByRole('tab', { name: 'Smart', exact: true }).click();
  await expect(page.locator('.group-heading').first()).toBeVisible();
  console.log('PASS The heading is 15/600, the search 32px and the tabs underlined');

  // ---- the group labels ----
  const group = page.locator('.group-heading').first();
  assert.equal(await style(group, 'font-size'), '12px');
  assert.equal(await style(group, 'text-transform'), 'uppercase');
  assert.equal(await style(group, 'font-weight'), '600');
  assert.equal(await style(group, 'background-color'), 'rgb(246, 247, 249)');
  console.log('PASS Group labels are 12px, uppercase, on the ground');

  // ---- a row: two lines, no avatar, the unread marks, the selection ----
  assert.equal(await page.locator('.sender-avatar, .reply-avatar').count(), 0, 'no avatar');
  const unread = page.locator('.mail-row.unread:not(.selected)').first();
  const dot = unread.locator('.unread-dot');
  const dotBox = await dot.boundingBox();
  assert.equal(Math.round(dotBox.width) + 'x' + Math.round(dotBox.height), '6x6', 'the dot is 6px');
  assert.equal(await style(dot, 'background-color'), ACCENT);
  assert.equal(await style(unread.locator('.mail-meta strong'), 'font-weight'), '600', 'an unread sender is bold');
  assert.equal(await style(unread.locator('.mail-meta time'), 'color'), ACCENT, 'and its time is the accent');
  const lines = async (row) => {
    const sender = await row.locator('.mail-meta strong').boundingBox();
    const time = await row.locator('.mail-meta time').boundingBox();
    const subject = await row.locator('.mail-subject-text').boundingBox();
    const snippet = await row.locator('.mail-snippet').boundingBox();
    return { sender, time, subject, snippet, row: await row.boundingBox() };
  };
  const two = await lines(unread);
  assert.ok(Math.abs(two.sender.y - two.time.y) < 6, 'the sender and the time share the first line');
  assert.ok(two.subject.y > two.sender.y + two.sender.height - 2, 'the subject is on the second line');
  assert.ok(Math.abs(two.subject.y - two.snippet.y) < 6, 'and the preview follows it on that line');
  assert.ok(two.row.height >= 44 && two.row.height < 64, `two lines high (${two.row.height})`);
  assert.ok((await unread.locator('.mail-preview').innerText()).length > 0);
  const selected = page.locator('.mail-row.selected').first();
  assert.equal(await style(selected, 'background-color'), 'rgb(219, 225, 255)', 'the selected row is the selection tint of Sigrid');
  console.log('PASS A row has two lines, no avatar, a 6px accent dot and an accent time when unread');

  // ---- the checkbox is Sigrid's, per row, and so is the selection bar's ----
  const check = page.locator('.mail-row').nth(1).locator('.mail-checkbox');
  assert.equal(await check.getAttribute('role'), 'checkbox', 'the row checkbox is Sigrid\u2019s');
  await page.locator('.mail-row').nth(1).hover();
  await check.check();
  const toolbar = page.locator('.selection-toolbar');
  await expect(toolbar).toBeVisible();
  assert.equal(await toolbar.locator('[role="checkbox"]').count(), 1, 'one checkbox in the bar: Sigrid\u2019s');
  const all = page.getByRole('checkbox', { name: 'Select loaded messages' });
  await expect(all).not.toBeChecked();
  assert.equal(await toolbar.locator('[role="checkbox"]').getAttribute('data-state'), 'indeterminate', 'some, not all: mixed');
  await all.check({ force: true });
  assert.equal(await page.locator('.mail-checkbox[data-state="checked"]').count(), await page.locator('.mail-checkbox').count(), 'ticking the bar ticks every row');
  await page.getByRole('button', { name: 'Clear', exact: true }).click();
  await expect(toolbar).toHaveCount(0);
  console.log('PASS The row checkbox and the selection bar use Sigrid\u2019s Checkbox');
  await page.context().close();

  // ---- a conversation: replies on the same edge, the open-conversation ground, the arrows ----
  page = await open({ fixture: withConversation(sample), threads: true });
  const main = page.locator('.mail-row', { hasText: 'Launch plan' });
  const toggle = main.locator('.thread-toggle');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  let size = await toggle.boundingBox();
  assert.ok(size.width >= 24 && size.height >= 24, `the conversation arrow is at least 24x24 (${size.width}x${size.height})`);
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const replies = page.locator('.reply-row');
  await expect(replies).toHaveCount(4);
  assert.equal(await style(replies.first(), 'background-color'), 'rgb(247, 248, 254)', 'the replies of an open conversation have their ground');
  const lefts = await replies.locator('.reply-head strong').evaluateAll((nodes) => nodes.map((n) => Math.round(n.getBoundingClientRect().left)));
  const mainLeft = Math.round((await main.locator('.mail-meta strong').boundingBox()).x);
  assert.deepEqual([...new Set(lefts)], [mainLeft], `replies sit on the same edge as the row (${lefts} vs ${mainLeft})`);
  assert.equal(await replies.locator('.reply-avatar').count(), 0);
  const fold = replies.locator('.reply-fold').first();
  await expect(fold).toHaveAttribute('aria-expanded', 'true');
  size = await fold.boundingBox();
  assert.ok(size.width >= 24 && size.height >= 24, `the fold arrow is at least 24x24 (${size.width}x${size.height})`);
  await fold.click();
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  size = await fold.boundingBox();
  assert.ok(size.width >= 24 && size.height >= 24, 'and stays so when folded');
  await fold.click();
  const reply = await replies.first().boundingBox();
  assert.ok(reply.height < 64, `a reply is lower than a row (${reply.height})`);
  console.log('PASS Replies are aligned rows on the open-conversation ground; both arrows are at least 24x24');
  await page.context().close();

  // ---- Review Focus 1: 80-character sender, 40 recipients, 1000x700 ----
  page = await open({ fixture: crowded(), size: { width: 1000, height: 700 }, threads: true });
  const heavy = page.locator('.mail-row', { hasText: 'Launch plan' });
  await heavy.locator('.thread-toggle').click();
  await expect(page.locator('.reply-row')).toHaveCount(4);
  const measure = () =>
    page.evaluate(() => {
      const scroll = document.querySelector('.mail-scroll');
      const bounds = scroll.getBoundingClientRect();
      return {
        overflow: scroll.scrollWidth - scroll.clientWidth,
        rows: [...document.querySelectorAll('.mail-row, .reply-row')].map((row) => {
          const box = row.getBoundingClientRect();
          const parts = [...row.querySelectorAll('strong, time, .reply-to, .thread-toggle, .reply-fold, .mail-subject, .mail-snippet, .reply-snippet')];
          const right = Math.max(...parts.map((part) => part.getBoundingClientRect().right));
          return { height: box.height, outside: right - box.right, right: box.right, bounds: bounds.right, text: row.innerText.replace(/\s+/g, ' ').slice(0, 40) };
        }),
      };
    });
  const crowdedRows = await measure();
  for (const row of crowdedRows.rows) {
    assert.ok(row.height < 64, `every row is lower than 64px (${row.height}: ${row.text})`);
    assert.ok(row.outside <= 0.5, `no column goes past its row (${row.outside}: ${row.text})`);
  }
  assert.ok(crowdedRows.overflow <= 0, `no horizontal scroll (${crowdedRows.overflow})`);
  assert.ok(crowdedRows.rows.length >= 5, 'the heavy rows and the others were measured');
  const more = page.locator('.reply-row', { hasText: 'Yes. Adding' }).locator('.to-more');
  await expect(more).toHaveText('+39');
  console.log('PASS 80-character senders and 40 recipients keep every row under 64px with no overflow');
  await page.context().close();

  // ---- SearchBar: no native select, checkbox or date input; Select and DateField instead ----
  page = await open();
  const search = page.getByRole('textbox', { name: 'Search mail', exact: true });
  await search.fill('Dinner');
  await search.press('Enter');
  await expect(page.locator('.search-progress')).toBeHidden();
  const bar = page.locator('.search-workbench');
  const natives = () => bar.locator('select, input[type=checkbox], input[type=date], input[type=time], input[type=datetime-local]').count();
  assert.equal(await natives(), 0, 'closed: none');
  const order = bar.getByRole('combobox', { name: 'Sort results' });
  await expect(order).toBeVisible();
  assert.equal(await order.evaluate((node) => node.tagName), 'BUTTON', 'the sort is a Select, not a <select>');
  await order.click();
  await page.getByRole('option', { name: 'Newest first' }).click();
  await expect(order).toContainText('Newest first');
  for (const name of ['From', 'To', 'When', 'Attachments', 'More filters']) {
    await bar.getByRole('button', { name: new RegExp(`^${name}`) }).click();
    assert.equal(await natives(), 0, `${name}: no native select, checkbox or date input`);
  }
  // When: the mode is a Select; dates are DateFields.
  await bar.getByRole('button', { name: /^When/ }).click();
  const panel = page.locator('.search-filter-panel');
  if (!(await panel.isVisible())) await bar.getByRole('button', { name: /^When/ }).click();
  const mode = panel.getByRole('combobox', { name: 'What do you remember about the date?' });
  await mode.click();
  await page.getByRole('option', { name: 'Date range' }).click();
  await expect(mode).toContainText('Date range');
  const from = panel.getByRole('textbox', { name: 'From date' });
  const through = panel.getByRole('textbox', { name: 'Through date, inclusive' });
  await from.fill('2026-10-15');
  await from.press('Enter');
  await through.fill('2026-10-20');
  await through.press('Enter');
  await expect(from).toHaveValue('10/15/2026');
  await expect(through).toHaveValue('10/20/2026');
  assert.equal(await natives(), 0, 'still none with the dates open');
  await panel.getByRole('button', { name: 'Apply filter', exact: true }).click();
  await expect(search).toHaveValue(/between:2026-10-15\.\.2026-10-20/);
  // More filters: the value is a Select and Apply waits for a choice.
  await bar.getByRole('button', { name: /^More filters/ }).click();
  await panel.getByRole('combobox', { name: 'Search by' }).click();
  await page.getByRole('option', { name: 'Folder' }).click();
  const apply = panel.getByRole('button', { name: 'Apply filter', exact: true });
  await expect(apply).toBeDisabled();
  await panel.getByRole('combobox', { name: 'Value' }).click();
  await page.getByRole('option', { name: 'Sent' }).click();
  await expect(apply).toBeEnabled();
  console.log('PASS SearchBar filters go through Select and DateField, with no native control');

  // ---- R3: a key typed in a Select of the search bar is not a shortcut of the page ----
  const calls = () => page.evaluate(() => window.__previewCalls.filter((c) => c === 'mail_action' || c === 'queue_mail_action').length);
  // The controls: on the bare page the same keys do act, so the detectors below can see an effect.
  {
    const control = await open();
    await expect(control.locator('.message-body').first()).toBeVisible();
    await control.mouse.click(1200, 600);
    await control.keyboard.press('n');
    await expect.poll(() => control.locator('.composer-panel, .composer-dock').count(), { timeout: 4000 }).toBeGreaterThan(0);
    await control.context().close();
    const archiving = await open();
    await expect(archiving.locator('.message-body').first()).toBeVisible();
    const archived = () => archiving.evaluate(() => window.__previewCalls.filter((c) => c === 'mail_action' || c === 'queue_mail_action').length);
    const was = await archived();
    await archiving.mouse.click(1200, 600);
    await archiving.keyboard.press('e');
    await expect.poll(archived, { timeout: 4000 }).toBeGreaterThan(was);
    await archiving.context().close();
  }
  await search.fill('');
  await search.press('Enter');
  await expect(page.locator('.search-progress')).toBeHidden();
  await page.locator('.mail-open').nth(1).click();
  await expect(page.locator('.message-body').first()).toBeVisible();
  await bar.getByRole('button', { name: /^When/ }).click();
  if (!(await panel.isVisible())) await bar.getByRole('button', { name: /^When/ }).click();
  const trigger = panel.getByRole('combobox', { name: 'What do you remember about the date?' });
  await trigger.focus();
  await expect(trigger).toBeFocused();
  const before = await calls();
  const composers = () => page.locator('.composer-panel, .composer-dock').count();
  const open0 = await composers();
  for (const key of ['e', 'n', 'r']) {
    await page.keyboard.press(key);
    await page.waitForTimeout(400);
    if (await page.getByRole('listbox').count()) await page.keyboard.press('Escape');
    await trigger.focus();
  }
  await page.waitForTimeout(1200);
  assert.equal(await calls(), before, 'E on a Select does not archive');
  assert.equal(await composers(), open0, 'N and R on a Select open no composer');
  console.log('PASS Typing e, n and r in a Select of the search bar triggers no page shortcut');

  // ---- Italian labels in the calendar (R14) ----
  await page.context().close();
  page = await open({ locale: 'it-IT' });
  const cerca = page.getByRole('textbox', { name: 'Cerca email', exact: true });
  await cerca.fill('Dinner');
  await cerca.press('Enter');
  await page.getByRole('button', { name: /^Quando/ }).click();
  const quando = page.locator('.search-filter-panel');
  await quando.getByRole('textbox', { name: 'Data', exact: true }).focus();
  await quando.getByRole('button', { name: 'Apri il calendario' }).click();
  await expect(page.getByRole('button', { name: 'Mese successivo' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Mese precedente' })).toBeVisible();
  console.log('PASS The calendar speaks Italian when the app does');
  await page.context().close();

  assert.deepEqual(errors, [], `Uncaught browser errors: ${errors.join('; ')}`);
} finally {
  await browser.close();
}
