// The Activity page on the browser preview (Playwright; set QAMILLE_PREVIEW_URL, without it the
// steps are skipped): reached from the sidebar panel, grouped by day, filtered, with undone and
// assistant rows told apart. The preview holds seven rows (see `activityRows` in lib/preview.ts).
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import test from 'node:test';

const previewUrl = process.env.QAMILLE_PREVIEW_URL;
const skip = !previewUrl ? { skip: 'set QAMILLE_PREVIEW_URL to run the preview steps' } : {};

async function withPage(run) {
  const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  try {
    await page.goto(previewUrl);
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await run(page);
    assert.deepEqual(errors, [], `No page error: ${errors}`);
  } finally {
    await browser.close();
  }
}

/** Opens the panel from the footer and follows "See all activity". */
async function openPage(page) {
  await page.locator('.activity-toggle').click();
  await page.locator('.activity-panel').getByRole('button', { name: 'See all activity' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Activity' })).toBeVisible();
}

test('preview: the panel leads to the page, which closes the panel and clears the dot', skip, async () =>
  withPage(async (page) => {
    await expect(page.locator('.activity-toggle')).toHaveClass(/has-unseen/);
    await openPage(page);
    await expect(page.locator('.activity-panel')).toHaveCount(0);
    await expect(page.locator('.activity-toggle')).not.toHaveClass(/has-unseen/);
    // The topbar names the page like any other.
    await expect(page.locator('.workspace-name')).toContainText('Activity');
  }));

test('preview: rows are grouped by day, newest first, with the time of each', skip, async () =>
  withPage(async (page) => {
    await openPage(page);
    const feed = page.locator('.activity-page .feed');
    await expect(feed.locator('.feed-day').first()).toHaveText('Today');
    await expect(feed.locator('.feed-day').nth(1)).toHaveText('Yesterday');
    await expect(feed.locator('.feed-row')).toHaveCount(8);
    // Newest first: the last thing done is on top of today.
    await expect(feed.locator('.feed-row').first()).toContainText('Marked as done');
    await expect(feed.locator('.feed-row').first().locator('.feed-time')).toHaveText(/^\d{2}:\d{2}$/);
  }));

test('preview: the Errors filter leaves only the failure', skip, async () =>
  withPage(async (page) => {
    await openPage(page);
    const rows = page.locator('.activity-page .feed-row');
    await expect(rows).toHaveCount(8);
    await page.getByRole('tab', { name: 'Errors' }).click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText('Sending failed');
    await expect(rows.first()).toContainText('Server unreachable');
    await expect(rows.first()).toHaveClass(/tone-error/);
    await page.getByRole('tab', { name: 'Events' }).click();
    await expect(rows).toHaveCount(2);
    await page.getByRole('tab', { name: 'Actions' }).click();
    await expect(rows).toHaveCount(6);
    await page.getByRole('tab', { name: 'All' }).click();
    await expect(rows).toHaveCount(8);
  }));

test('preview: an undone row is struck through and an assistant row carries the spark', skip, async () =>
  withPage(async (page) => {
    await openPage(page);
    const undone = page.locator('.activity-page .feed-row.undone');
    await expect(undone).toHaveCount(1);
    await expect(undone).toContainText('Snoozed');
    const decoration = await undone
      .locator('strong')
      .evaluate((element) => getComputedStyle(element).textDecorationLine);
    assert.match(decoration, /line-through/, 'an undone action keeps its row, struck through');
    const assistant = page.locator('.activity-page .assistant-mark');
    await expect(assistant).toHaveCount(1);
    await expect(assistant).toHaveAccessibleName('Done by the assistant');
    await expect(
      page.locator('.activity-page .feed-row').filter({ has: page.locator('.assistant-mark') }),
    ).toContainText('12 messages');
  }));

test('preview: the live work is on top of the page while it runs', skip, async () =>
  withPage(async (page) => {
    await openPage(page);
    const live = page.locator('.activity-page [data-activity-id]');
    await expect(live).toHaveCount(0);
    await page.evaluate(() =>
      window.__previewAnnounce({ kind: 'index-progress', account_id: 2, percent: 40 }),
    );
    await expect(live).toHaveCount(1, { timeout: 2000 });
    await expect(live).toContainText('Indexing 40%');
    await expect(page.locator('.activity-page .feed-day').first()).toHaveText('In progress');
    await page.evaluate(() =>
      window.__previewAnnounce({ kind: 'index-progress', account_id: 2, percent: 100 }),
    );
    await expect(live).toHaveCount(0, { timeout: 2500 });
  }));

test('preview: the whole feed is not a live region (the durations would be read out every second)', skip, async () =>
  withPage(async (page) => {
    await openPage(page);
    await expect(page.locator('.activity-page [aria-live]')).toHaveCount(0);
  }));

test('preview: a history that cannot be read says so, in the panel and on the page', skip, async () =>
  withPage(async (page) => {
    await page.evaluate(() => {
      window.__previewCommandFail = {
        activity_available: 'boom',
        activity_recent: 'boom',
        activity_page: 'boom',
        activity_unseen_errors: 'boom',
      };
    });
    await page.locator('.activity-toggle').click();
    const panel = page.locator('.activity-panel');
    await expect(panel).toContainText('History is unavailable right now');
    await expect(panel).not.toContainText('Nothing to report');
    await panel.getByRole('button', { name: 'See all activity' }).click();
    const feed = page.locator('.activity-page .feed');
    await expect(feed).toContainText('History is unavailable right now');
    await expect(feed).not.toContainText('Here you will see');
    // It recovers by itself when the backend answers again.
    await page.evaluate(() => {
      window.__previewCommandFail = {};
    });
    await page.getByRole('tab', { name: 'Errors' }).click();
    await expect(feed).not.toContainText('History is unavailable right now');
    await expect(page.locator('.activity-page .feed-row')).toHaveCount(1);
  }));

test('preview: the keyboard reaches the panel, and Escape gives the focus back to the button', skip, async () =>
  withPage(async (page) => {
    const toggle = page.locator('.activity-toggle');
    await toggle.focus();
    await page.keyboard.press('Enter');
    const panel = page.locator('.activity-panel');
    await expect(panel).toBeVisible();
    // The panel takes the focus, so the next Tab lands on its own button, not on the main area.
    await expect(panel).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(panel.getByRole('button', { name: 'See all activity' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    await expect(toggle).toBeFocused();
  }));

test('preview: a bulk action is one row in the panel and on the page', skip, async () =>
  withPage(async (page) => {
    await openPage(page);
    const rows = page.locator('.activity-page .feed-row');
    await expect(rows).toHaveCount(8, { timeout: 3000 });
    const bulk = rows.filter({ hasText: 'Moved to trash' });
    await expect(bulk).toHaveCount(1);
    await expect(bulk).toContainText('4 messages');
  }));
