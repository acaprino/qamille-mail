import { chromium, expect } from '@playwright/test';
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
page.setDefaultTimeout(15000);
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
if (!native) await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
await page.locator('.compose-main').waitFor();
const snapshot = () =>
  page.evaluate(async () =>
    window.__TAURI_INTERNALS__
      ? window.__TAURI_INTERNALS__.invoke('workspace_snapshot')
      : (await import('/src/lib/api.ts')).call('workspace_snapshot'),
  );
if (native) {
  const state = await snapshot();
  assert.ok(
    state.accounts.length && state.accounts.every((account) => account.kind === 'Sample'),
    'Composer tests require only Sample accounts.',
  );
}
const tabs = page.getByRole('tablist', { name: 'Draft tabs' });
const panel = page.getByRole('tabpanel');
const subject = () => panel.getByRole('textbox', { name: 'Subject', exact: true });
const body = () => panel.getByRole('textbox', { name: 'Message body' });
const titleA = 'Q4 review — draft A';
const titleB = 'Design notes — draft B';
const tab = (title) => tabs.getByRole('tab', { name: title, exact: true });
const activate = async (title) => {
  if ((await tab(title).getAttribute('aria-selected')) !== 'true') await tab(title).click();
  await expect(subject()).toHaveValue(title);
};
async function create(title, text) {
  const previousPanel = (await panel.count()) ? await panel.getAttribute('id') : null;
  await page.locator('.compose-main').click();
  await expect
    .poll(async () => ((await panel.count()) ? await panel.getAttribute('id') : null))
    .not.toBe(previousPanel);
  await subject().fill(title);
  await panel.getByRole('combobox', { name: 'To', exact: true }).fill('sample@example.com');
  await body().fill(text);
}
const results = [];
async function step(name, work) {
  await work();
  results.push(name);
  console.log(`PASS ${name}`);
}
try {
  await step('Two drafts keep separate recipients, formatting and text', async () => {
    await create(titleA, 'First draft content.');
    await body().press('Control+a');
    await panel.getByRole('button', { name: 'Bold', exact: true }).click();
    await create(titleB, 'Second draft content.');
    await expect(tabs.getByRole('tab')).toHaveCount(2);
    await expect(panel).toHaveCount(1);
    await activate(titleA);
    assert.match(await body().innerHTML(), /<strong>First draft content\.<\/strong>/);
    await activate(titleB);
    await expect(body()).toHaveText('Second draft content.');
    await expect
      .poll(async () => (await snapshot()).drafts.find((draft) => draft.subject === titleA)?.body)
      .toContain('First draft content.');
  });
  await step('Collapsed tabs survive navigation and reopen without losing edits', async () => {
    await panel.getByRole('button', { name: 'Minimize draft' }).click();
    await expect(panel).toHaveCount(0);
    await expect(tabs.getByRole('tab')).toHaveCount(2);
    await page.getByRole('button', { name: 'Calendar', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Calendar', exact: true })).toBeVisible();
    await activate(titleB);
    await expect(body()).toHaveText('Second draft content.');
    await body().press('Escape');
    await expect(panel).toHaveCount(0);
    await expect(tab(titleB)).toBeFocused();
  });
  await step('Opening the same draft from Drafts reuses its existing tab', async () => {
    await page.getByRole('button', { name: /^Drafts\s*\d*$/ }).click();
    await page
      .locator('.drafts-list')
      .getByRole('button', { name: new RegExp(titleA) })
      .click();
    await expect(subject()).toHaveValue(titleA);
    await expect(tabs.getByRole('tab')).toHaveCount(2);
    await panel.getByRole('button', { name: 'Minimize draft' }).click();
    await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
  });
  await step(
    'Closing an inactive tab saves its latest text and leaves the other tabs open',
    async () => {
      await activate(titleA);
      await body().fill('Latest A edit before closing its inactive tab.');
      await create('Temporary third draft', 'Temporary text.');
      await page.getByRole('button', { name: `Save and close ${titleA}`, exact: true }).click();
      await expect(tab(titleA)).toHaveCount(0);
      await expect(tab(titleB)).toHaveCount(1);
      await expect
        .poll(async () => (await snapshot()).drafts.find((draft) => draft.subject === titleA)?.body)
        .toContain('Latest A edit');
      await panel.getByRole('button', { name: 'Save and close draft' }).click();
      await page.getByRole('button', { name: /^Drafts\s*\d*$/ }).click();
      await page
        .locator('.drafts-list')
        .getByRole('button', { name: new RegExp(titleA) })
        .click();
      await expect(body()).toHaveText('Latest A edit before closing its inactive tab.');
      await expect(tabs.getByRole('tab')).toHaveCount(2);
    },
  );
  await step('Undo Send restores its tab without replacing another open draft', async () => {
    await panel.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(tab(titleA)).toHaveCount(0);
    await page.getByRole('button', { name: /^Undo: Sending message/ }).click();
    await expect(subject()).toHaveValue(titleA);
    await expect(tabs.getByRole('tab')).toHaveCount(2);
    await activate(titleB);
    await expect(body()).toHaveText('Second draft content.');
  });
  await step('Discard and Undo affect only the selected draft', async () => {
    await activate(titleA);
    await panel.getByRole('button', { name: 'Discard draft' }).click();
    await expect(tab(titleA)).toHaveCount(0);
    await expect(tab(titleB)).toHaveCount(1);
    await page.getByRole('button', { name: /^Undo: Draft discarded/ }).click();
    await expect(subject()).toHaveValue(titleA);
    await expect(tabs.getByRole('tab')).toHaveCount(2);
  });
  await step('Keyboard tab navigation and overflow work at the minimum window size', async () => {
    if (!native) await page.setViewportSize({ width: 1000, height: 640 });
    for (let i = 1; i <= 4; i++) await create(`Extra draft ${i}`, `Independent message ${i}.`);
    await panel.getByRole('button', { name: 'Minimize draft' }).click();
    await tab('Extra draft 4').press('Home');
    const firstTitle = await tabs.getByRole('tab').first().getAttribute('aria-label');
    await expect(tabs.getByRole('tab').first()).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(subject()).toHaveValue(firstTitle);
    const bounds = await panel.boundingBox();
    const viewport = await page.evaluate(() => ({ width: innerWidth, height: innerHeight }));
    assert.ok(
      bounds.x >= 0 &&
        bounds.y >= 48 &&
        bounds.x + bounds.width <= viewport.width &&
        bounds.y + bounds.height <= viewport.height - 46,
    );
    await page.screenshot({ path: '../checks/composer-tabs-small.png' });
    for (let i = 1; i <= 4; i++)
      await page
        .getByRole('button', { name: `Save and close Extra draft ${i}`, exact: true })
        .click();
    if (!native) await page.setViewportSize({ width: 1440, height: 900 });
    await expect(tabs.getByRole('tab')).toHaveCount(2);
  });
  if (native && process.env.QAMILLE_TEST_DATA_DIR)
    await step('A failed save keeps the tab and its text available for retry', async () => {
      await activate(titleA);
      const file = path.resolve(process.env.QAMILLE_TEST_DATA_DIR, 'store.json');
      const relative = path.relative(path.resolve('../checks'), file);
      assert.ok(
        !relative.startsWith('..') && !path.isAbsolute(relative),
        'Fault injection is limited to the project checks directory.',
      );
      const mode = fs.statSync(file).mode & 0o777;
      // Windows rejects replacing a read-only store. Exercise a real disk error,
      // then restore its permissions even if an assertion fails.
      fs.chmodSync(file, 0o444);
      try {
        await body().fill('Text survives a failed save.');
        await panel.getByRole('button', { name: 'Save and close draft' }).click();
        await expect(panel.getByText('Could not save', { exact: true })).toBeVisible();
        await expect(body()).toHaveText('Text survives a failed save.');
        await expect(tab(titleA)).toHaveCount(1);
      } finally {
        fs.chmodSync(file, mode);
      }
      await panel.getByRole('button', { name: 'Minimize draft' }).click();
      await expect
        .poll(async () => (await snapshot()).drafts.find((draft) => draft.subject === titleA)?.body)
        .toContain('Text survives a failed save.');
    });
  await activate(titleA);
  await panel.getByRole('button', { name: 'Minimize draft' }).click();
  await page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
  await openAllGroups(page);
  await page.locator('.mail-open', { hasText: 'Q4 proposal: can you review by Thursday?' }).click();
  await activate(titleB);
  await page.screenshot({
    path: native ? '../checks/composer-tabs-native.png' : '../checks/composer-tabs.png',
  });
  assert.deepEqual(errors, []);
  if (native && process.env.QAMILLE_TEST_DATA_DIR) {
    await step('Closing the app flushes active and collapsed drafts before exiting', async () => {
      await activate(titleA);
      await body().fill('A: saved when the app closes.');
      // Opening a new message leaves A collapsed while its autosave is pending.
      await create('Window close — new draft', 'New draft: saved when the app closes.');
      const closed = page.waitForEvent('close');
      await page.evaluate(() =>
        window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' }),
      );
      await closed;
      const store = JSON.parse(
        fs.readFileSync(path.join(process.env.QAMILLE_TEST_DATA_DIR, 'store.json'), 'utf8'),
      );
      assert.ok(
        store.drafts.some(
          (draft) =>
            draft.subject === titleA && draft.body.includes('A: saved when the app closes.'),
        ),
      );
      assert.ok(
        store.drafts.some(
          (draft) =>
            draft.subject === 'Window close — new draft' &&
            draft.body.includes('New draft: saved when the app closes.'),
        ),
      );
    });
  }
  console.log(JSON.stringify({ native: !!native, passed: results.length, errors }));
} catch (error) {
  await page.screenshot({ path: '../checks/composer-tabs-failure.png' }).catch(() => {});
  throw error;
} finally {
  await browser.close().catch(() => {});
}
