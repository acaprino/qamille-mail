// The history of a Gmail account (copertura della posta, wave 3) as the browser preview shows it:
// one row in the single activity indicator while older mail comes in, the coverage per account in
// Settings → Search with «Include Spam and Trash», and the unread notice asked once with its two
// choices. Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
page.setDefaultTimeout(15000);
await page.addInitScript(() => {
  window.__previewHistory = { state: 'running', total: 48300 };
});
const announce = (event) => page.evaluate((detail) => window.__previewAnnounce(detail), event);

try {
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  const accountId = 1;

  await announce({ kind: 'history-progress', account_id: accountId, fetched: 1200, total: 48300, complete: false });
  await expect(page.getByText(/Bringing in older mail · 1,200 of 48,300/).first()).toBeVisible();
  await announce({ kind: 'history-progress', account_id: accountId, fetched: 48300, total: 48300, complete: true });
  await expect(page.getByText(/Bringing in older mail/)).toHaveCount(0);
  console.log('PASS the indicator shows the history while it runs and drops it when complete');

  await announce({ kind: 'history-unread', account_id: accountId, count: 140 });
  const dialog = page.getByRole('dialog', { name: 'Lots of old unread mail' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('140 unread messages');
  await dialog.getByRole('button', { name: 'Leave as is', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await announce({ kind: 'history-unread', account_id: accountId, count: 140 });
  await dialog.getByRole('button', { name: 'Mark as read those older than 30 days', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText(/messages marked as read/)).toBeVisible();
  assert.ok(await page.evaluate(() => window.__previewCalls.includes('mark_old_unread_read')));
  console.log('PASS the unread notice offers its two choices and marks the old ones read');

  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.locator('.settings-nav').getByRole('button', { name: 'Search', exact: true }).click();
  const coverage = page.getByTestId('history-coverage').first();
  await expect(coverage).toContainText(/Listed: \d+ of 48300 · Indexed by content: 0/);
  await expect(page.getByText('Bringing in older mail', { exact: true }).first()).toBeVisible();
  const toggle = page.locator('[data-scope="switch"][data-part="root"]', { hasText: 'Include Spam and Trash' }).first();
  await expect(toggle).toHaveAttribute('data-state', 'unchecked');
  await toggle.click();
  await expect(toggle).toHaveAttribute('data-state', 'checked');
  assert.ok(await page.evaluate(() => window.__previewCalls.includes('set_include_spam_trash')));
  console.log('PASS Settings → Search shows the coverage per account and «Include Spam and Trash»');
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
