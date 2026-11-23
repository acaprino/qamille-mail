// Task 9 of the Ark Compatto plan: the reader and the attachments, measured in computed values
// (spec 2026-11-08 section 6, "Lettore") and driven through roles and accessible names.
// Sample data only. Run: node scripts/test-browser.mjs compatto-reader
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { box, css, previewUrl } from './support/compatto.mjs';
import { openAllGroups } from './support/groups.mjs';

const root = path.resolve(import.meta.dirname, '..');
const fixture = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));
// The first message the app opens: 40 recipients, six in copy (as recipients.mjs does).
const first = fixture.emails.find((e) => e.subject === 'Q4 proposal: can you review by Thursday?');
first.to = Array.from({ length: 40 }, (_, i) => `person${String(i + 1).padStart(2, '0')}@example.test`);
first.to[4] = 'alex@studio-example.com';
first.cc = Array.from({ length: 6 }, (_, i) => `copy${i + 1}@example.test`);

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];

async function open({ width = 1440, height = 900, theme = 'light' } = {}) {
  const context = await browser.newContext({ locale: 'en-US', viewport: { width, height }, colorScheme: theme });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await page.goto(previewUrl);
  await expect(page.locator('.message-title')).toHaveText(first.subject);
  if (theme === 'dark') await page.evaluate(() => (document.documentElement.dataset.scheme = 'dark'));
  return page;
}
const token = (page, name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }, name);

try {
  const page = await open();
  const toolbar = page.locator('.reader-toolbar');
  const named = (name) => toolbar.getByRole('button', { name, exact: true });

  // (a) Reply, Reply all and Forward carry their words; the four quick actions are icons with a tip.
  for (const name of ['Reply', 'Reply all', 'Forward']) {
    await expect(named(name)).toBeVisible();
    assert.equal((await named(name).innerText()).trim(), name, `${name} shows its text`);
  }
  for (const name of ['Archive message', 'Move message to trash', 'Snooze until tomorrow', 'Done']) {
    assert.equal((await named(name).innerText()).trim(), '', `${name} is icon only`);
    await expect(named(name).locator('svg')).toBeVisible();
  }
  assert.equal(await toolbar.locator('button[title]').count(), 0, 'no toolbar button uses a title attribute');
  console.log('PASS Text on Reply, Reply all, Forward; icon only on the quick actions; no title');

  // (b) Tips: only the real shortcuts. Archive E, Reply R; the others have none.
  const tip = page.locator('[data-slot="tooltip-content"]');
  const tipOf = async (name) => {
    await named(name).focus();
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Tab');
    await expect(named(name)).toBeFocused();
    await expect(tip).toBeVisible();
    const text = (await tip.innerText()).replace(/\s+/g, ' ').trim();
    const keys = await tip.locator('kbd').count();
    await page.keyboard.press('Escape');
    await expect(tip).toHaveCount(0);
    return { text, keys };
  };
  assert.deepEqual(await tipOf('Archive message'), { text: 'Archive E', keys: 1 });
  assert.deepEqual(await tipOf('Reply'), { text: 'Reply R', keys: 1 });
  for (const [name, text] of [['Move message to trash', 'Move to trash'], ['Snooze until tomorrow', 'Snooze until tomorrow'], ['Done', 'Done']]) {
    assert.deepEqual(await tipOf(name), { text, keys: 0 }, `${name} has a tip and no shortcut`);
  }
  console.log('PASS Tips show E on Archive and R on Reply, nothing on the others');

  // (c) Every button of the bar is 32px high; icon buttons are square.
  const heights = await toolbar
    .locator('button')
    .evaluateAll((nodes) => nodes.map((n) => [n.getAttribute('aria-label') || n.textContent.trim(), n.getBoundingClientRect().height]));
  for (const [name, height] of heights) assert.equal(height, 32, `${name} is 32px high`);
  assert.equal((await box(named('Archive message'))).width, 32);
  console.log('PASS Every toolbar button is 32px high');

  // (d) "1 of N" and the arrows.
  const position = toolbar.locator('.reader-position');
  await expect(position).toHaveText(/^1 of \d+$/);
  const total = Number((await position.innerText()).split(' of ')[1]);
  assert.ok(total > 1);
  await expect(named('Previous message')).toBeDisabled();
  await named('Next message').click();
  await expect(position).toHaveText(`2 of ${total}`);
  await expect(page.locator('.message-title')).not.toHaveText(first.subject);
  await named('Previous message').click();
  await expect(position).toHaveText(`1 of ${total}`);
  await expect(page.locator('.message-title')).toHaveText(first.subject);
  console.log('PASS Position and arrows');

  // (e) The subject, with the account label above it.
  const title = page.locator('.message-title');
  assert.equal(await css(title, 'font-size'), '20px');
  assert.equal(await css(title, 'font-weight'), '600');
  await expect(page.locator('.message-location')).toContainText('Work');
  console.log('PASS Subject 20px, weight 600, account label');

  // (f) The grey From / To / Date box, three names and "and 37 more".
  const header = page.locator('.message-sender');
  assert.equal(await css(header, 'background-color'), 'rgb(246, 247, 249)');
  assert.equal(await css(header, 'border-top-color'), await token(page, '--color-paper-2'));
  assert.equal(await css(header, 'border-top-width'), '1px');
  assert.equal(await css(header, 'border-top-left-radius'), '6px');
  const text = (await header.innerText()).replace(/\s+/g, ' ');
  assert.match(text, /From .*Maria/, 'the From row');
  assert.match(text, /To you, person01@example.test, person02@example.test, and 37 more/, 'three names and the rest counted');
  assert.match(text, /Date /, 'the Date row');
  console.log('PASS From / To / Date box');

  // (g) The body: 14px, line 1.6, at most 760px wide.
  const body = page.locator('article.message-body');
  assert.equal(await css(body, 'font-size'), '14px');
  assert.equal(parseFloat(await css(body, 'line-height')), 22.4);
  assert.equal(await css(body, 'max-width'), '760px');
  console.log('PASS Body 14px, line 1.6, 760px');

  // (h) Attachments are 32px chips and still open the panel.
  const chip = page.locator('.attachments .attachment').first();
  assert.equal((await box(chip)).height, 32);
  assert.equal(await css(chip, 'border-top-left-radius'), '6px');
  await chip.click();
  await expect(page.locator('.attachment-panel')).toBeVisible();
  await page.locator('.attachment-panel').getByRole('button', { name: 'Close attachment' }).click();
  await expect(page.locator('.attachment-panel')).toHaveCount(0);
  console.log('PASS Attachment chips');

  // (i) Reply all opens the composer with the people of the message.
  await named('Reply all').click();
  // Recipients are chips (RecipientsInput): read the whole entry from each chip's title.
  const chipsOf = (label) =>
    page
      .locator('.field-row')
      .filter({ has: page.getByRole('combobox', { name: label, exact: true }) })
      .locator('[data-scope="tags-input"][data-part="item-preview"]');
  await expect(page.getByRole('combobox', { name: 'To', exact: true })).toBeVisible();
  const toChips = await chipsOf('To').evaluateAll((nodes) => nodes.map((n) => n.getAttribute('title')));
  assert.ok(toChips.some((entry) => entry.includes(first.from_addr)), 'the sender is in To');
  assert.ok(toChips.some((entry) => entry.includes('person01@example.test')) && toChips.some((entry) => entry.includes('person40@example.test')), 'the other recipients are in To');
  assert.ok(!toChips.some((entry) => entry.includes('alex@studio-example.com')), 'the reader is not in To');
  assert.equal(toChips.length, 40, 'the sender and the 39 other recipients (the reader removed) are in To');
  await expect(chipsOf('Cc')).toHaveCount(6);
  console.log('PASS Reply all opens the composer with everyone');
  await page.context().close();

  // (j) Down to 850px nothing overflows and the buttons stay inside the pane.
  for (const [width, height] of [[1000, 640], [850, 640]]) {
    const narrow = await open({ width, height });
    const geometry = await narrow.evaluate(() => {
      const bar = document.querySelector('.reader-toolbar');
      const reader = document.querySelector('.reader').getBoundingClientRect();
      const out = [...bar.querySelectorAll('button')]
        .filter((b) => {
          const r = b.getBoundingClientRect();
          return r.right > reader.right + 0.5 || r.left < reader.left - 0.5;
        })
        .map((b) => b.getAttribute('aria-label') || b.textContent.trim());
      const scroller = document.querySelector('.reader-scroll');
      return {
        out,
        barOverflow: bar.scrollWidth - bar.clientWidth,
        page: document.documentElement.scrollWidth - innerWidth,
        scroll: scroller.scrollWidth - scroller.clientWidth,
      };
    });
    assert.deepEqual(geometry.out, [], `${width}: every button is inside the pane`);
    assert.ok(geometry.barOverflow <= 0 && geometry.page <= 0 && geometry.scroll <= 0, `${width}: no sideways scroll ${JSON.stringify(geometry)}`);
    await narrow.context().close();
  }
  console.log('PASS The toolbar fits down to 850px');

  // (k) Dark theme: the box is not the light grey.
  const dark = await open({ theme: 'dark' });
  assert.notEqual(await css(dark.locator('.message-sender'), 'background-color'), 'rgb(246, 247, 249)');
  await dark.context().close();
  console.log('PASS Dark theme box');

  // (l) The From / To / Date box and the summary box span the whole pane, like the remote-images bar.
  for (const [width, height] of [[1440, 900], [1000, 700]]) {
    const wide = await open({ width, height });
    await openAllGroups(wide);
    await wide.getByText('This week: 5 tools for focused work').first().click();
    const bar = wide.locator('.remote-bar');
    await expect(bar).toBeVisible();
    const barBox = await box(bar);
    for (const [name, locator] of [['headers box', wide.locator('.message-sender')], ['summary box', wide.locator('.ai-summary')]]) {
      const b = await box(locator);
      assert.ok(Math.abs(b.x - barBox.x) <= 1, `${width}: ${name} left edge ${b.x} vs bar ${barBox.x}`);
      assert.ok(Math.abs(b.x + b.width - (barBox.x + barBox.width)) <= 1, `${width}: ${name} right edge ${b.x + b.width} vs bar ${barBox.x + barBox.width}`);
      if (width === 1440) assert.ok(b.width > 760, `${width}: ${name} is wider than 760px (${b.width})`);
    }
    const lead = await box(wide.locator('.reader-toolbar button').first());
    assert.ok(Math.abs(lead.x - barBox.x) <= 1, `${width}: the toolbar starts at the same left edge (${lead.x} vs ${barBox.x})`);
    await wide.context().close();
  }
  console.log('PASS Headers and summary boxes span the pane like the remote-images bar');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
