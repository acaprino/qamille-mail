// Dark theme with the recipient list open (spec 2026-11-07 §4.4, §8: "not verified"), at 1440 and
// 1000 wide: the contrast of the label, the names and "Show less" against their real background, and
// of a hovered and a selected reply row. Extends the checks of recipients.mjs. Synthetic data only.
// Port 28179. Screenshots in checks/recipients-dark/ (look at them: numbers can hide a defect).
//   node tests/recipients-dark.mjs        (from desktop/)
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { IDS, SUBJECT, withConversation } from './support/conversation-fixture.mjs';

const port = 28179;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const out = path.resolve(root, '../checks/recipients-dark');
await fs.mkdir(out, { recursive: true });
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));
const fixture = withConversation(sample);
for (const email of fixture.emails) email.reminder_at = null;

// The first message the app opens: 40 recipients (the reader is the fifth, one address is far too long), six in copy.
const subject = 'Q4 proposal: can you review by Thursday?';
const message = fixture.emails.find((e) => e.subject === subject);
const longAddress = `${'a-very-long-local-part-'.repeat(3)}x@a-very-long-domain-name-for-a-test.example.test`;
const to = Array.from({ length: 40 }, (_, i) => `person${String(i + 1).padStart(2, '0')}@example.test`);
to[1] = 'maria@client-example.com';
to[4] = 'alex@studio-example.com';
to[5] = longAddress;
message.to = to;
message.cc = Array.from({ length: 6 }, (_, i) => `copy${i + 1}@example.test`);

/** Runs in the page, on the element: its text colour over what is really painted behind it, as a WCAG ratio. */
function contrast(el) {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  const rgba = (css) => {
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = '#000';
    ctx.fillStyle = css;
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    return [d[0], d[1], d[2], d[3] / 255];
  };
  const over = (top, under) => [0, 1, 2].map((i) => top[i] * top[3] + under[i] * (1 - top[3])).concat([1]);
  // The painted background: the backgrounds of the element and its ancestors, composited from the page down.
  const chain = [];
  let opacity = 1;
  for (let node = el; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    chain.unshift(rgba(style.backgroundColor));
    opacity *= parseFloat(style.opacity);
  }
  const scheme = getComputedStyle(document.documentElement).colorScheme;
  let background = scheme.includes('dark') ? [18, 18, 18, 1] : [255, 255, 255, 1];
  for (const layer of chain) background = over(layer, background);
  const color = rgba(getComputedStyle(el).color);
  color[3] *= opacity;
  const text = over(color, background);
  const lum = ([r, g, b]) => {
    const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const [hi, lo] = [lum(text), lum(background)].sort((a, b) => b - a);
  const hex = (c) => `#${c.slice(0, 3).map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`;
  return { ratio: (hi + 0.05) / (lo + 0.05), fg: hex(text), bg: hex(background), size: getComputedStyle(el).fontSize, weight: getComputedStyle(el).fontWeight };
}

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];
const report = [];

async function ratio(locator, label, at, minimum = 4.5) {
  const r = await locator.first().evaluate(contrast);
  report.push(`${at}  ${label.padEnd(44)} ${r.ratio.toFixed(2)}:1  ${r.fg} on ${r.bg}  ${r.size}/${r.weight}`);
  assert.ok(r.ratio >= minimum, `${at}: ${label} is ${r.ratio.toFixed(2)}:1 (${r.fg} on ${r.bg}), needs ${minimum}:1`);
  return r;
}

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true });

  for (const [width, height] of [[1440, 900], [1000, 640]]) {
    const at = `${width}`;
    const page = await browser.newPage({ locale: 'en-US', viewport: { width, height } });
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript(() => {
      localStorage.setItem('qamille:appearance', 'dark');
      localStorage.setItem('qamille:mail-display', 'threads');
    });
    await page.route(/\/src\/lib\/sample\.json/, (route) =>
      route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
    );
    await page.goto(url);
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await expect(page.locator('.message-title')).toHaveText(subject);
    await page.evaluate(() => document.fonts.ready);
    await page.mouse.move(0, 0);

    // ---- the recipient list, closed then open ----
    const toLine = page.locator('.recipient-line[data-kind="to"]');
    await expect.poll(async () => (await toLine.innerText()).replace(/\s+/g, ' ')).toMatch(/and 37 more$/);
    await ratio(toLine.locator('.recipient-label'), 'label "To" (closed)', at);
    await ratio(toLine.locator('.recipient-names'), 'names (closed)', at);
    await ratio(toLine.locator('.recipient-toggle'), '"and 37 more"', at);
    await toLine.getByRole('button', { name: 'and 37 more' }).click();
    const list = toLine.locator('.recipient-full');
    await expect(list).toBeVisible();
    await ratio(toLine.locator('.recipient-label'), 'label "To" (open)', at);
    await ratio(list, 'names in the full list', at);
    await ratio(toLine.getByRole('button', { name: 'Show less' }), '"Show less"', at);
    const ccLine = page.locator('.recipient-line[data-kind="cc"]');
    await ratio(ccLine.locator('.recipient-label'), 'label "Cc"', at);
    await ratio(ccLine.locator('.recipient-names'), 'names in Cc', at);
    // The box itself must read as a box: its edge against the pane, a non-text 3:1 would be ideal, so it is reported.
    const edge = await list.evaluate((el) => { const s = getComputedStyle(el); return `${s.borderTopWidth} ${s.borderTopStyle} ${s.borderTopColor}, bg ${s.backgroundColor}`; });
    report.push(`${at}  the full list box: ${edge}`);
    await expect(toLine.getByRole('button', { name: 'Show less' })).toBeFocused();
    await page.screenshot({ path: path.join(out, `dark-${width}-list-open.png`) });
    assert.ok(await page.locator('.reader-scroll').evaluate((el) => el.scrollWidth <= el.clientWidth), `${at}: no sideways scroll`);
    await toLine.getByRole('button', { name: 'Show less' }).click();
    await expect(list).toHaveCount(0);
    console.log(`PASS ${at}: dark theme, recipient list closed and open: every text is at least 4.5:1`);

    // ---- the reply rows: resting, unread, hovered, selected ----
    const row = page.locator('.mail-row', { hasText: SUBJECT });
    await row.locator('.thread-toggle').click();
    await expect(page.locator('.reply-row')).toHaveCount(4);
    const reply = (id) => page.locator(`.reply-row[data-reply-id="${id}"]`);
    const parts = async (id, state) => {
      const r = reply(id);
      await ratio(r.locator('.reply-head strong'), `reply ${state}: name`, at);
      await ratio(r.locator('.to-label'), `reply ${state}: "to"`, at);
      await ratio(r.locator('.to-first'), `reply ${state}: the person answered`, at);
      await ratio(r.locator('time'), `reply ${state}: date`, at);
      await ratio(r.locator('.reply-snippet'), `reply ${state}: first lines`, at);
      if (await r.locator('.to-more').count()) await ratio(r.locator('.to-more'), `reply ${state}: "+N"`, at);
    };
    await page.mouse.move(0, 0);
    await parts(IDS.root, 'at rest');
    await parts(IDS.many, 'unread');
    await ratio(reply(IDS.a).locator('.fold-count, .reply-fold svg').first(), 'fold chevron (open)', at, 3);
    // hovered: the pointer over a reply that is not the open one
    await reply(IDS.sent).locator('.reply-open').hover();
    await parts(IDS.sent, 'hovered');
    const listBox = await page.locator('.mail-list').boundingBox();
    await page.screenshot({ path: path.join(out, `dark-${width}-reply-hovered.png`), clip: { x: listBox.x, y: listBox.y + 200, width: listBox.width, height: 330 } });
    // selected: open reply A, move the pointer away
    await reply(IDS.a).locator('.reply-open').click();
    await expect(reply(IDS.a)).toHaveClass(/selected/);
    await page.mouse.move(0, 0);
    await page.waitForTimeout(250);
    await parts(IDS.a, 'selected');
    // hovered and selected at once
    await reply(IDS.a).locator('.reply-open').hover();
    await page.waitForTimeout(250);
    await parts(IDS.a, 'selected + hovered');
    // the unread dot and the blue fold count are not text: 3:1
    await reply(IDS.a).locator('.reply-fold').click();
    await ratio(reply(IDS.a).locator('.fold-count'), 'folded count, blue (an unread one is hidden)', at);
    await page.mouse.move(0, 0);
    await page.screenshot({ path: path.join(out, `dark-${width}-replies.png`) });
    await reply(IDS.a).locator('.reply-fold').click();
    console.log(`PASS ${at}: dark theme, reply rows at rest, unread, hovered, selected and both: every text is at least 4.5:1`);
    await page.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await fs.writeFile(path.join(out, 'contrast.txt'), report.join('\n')).catch(() => {});
  await browser?.close();
  server.kill();
}
console.log(`Contrast table: ${path.join(out, 'contrast.txt')}\nScreenshots: ${out}`);
