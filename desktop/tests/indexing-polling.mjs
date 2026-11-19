import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
// Browser preview only: there is no Tauri event bus, so Settings > Search relies on its 30 s safety
// poll instead of the old 2.5 s loop (INV-005, INV-009). The same 30 s poll stays on desktop for
// transitions the backend does not announce; the `indexing-changed` listener itself is
// desktop-only and is exercised by the native build.
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
page.setDefaultTimeout(15000);
// A fake clock that still runs in real time, so the 30 s period can be jumped over.
await page.clock.install();
await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
await page.getByRole('button', { name: /^New message/ }).waitFor();
try {
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.locator('.settings-nav').getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Full mail index' })).toBeVisible();
  const statusCalls = () =>
    page.evaluate(() => window.__previewCalls.filter((c) => c === 'indexing_status').length);
  const opened = await statusCalls();
  assert.ok(opened >= 1, 'The section loads its status once when it opens');
  // The old loop would have called four times in this window.
  await page.waitForTimeout(8000);
  assert.equal(await statusCalls(), opened, 'No polling inside the 30 s fallback period');
  console.log('PASS Indexing settings do not poll every few seconds');
  await page.clock.fastForward(31000);
  await expect.poll(statusCalls).toBe(opened + 1);
  await page.waitForTimeout(500);
  assert.equal(await statusCalls(), opened + 1, 'The safety poll runs once per 30 s period');
  console.log('PASS Indexing settings keep a 30 s safety poll');
  // Without a server the account says so (not «paused», not «waiting for the model»).
  await expect(page.getByText('Semantic search: a server is needed', { exact: true }).first()).toBeVisible();
  assert.equal(await page.getByText('Waiting for the search model').count(), 0);
  assert.equal(await page.getByRole('region', { name: 'Search model' }).count(), 0, 'no model section, no automatic download');
  console.log('PASS the account state needs_server reads «Semantic search: a server is needed»');
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
