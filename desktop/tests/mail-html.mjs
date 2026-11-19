// HTML mail renders sanitized in an isolated frame, with remote images blocked until allowed.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { openAllGroups } from './support/groups.mjs';

const port = 28163;
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
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 1000 }, colorScheme: 'dark' });
  const errors = [];
  const remoteRequests = [];
  page.on('pageerror', error => errors.push(error.message));
  // A request reaches the network only when the route sees it; CSP-blocked loads never do.
  await page.route('https://images.example.com/**', route => {
    remoteRequests.push(route.request().url());
    return route.fulfill({ status: 404, body: '' });
  });
  if (process.env.DEBUG_HTML) page.on('requestfailed', r => console.log('FAILED', r.url(), r.failure()?.errorText));
  await page.addInitScript(() => {
    localStorage.setItem('qamille:mail-display', 'threads');
    window.openedLinks = [];
  });
  await page.route(/\/src\/lib\/preview\.ts(?:\?|$)/, async route => {
    const response = await route.fetch();
    const source = await response.text();
    assert.match(source, /export async function previewCommand\(/);
    await route.fulfill({ response, body: source.replace('export async function previewCommand(', 'async function originalPreviewCommand(') + `
      export async function previewCommand(command, args = {}) {
        if (command === 'open_link') { window.openedLinks.push(args.url); return; }
        return originalPreviewCommand(command, args);
      }
    ` });
  });
  await page.goto(url);
  await expect(page.locator('.message-body')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  // A plain-text message keeps the text reader.
  await expect(page.locator('article.message-body')).toBeVisible();

  // The newsletter lives in a quiet group, which starts closed.
  await openAllGroups(page);
  await page.getByText('This week: 5 tools for focused work').first().click();
  const frame = page.frameLocator('.html-body iframe');
  await expect(frame.locator('h1')).toHaveText('5 tools for focused work');
  await expect(page.locator('article.message-body')).toHaveCount(0);

  const inside = await page.locator('.html-body iframe').evaluate(element => {
    const doc = element.contentDocument;
    return {
      sandbox: element.getAttribute('sandbox'),
      scripts: doc.querySelectorAll('script').length,
      ran: doc.body.dataset.ran ?? null,
      policy: doc.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute('content'),
      styled: getComputedStyle(doc.querySelector('.card')).borderRadius,
      height: element.getBoundingClientRect().height,
      content: doc.documentElement.scrollHeight,
    };
  });
  assert.equal(inside.sandbox, 'allow-same-origin', 'Scripts are never allowed in the frame');
  assert.equal(inside.scripts, 0, 'Sanitizing removes scripts');
  assert.equal(inside.ran, null, 'No script ran');
  assert.match(inside.policy, /img-src data: cid:;/, 'Remote images are blocked by default');
  assert.equal(inside.styled, '12px', 'The sender stylesheet applies');
  assert.ok(Math.abs(inside.height - inside.content) <= 2, `Frame fits its content: ${JSON.stringify(inside)}`);
  assert.deepEqual(remoteRequests, [], 'No remote image is requested before consent');
  const darkEmail = await page.locator('.html-body iframe').evaluate(element => {
    const doc = element.contentDocument;
    return {
      theme: doc.documentElement.dataset.theme,
      filter: getComputedStyle(doc.documentElement).filter,
      imageFilter: getComputedStyle(doc.querySelector('img')).filter,
    };
  });
  assert.equal(darkEmail.theme, 'dark', 'The email follows the app dark theme');
  assert.notEqual(darkEmail.filter, 'none', 'Dark mode transforms email colors');
  assert.notEqual(darkEmail.imageFilter, 'none', 'Images are counter-transformed and keep their colors');

  await page.emulateMedia({ colorScheme: 'light' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(frame.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Toggle dark appearance' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(frame.locator('html')).toHaveAttribute('data-theme', 'dark');
  assert.equal(await page.evaluate(() => localStorage.getItem('qamille:appearance')), 'dark');

  await page.getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.getByRole('combobox', { name: 'Appearance', exact: true }).click();
  await page.getByRole('option', { name: 'Automatic (system)', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  await page.getByRole('button', { name: 'Inbox', exact: false }).first().click();
  await page.getByText('This week: 5 tools for focused work').first().click();
  await frame.getByRole('link', { name: 'Read the issue' }).click();
  await expect.poll(() => page.evaluate(() => window.openedLinks)).toEqual(['https://focus.example.com/issue']);
  await expect(frame.locator('h1')).toBeVisible();

  // Keys pressed in the message body reach the app's shortcuts; the reading keys stay in the frame.
  await frame.locator('h1').click();
  await page.keyboard.press('ArrowDown');
  await expect(frame.locator('h1')).toHaveText('5 tools for focused work');
  await page.keyboard.press('Control+k');
  await expect(page.locator('.global-search input')).toBeFocused();

  await page.getByRole('button', { name: 'Show images' }).click();
  await expect(page.getByRole('button', { name: 'Show images' })).toHaveCount(0);
  await expect.poll(() => remoteRequests.length).toBeGreaterThan(0);
  const allowed = await page.locator('.html-body iframe').evaluate(element =>
    element.contentDocument.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute('content'));
  assert.match(allowed, /img-src data: cid: https: http:/);

  // E pressed in the message body archives it, as anywhere else outside a field.
  await frame.locator('h1').click();
  const archived = () => page.evaluate(() => (window.__previewCalls ?? []).filter((c) => c === 'mail_action' || c === 'queue_mail_action').length);
  const before = await archived();
  await page.keyboard.press('e');
  await expect.poll(archived).toBeGreaterThan(before);

  assert.deepEqual(errors, []);
  console.log('mail-html: 29 checks passed');
} finally {
  await browser?.close();
  server.kill();
}
