import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { openAllGroups } from './support/groups.mjs';
const native = process.env.QAMILLE_CDP;
const browser = native
  ? await chromium.connectOverCDP(native)
  : await chromium.launch({ channel: 'msedge', headless: true });
const context = native
  ? browser.contexts()[0]
  : await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
const page = native ? context.pages()[0] : await context.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(e.message));
if (!native) await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
// The label depends on the system language until the native branch sets en-US.
await page.locator('.compose-main').waitFor();
if (native) {
  const kinds = await page.evaluate(async () => {
    const workspace = await window.__TAURI_INTERNALS__.invoke('workspace_snapshot');
    return workspace.accounts.map((account) => account.kind);
  });
  assert.ok(
    kinds.length > 0 && kinds.every((kind) => kind === 'Sample'),
    'Native tests require sample accounts only. No test actions were performed.',
  );
  await page.evaluate(async () => {
    const workspace = await window.__TAURI_INTERNALS__.invoke('workspace_snapshot');
    await window.__TAURI_INTERNALS__.invoke('save_settings', {
      update: { settings: { ...workspace.settings, locale: 'en-US' }, ai_key: null, google_client_secret: null },
    });
  });
  await page.reload();
  await page.getByRole('button', { name: /^New message/ }).waitFor();
}
// Native windows retain navigation preferences between runs.
await page.getByRole('button', { name: 'All accounts', exact: true }).click();
await page.getByRole('button', { name: 'Emails', exact: true }).click();
await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
// The quiet groups start closed, and the message opened at startup moves into one of them.
await openAllGroups(page);
const results = [];
async function step(name, work) {
  await work();
  results.push(name);
  console.log(`PASS ${name}`);
}
try {
  await step('Inbox and reading pane load', async () => {
    await page.getByRole('heading', { name: 'Inbox', exact: true }).waitFor();
    await page
      .locator('.mail-open', { hasText: 'Q4 proposal: can you review by Thursday?' })
      .click();
    await page.getByRole('heading', { name: 'Q4 proposal: can you review by Thursday?' }).waitFor();
  });
  await step('Attachment preview', async () => {
    await page.getByRole('button', { name: /Q4-proposal.txt.*Preview/ }).click();
    await page.getByRole('dialog', { name: 'Q4-proposal.txt' }).waitFor();
    await page.getByRole('button', { name: 'Close attachment', exact: true }).click();
    await page.getByRole('dialog', { name: 'Q4-proposal.txt' }).waitFor({ state: 'detached' });
  });
  await step('Create, save, close and reopen a formatted draft', async () => {
    await page.getByRole('button', { name: /^New message/ }).click();
    await page.getByRole('tabpanel').waitFor();
    await page.getByRole('combobox', { name: 'To', exact: true }).fill('review@example.com');
    await page
      .getByRole('textbox', { name: 'Subject', exact: true })
      .fill('Tauri draft persistence check');
    await page
      .getByRole('textbox', { name: 'Message body' })
      .fill('Draft saved through the new editor.');
    await page.getByRole('textbox', { name: 'Message body' }).press('Control+a');
    await page.getByRole('button', { name: 'Bold', exact: true }).click();
    await page.getByRole('button', { name: 'Save and close draft' }).click();
    await page.getByRole('button', { name: /^Drafts\s*\d*$/ }).click();
    await page
      .getByRole('button', { name: /Tauri draft persistence check/ })
      .first()
      .click();
    assert.equal(
      await page.getByRole('textbox', { name: 'Subject', exact: true }).inputValue(),
      'Tauri draft persistence check',
    );
    assert.match(
      await page.getByRole('textbox', { name: 'Message body' }).innerHTML(),
      /<strong>Draft saved through the new editor\.<\/strong>/,
    );
    await page.getByRole('button', { name: 'Save and close draft' }).click();
  });
  await step('Search narrows messages', async () => {
    await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
    await page.getByRole('textbox', { name: 'Search mail' }).fill('Dinner');
    await page.locator('.mail-open', { hasText: 'Dinner on Saturday?' }).waitFor();
    await page.waitForFunction(() => document.querySelectorAll('.mail-open').length === 1);
    await page.getByRole('textbox', { name: 'Search mail' }).fill('');
  });
  await step('Settings and dark appearance', async () => {
    await page.getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.getByRole('heading', { name: 'Settings', exact: true }).waitFor();
    await page.getByRole('combobox', { name: 'Appearance', exact: true }).click();
    await page.getByRole('option', { name: 'Dark', exact: true }).click();
    await page.waitForFunction(() => document.documentElement.dataset.scheme === 'dark');
    await page.screenshot({ path: '../checks/tauri-dark.png' });
    await page.getByRole('combobox', { name: 'Appearance', exact: true }).click();
    await page.getByRole('option', { name: 'Automatic (system)', exact: true }).click();
    await page.emulateMedia({ colorScheme: 'light' });
    await page.waitForFunction(() => document.documentElement.dataset.scheme === 'light');
  });
  await step('Calendar event persists', async () => {
    await page.getByRole('button', { name: 'Calendar', exact: true }).click();
    await page.getByRole('button', { name: 'New event', exact: true }).click();
    await page.getByRole('textbox', { name: 'Title', exact: true }).fill('Tauri calendar check');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await page.getByRole('button', { name: /Tauri calendar check/ }).waitFor();
  });
  if (native)
    await step('Sample send completes via Rust without SMTP', async () => {
      await page.getByRole('button', { name: /^Drafts\s*\d*$/ }).click();
      await page
        .getByRole('button', { name: /Tauri draft persistence check/ })
        .first()
        .click();
      await page.getByRole('button', { name: 'Send', exact: true }).click();
      await page.getByRole('tabpanel').waitFor({ state: 'hidden' });
      await page.getByRole('button', { name: /^Sent\s*\d*$/ }).click();
      await page.locator('.mail-open', { hasText: 'Tauri draft persistence check' }).waitFor();
    });
  await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
  await page.locator('.mail-open', { hasText: 'Q4 proposal: can you review by Thursday?' }).click();
  await page.getByRole('heading', { name: 'Q4 proposal: can you review by Thursday?' }).waitFor();
  await page.screenshot({
    path: native ? '../checks/tauri-native.png' : '../checks/tauri-inbox.png',
  });
  assert.deepEqual(errors, [], `Uncaught browser errors: ${errors.join('; ')}`);
  if (native && process.env.QAMILLE_TEST_DATA_DIR) {
    await step('Closing the desktop flushes the latest draft edit', async () => {
      await page.getByRole('button', { name: /^New message/ }).click();
      await page
        .getByRole('textbox', { name: 'Subject', exact: true })
        .fill('Close protection check');
      await page
        .getByRole('textbox', { name: 'Message body' })
        .fill('Saved even when closing immediately.');
      const closed = page.waitForEvent('close');
      // The close button closes Qamille in this test window, instead of putting it in the tray.
      await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('close_to_tray_set', { on: false }));
      await page.evaluate(() =>
        window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' }),
      );
      await closed;
      const persisted = JSON.parse(
        fs.readFileSync(path.join(process.env.QAMILLE_TEST_DATA_DIR, 'store.json'), 'utf8'),
      );
      assert.ok(
        persisted.drafts.some(
          (draft) =>
            draft.subject === 'Close protection check' &&
            draft.body.includes('Saved even when closing immediately.'),
        ),
      );
    });
  }
  console.log(JSON.stringify({ native: !!native, passed: results.length, errors }));
} catch (error) {
  await page.screenshot({ path: '../checks/tauri-test-failure.png' }).catch(() => {});
  console.error('Page errors:', errors);
  throw error;
} finally {
  await browser.close().catch(() => {});
}
