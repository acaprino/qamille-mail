// The old-mail dialog (EX-9) opens by itself when the history brings many old unread messages, so
// it can appear while someone is typing. Its focus starts on «Leave as is», not on the bulk action
// «Mark as read those older than 30 days»: a Space or Enter right after it opens changes nothing
// on Gmail. Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
page.setDefaultTimeout(15000);
const announce = (event) => page.evaluate((detail) => window.__previewAnnounce(detail), event);
const calls = () => page.evaluate(() => window.__previewCalls ?? []);

try {
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^New message/ }).waitFor();

  // Someone is typing in a composer when the event arrives.
  await page.getByRole('button', { name: /^New message/ }).click();
  const body = page.getByRole('textbox', { name: 'Message body' });
  await body.waitFor();
  await body.click();
  await expect(body).toBeFocused();

  await announce({ kind: 'history-unread', account_id: 1, count: 140 });
  const dialog = page.getByRole('dialog', { name: 'Lots of old unread mail' });
  await expect(dialog).toBeVisible();

  // The focus starts on the safe button.
  const leave = dialog.getByRole('button', { name: 'Leave as is', exact: true });
  await expect(leave).toBeFocused();
  const focused = await page.evaluate(() => document.activeElement?.textContent?.trim());
  assert.equal(focused, 'Leave as is');
  console.log('PASS the dialog starts on «Leave as is»');

  // A Space (and an Enter) typed at that moment must not mark anything read.
  assert.ok(!(await calls()).includes('mark_old_unread_read'));
  await page.keyboard.press('Space');
  await expect(dialog).toHaveCount(0);
  assert.ok(!(await calls()).includes('mark_old_unread_read'), 'Space ran the mark-read action');
  console.log('PASS a Space right after it opens leaves the old mail as it is');

  await announce({ kind: 'history-unread', account_id: 1, count: 140 });
  await expect(dialog).toBeVisible();
  await expect(leave).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  assert.ok(!(await calls()).includes('mark_old_unread_read'), 'Enter ran the mark-read action');
  console.log('PASS an Enter right after it opens leaves the old mail as it is');

  // The bulk action is still one click away.
  await announce({ kind: 'history-unread', account_id: 1, count: 140 });
  await dialog.getByRole('button', { name: 'Mark as read those older than 30 days', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  assert.ok((await calls()).includes('mark_old_unread_read'));
  console.log('PASS the bulk action still runs when it is chosen');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
