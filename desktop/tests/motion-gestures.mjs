// Movement answers a gesture (Sigrid's motion, src/ui/motion.tsx): the panel under the search field comes in from the side of
// the field, another message settles into the reader, the arrow of a group turns when it opens or closes. With the system
// asking for less motion nothing moves and the arrow is already turned. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { spawn } from 'node:child_process';
import path from 'node:path';

const port = 28181;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];

async function open(reducedMotion) {
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 }, reducedMotion });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(url);
  await expect(page.locator('.mail-row').first()).toBeVisible();
  // The first moments are loading: nothing moves then (INV-002 of Sigrid).
  await page.waitForTimeout(2000);
  return page;
}
// Animations running on the first element matching `selector`, right after `act`: what they start from.
async function started(page, selector, act) {
  await act();
  return page.evaluate((sel) => {
    const el = document.querySelector(sel);
    return el ? el.getAnimations().flatMap((a) => a.effect.getKeyframes().map((k) => k.opacity ?? k.translate ?? k.transform ?? '')).filter(Boolean) : null;
  }, selector);
}

try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true });

  let page = await open('no-preference');
  const search = () => page.locator('.global-search input').first().click();
  const panel = await started(page, '#search-filters-popover', search);
  assert.ok(panel?.includes('0'), `the search panel fades in (${panel})`);
  console.log('PASS The panel under the search field comes in as it opens');
  await page.keyboard.press('Escape');

  const reader = await started(page, '.reader-scroll', () => page.locator('.mail-row').nth(2).click());
  assert.ok(reader?.length, `another message settles into the reader (${reader})`);
  console.log('PASS Another message settles into the reader');

  const arrow = page.locator('.group-heading').first().locator('span.inline-flex').first();
  const heading = page.locator('.group-heading').first();
  const wasOpen = await heading.getAttribute('aria-expanded');
  const turning = await started(page, '.group-heading span.inline-flex', () => heading.click());
  assert.ok(turning?.length, `the arrow of a group turns when it closes or opens (${turning})`);
  await page.waitForTimeout(500);
  const turned = await arrow.evaluate((el) => getComputedStyle(el).transform);
  assert.equal(turned === 'none', wasOpen === 'true', `closed it points right, open it points down (was ${wasOpen}, now ${turned})`);
  console.log('PASS The arrow of a group turns a quarter');
  await page.context().close();

  // Less motion: no animation at all, and the arrow is already where it goes.
  page = await open('reduce');
  const still = await started(page, '#search-filters-popover', () => page.locator('.global-search input').first().click());
  assert.deepEqual(still, [], `the search panel does not move (${still})`);
  await page.keyboard.press('Escape');
  const stillReader = await started(page, '.reader-scroll', () => page.locator('.mail-row').nth(2).click());
  assert.deepEqual(stillReader, [], `the reader does not move (${stillReader})`);
  const group = page.locator('.group-heading').first();
  const before = await group.getAttribute('aria-expanded');
  await group.click();
  await expect(group).toHaveAttribute('aria-expanded', before === 'true' ? 'false' : 'true');
  const rotate = await group.locator('span.inline-flex').first().evaluate((el) => getComputedStyle(el).rotate);
  assert.equal(rotate !== 'none', before !== 'true', `without animation the arrow is turned at once (rotate ${rotate})`);
  console.log('PASS With less motion nothing moves and the arrow is already turned');
  await page.context().close();

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
