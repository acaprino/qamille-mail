// Wave 2 (E10): the one-time notice about the old local compute mode. It appears once, «Free space»
// asks for a confirmation in the page before anything is deleted, «I understand» hides it for good.
// Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
async function open(legacy) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  // A native `confirm()` would be a second, wrong confirmation: it must never be called.
  await page.addInitScript((value) => {
    window.__previewLegacy = value;
    window.__confirmCalls = 0;
    window.confirm = () => {
      window.__confirmCalls++;
      return true;
    };
  }, legacy);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  return { page, context };
}
const notice = (page) => page.getByRole('status').filter({ hasText: 'The compute on your PC has changed' });
const freed = (page) => page.evaluate(() => window.__legacyFreed ?? null);

try {
  {
    const { page, context } = await open({ models_bytes: 3_400_000_000, had_server: true });
    await expect(notice(page)).toBeVisible();
    await expect(notice(page)).toContainText('Semantic search and transcription now require a server. You can start it on your PC with Docker.');
    await expect(notice(page).getByRole('button', { name: 'Open the guide', exact: true })).toBeVisible();
    // Free space: a confirmation in the page, nothing deleted until it is accepted.
    await notice(page).getByRole('button', { name: 'Free space (3.4 GB)', exact: true }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    assert.equal(await freed(page), null, 'nothing is deleted before the confirmation');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    assert.equal(await freed(page), null, 'cancel deletes nothing');
    await notice(page).getByRole('button', { name: 'Free space (3.4 GB)', exact: true }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Free space', exact: true }).click();
    await expect.poll(() => freed(page)).toBe(true);
    await expect(notice(page).getByRole('button', { name: /^Free space/ })).toHaveCount(0);
    assert.equal(await page.evaluate(() => window.__confirmCalls), 0, 'never a browser confirm()');
    console.log('PASS the notice appears once; «Free space» asks for a confirmation in the page before anything is deleted');
    await notice(page).getByRole('button', { name: 'I understand', exact: true }).click();
    await expect(notice(page)).toHaveCount(0);
    const status = await page.evaluate(async () => (await import('/src/lib/api.ts')).call('legacy_local_status'));
    assert.equal(status.show, false);
    console.log('PASS «I understand» hides it for good');
    await context.close();
  }
  {
    const { page, context } = await open({ models_bytes: 0, had_server: false });
    await expect(page.getByRole('button', { name: /^New message/ })).toBeVisible();
    assert.equal(await notice(page).count(), 0, 'a fresh data folder shows nothing');
    console.log('PASS a fresh data folder shows no notice');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
