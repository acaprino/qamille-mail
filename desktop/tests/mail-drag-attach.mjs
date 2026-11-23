// Dragging messages from the list onto an open composer attaches each one as .eml.
import { chromium, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import path from 'node:path';

const port = 28166;
const url = `http://127.0.0.1:${port}`;
const server = spawn(
  process.execPath,
  ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'],
  { cwd: path.resolve(import.meta.dirname, '..'), windowsHide: true, stdio: 'pipe' },
);
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));

async function waitForServer() {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (server.exitCode != null) throw new Error(output);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    if (attempt === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

let browser;
try {
  await waitForServer();
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(url);
  await page.locator('.compose-main').waitFor();
  await page.getByRole('button', { name: 'All accounts', exact: true }).click();
  await page.getByRole('button', { name: 'Emails', exact: true }).click();
  await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();

  await page.getByRole('button', { name: /^New message/ }).click();
  const composer = page.getByRole('tabpanel');
  await composer.waitFor();
  const chips = composer.locator('.compose-attachments > span');
  // Drop on the rich editor itself: it must not swallow the drop as text.
  const editor = composer.locator('[contenteditable="true"]');
  await expect(editor).toHaveCount(1);

  const rows = page.locator('.mail-row');
  const first = rows.nth(0).locator('.mail-open');
  await first.dragTo(editor);
  await expect(chips).toHaveCount(1);
  await expect(chips.first()).toContainText(/^email-\d+-.+\.eml/);
  await expect(page.getByText('1 email attached as .eml')).toBeVisible();
  await expect(composer.locator('.mail-drop-hint')).toHaveCount(0);
  await expect(editor).not.toContainText('email-');
  await expect(editor).not.toContainText('[');
  console.log('PASS one dragged message is attached as .eml');

  await rows.nth(1).locator('.mail-checkbox').check();
  await rows.nth(2).locator('.mail-checkbox').check();
  await rows.nth(2).locator('.mail-open').dragTo(editor);
  await expect(chips).toHaveCount(3);
  await expect(page.getByText('2 emails attached as .eml')).toBeVisible();
  console.log('PASS dragging a checked row attaches the whole selection');

  await first.dragTo(editor);
  await expect(chips).toHaveCount(3);
  console.log('PASS dropping the same message again does not duplicate it');

  // The drag label follows the interface language.
  await rows.nth(1).locator('.mail-checkbox').check();
  const label = await rows.nth(1).locator('.mail-open').evaluate((row) => {
    const transfer = new DataTransfer();
    row.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: transfer }));
    return document.querySelector('.mail-drag-badge')?.textContent ?? null;
  });
  if (label !== '2 emails') throw new Error(`Unexpected drag label: ${label}`);
  console.log('PASS the multi-message drag label is translated');

  if (errors.length) throw new Error(`Page errors: ${errors.join('\n')}`);
} finally {
  await browser?.close();
  server.kill();
}
