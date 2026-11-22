// Many recipients in the reading pane (spec 2026-11-07 §4.4): three names with the reader first
// and the rest counted, a bounded full list, and nothing that widens or lengthens the pane.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';

const port = 28173;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const fixture = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));

// The first message the app opens: 40 recipients, the reader is the fifth, one is a known sender,
// one address is far too long for the pane; six more people are in copy.
const subject = 'Q4 proposal: can you review by Thursday?';
const message = fixture.emails.find((e) => e.subject === subject);
const longAddress = `${'a-very-long-local-part-'.repeat(3)}x@a-very-long-domain-name-for-a-test.example.test`;
const to = Array.from({ length: 40 }, (_, i) => `person${String(i + 1).padStart(2, '0')}@example.test`);
to[1] = 'maria@client-example.com'; // Maria Rossi, a sender in the sample mail
to[4] = 'alex@studio-example.com'; // the reader
to[5] = longAddress;
message.to = to;
message.cc = Array.from({ length: 6 }, (_, i) => `copy${i + 1}@example.test`);
const expectedTo = ['you', 'person01@example.test', 'Maria Rossi'];

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];
const text = async (locator) => (await locator.innerText()).replace(/\s+/g, ' ').trim();

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });

  for (const [width, height] of [[1440, 900], [1000, 640]]) {
    const at = `${width}x${height}`;
    const page = await browser.newPage({ locale: 'en-US', viewport: { width, height } });
    page.on('pageerror', (error) => errors.push(error.message));
    await page.route(/\/src\/lib\/sample\.json/, (route) =>
      route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
    );
    await page.goto(url);
    await expect(page.locator('.message-title')).toHaveText(subject);
    const toLine = page.locator('.recipient-line[data-kind="to"]');
    const ccLine = page.locator('.recipient-line[data-kind="cc"]');

    // Three names, the reader first, then the count. The names are known by the time the pane shows them.
    await expect.poll(() => text(toLine)).toBe(`To ${expectedTo.join(', ')}, and 37 more`);
    assert.equal(await text(ccLine), 'Cc copy1@example.test, copy2@example.test, copy3@example.test, and 3 more');
    console.log(`PASS ${at}: three names, the reader first, and the rest counted`);

    // A very long address does not widen the pane.
    const scroll = page.locator('.reader-scroll');
    assert.ok(await scroll.evaluate((el) => el.scrollWidth <= el.clientWidth), `${at}: no sideways scroll`);
    const body = page.locator('.message-body').first();
    const top = async () => (await body.boundingBox()).y;
    const before = await top();

    // Opening the list shows everyone in a box of bounded height that scrolls by itself.
    await toLine.getByRole('button', { name: 'and 37 more' }).click();
    const list = toLine.locator('.recipient-full');
    await expect(list).toBeVisible();
    const box = await list.boundingBox();
    const limit = await page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--recipients-max')));
    assert.equal(limit, 96);
    assert.ok(box.height <= limit + 1, `${at}: the list is at most ${limit}px (${box.height})`);
    assert.ok(await list.evaluate((el) => el.scrollHeight > el.clientHeight), `${at}: 40 names scroll inside the box`);
    assert.equal(await list.evaluate((el) => el.scrollTop), 0, 'it starts at the first name');
    const everyone = await text(list);
    for (const name of ['you', 'Maria Rossi', 'person40@example.test']) assert.ok(everyone.includes(name), `${name} is in the full list`);
    assert.ok((await top()) - before <= 110, `${at}: the message text moves down at most 110px (${(await top()) - before})`);
    assert.ok(await scroll.evaluate((el) => el.scrollWidth <= el.clientWidth), `${at}: still no sideways scroll`);
    // Focus follows the control, and the page does not jump.
    await expect(toLine.getByRole('button', { name: 'Show less' })).toBeFocused();
    assert.equal(await scroll.evaluate((el) => el.scrollTop), 0, 'the pane did not scroll');

    // Closing puts everything back where it was.
    await toLine.getByRole('button', { name: 'Show less' }).click();
    await expect(list).toHaveCount(0);
    await expect(toLine.getByRole('button', { name: 'and 37 more' })).toBeFocused();
    assert.ok(Math.abs((await top()) - before) <= 1, `${at}: the text returns to its place`);
    console.log(`PASS ${at}: the full list is bounded and closes cleanly`);
    await page.close();
  }

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
