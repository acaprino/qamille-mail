import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
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
page.on('pageerror', (error) => errors.push(error.message));
page.setDefaultTimeout(15000);
// Browser preview only: counts the 50 ms intervals (the countdown tick) that are alive right now.
if (!native)
  await page.addInitScript(() => {
    const live = new Map();
    window.__intervals = live;
    const start = window.setInterval.bind(window);
    const stop = window.clearInterval.bind(window);
    window.setInterval = (fn, ms, ...rest) => {
      const id = start(fn, ms, ...rest);
      live.set(id, ms);
      return id;
    };
    window.clearInterval = (id) => {
      live.delete(id);
      stop(id);
    };
  });
const ticks = () =>
  page.evaluate(() => [...window.__intervals.values()].filter((ms) => ms === 50).length);
if (!native) await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
await page.getByRole('button', { name: /^New message/ }).waitFor();
if (native) {
  const kinds = await page.evaluate(async () =>
    (await window.__TAURI_INTERNALS__.invoke('workspace_snapshot')).accounts.map((a) => a.kind),
  );
  assert.ok(
    kinds.length && kinds.every((kind) => kind === 'Sample'),
    'Undo tests require sample accounts only.',
  );
  await page.reload();
  await page.getByRole('button', { name: /^New message/ }).waitFor();
}
const subject = 'Q4 proposal: can you review by Thursday?';
const message = page.locator('.mail-open', { hasText: `${subject}` });
const notices = page.getByRole('region', { name: 'Recent email actions' });
const undo = () => notices.getByRole('button', { name: /^Undo:/ }).last();
const advance = (ms) => page.waitForTimeout(ms);
// A Select: open its button, then choose the item by its label.
const choose = async (select, label) => {
  await select.click();
  await page.getByRole('option', { name: label, exact: true }).click();
};
const inbox = () => page.getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
const open = async () => {
  await inbox();
  // The quiet groups start closed; a message already read lives in one of them.
  await openAllGroups(page);
  await message.click();
};
const results = [];
async function step(name, fn) {
  await fn();
  results.push(name);
  console.log(`PASS ${name}`);
}
try {
  await step('Done removes immediately; Undo restores Inbox and reader', async () => {
    await open();
    await page.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(message).toHaveCount(0);
    await expect(notices.getByText('Marked as done', { exact: true })).toBeVisible();
    await expect(notices.getByRole('progressbar')).toHaveAttribute('aria-valuemax', '5');
    const bounds = await notices.boundingBox();
    assert.ok(bounds.x < 30 && bounds.y > 700, 'Undo sits at the bottom left');
    await page.screenshot({ path: '../checks/undo-light.png' });
    await undo().click();
    await expect(notices).toHaveCount(0);
    await expect(message).toBeVisible();
    await expect(page.getByRole('heading', { name: subject, exact: true })).toBeVisible();
  });
  if (!native)
    await step('Done then Undo within a second lists the row once', async () => {
      await open();
      // The reload that follows Done is still running when Undo is pressed.
      await page.evaluate(() => (window.__previewDelay = 400));
      await page.getByRole('button', { name: 'Done', exact: true }).click();
      await undo().click();
      await expect(notices).toHaveCount(0);
      await expect(page.getByRole('heading', { name: subject, exact: true })).toBeVisible();
      await advance(1200);
      await expect(message).toHaveCount(1);
      const ids = await page
        .locator('.mail-row .visually-hidden[id^="mail-row-"]')
        .evaluateAll((nodes) => nodes.map((node) => node.id));
      assert.equal(new Set(ids).size, ids.length, 'No row appears twice');
      await page.evaluate(() => (window.__previewDelay = 0));
    });
  for (const name of [
    'Archive message',
    'Move message to trash',
    'Snooze until tomorrow',
    'Mark message unread',
  ]) {
    await step(`${name} has a working Undo`, async () => {
      await open();
      await page.getByRole('button', { name, exact: true }).click();
      await undo().click();
      await expect(notices).toHaveCount(0);
      await expect(message).toBeVisible();
    });
  }
  await step(
    'Multiple actions keep independent Undo controls; Ctrl+Z restores the latest',
    async () => {
      await open();
      const reader = page.getByRole('region', { name: 'Reading pane' });
      const pin = reader.getByRole('button', { name: /^(Unpin|Pin) message$/ });
      const original = await pin.getAttribute('aria-label');
      await pin.click();
      await page.getByRole('button', { name: 'Done', exact: true }).click();
      await expect(notices.getByRole('button', { name: /^Undo:/ })).toHaveCount(2);
      if (native) {
        await notices
          .getByRole('button', { name: /^Undo:/ })
          .first()
          .click();
      } else {
        await page.locator('body').click({ position: { x: 700, y: 45 } });
        await page.keyboard.press('Control+z');
      }
      await expect(message).toBeVisible();
      await undo().click();
      await expect(notices).toHaveCount(0);
      await expect(reader.getByRole('button', { name: original, exact: true })).toBeVisible();
    },
  );
  await step('Expiry commits Done to Archive', async () => {
    assert.equal(native ? 0 : await ticks(), 0, 'No notice, no tick');
    await open();
    // Browser preview: keep the action `committing` for a while after its window closes.
    if (!native) await page.evaluate(() => (window.__previewCommitHold = 2500));
    await page.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(notices.getByRole('button', { name: /^Undo:/ })).toBeVisible();
    if (!native) {
      assert.equal(await ticks(), 1, 'One tick while the countdown is on screen');
      // The window closes: the action is only being confirmed, counted by the single activity
      // indicator (ADR-0007 decision 7) — the stack no longer has a "Saving…" line.
      const activity = page.locator('.activity-indicator');
      await expect(activity.getByText('Confirming 1 action…')).toBeVisible({ timeout: 8000 });
      assert.equal(
        await page.locator('.undo-confirming').count(),
        0,
        'The confirming line is not duplicated in the stack',
      );
      await expect(notices.getByRole('button', { name: /^Undo:/ })).toHaveCount(0);
      assert.equal(await ticks(), 0, 'No tick while actions are only confirming');
      await expect(activity).toHaveCount(0, { timeout: 8000 });
      await expect(notices).toHaveCount(0, { timeout: 8000 });
      assert.equal(await ticks(), 0, 'No tick once the stack is gone');
      await page.evaluate(() => (window.__previewCommitHold = 0));
    } else {
      await advance(5500);
      await expect(notices).toHaveCount(0);
    }
    await page.getByRole('button', { name: /^Archive\s*\d*$/ }).click();
    await expect(message).toBeVisible();
    await message.click();
    await page.getByRole('button', { name: 'More message actions' }).click();
    await page.getByRole('button', { name: 'Move to inbox', exact: true }).click();
    await expect(notices.getByRole('button', { name: /^Undo:/ })).toBeVisible();
    await advance(5500);
    await expect(notices).toHaveCount(0);
    await inbox();
    await expect(message).toBeVisible();
  });
  await step('Configured duration and dark-mode progress', async () => {
    await page.getByRole('button', { name: /Alex Personal workspace/ }).click();
    await choose(page.getByRole('combobox', { name: /^Undo time/ }), '10 seconds');
    await choose(page.getByRole('combobox', { name: 'Appearance', exact: true }), 'Dark');
    await page.getByRole('button', { name: 'Save changes' }).click();
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    await open();
    await page.getByRole('button', { name: 'Done', exact: true }).click();
    const progress = notices.getByRole('progressbar');
    await expect(progress).toHaveAttribute('aria-valuemax', '10');
    await advance(2000);
    assert.ok(Number(await progress.getAttribute('aria-valuenow')) <= 8);
    await page.screenshot({ path: '../checks/undo-dark.png' });
    await undo().click();
    await expect(notices).toHaveCount(0);
    await page.getByRole('button', { name: /Alex Personal workspace/ }).click();
    await expect(page.getByRole('combobox', { name: /^Undo time/ })).toContainText('10 seconds');
    await choose(page.getByRole('combobox', { name: /^Undo time/ }), '5 seconds');
    await choose(page.getByRole('combobox', { name: 'Appearance', exact: true }), 'Light');
    await page.getByRole('button', { name: 'Save changes' }).click();
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
  });
  await step('Undo Send reopens the complete draft before delivery', async () => {
    await page.getByRole('button', { name: /^New message/ }).click();
    await page.getByRole('combobox', { name: 'To', exact: true }).fill('sample@example.com');
    await page.getByRole('textbox', { name: 'Subject', exact: true }).fill('Undo send check');
    await page.getByRole('textbox', { name: 'Message body' }).fill('Keep this full draft.');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(page.locator('.composer-dialog, .composer-panel:not([hidden])')).toHaveCount(0);
    await expect(notices.getByText('Sending message', { exact: true })).toBeVisible();
    await undo().click();
    await expect(notices).toHaveCount(0);
    await expect(page.getByRole('textbox', { name: 'Subject', exact: true })).toHaveValue(
      'Undo send check',
    );
    await expect(page.getByRole('textbox', { name: 'Message body' })).toContainText(
      'Keep this full draft.',
    );
  });
  await step('Discard draft also supports Undo', async () => {
    await page.getByRole('button', { name: 'Discard draft' }).click();
    await expect(notices.getByText('Draft discarded', { exact: true })).toBeVisible();
    await undo().click();
    await expect(notices).toHaveCount(0);
    await expect(page.getByRole('textbox', { name: 'Subject', exact: true })).toHaveValue(
      'Undo send check',
    );
    await page.getByRole('button', { name: 'Save and close draft' }).click();
  });
  if (!native)
    await step('A failed action names what could not be confirmed', async () => {
      // The browser preview cannot deliver mail, so a send fails after the countdown.
      await page.getByRole('button', { name: /^Drafts\s*\d*$/ }).click();
      await page
        .getByRole('button', { name: /Undo send check/ })
        .last()
        .click();
      await page.getByRole('button', { name: 'Send', exact: true }).click();
      await advance(5500);
      await expect(
        notices.getByText('Could not confirm: Sending message', { exact: true }),
      ).toBeVisible();
      await notices.getByRole('button', { name: 'Dismiss failed action' }).click();
      await expect(notices).toHaveCount(0);
    });
  if (native)
    await step('Send commits after the countdown using a Sample account only', async () => {
      await page.getByRole('button', { name: /^Drafts\s*\d*$/ }).click();
      await page
        .getByRole('button', { name: /Undo send check/ })
        .last()
        .click();
      await page.getByRole('button', { name: 'Send', exact: true }).click();
      await expect(notices.getByText('Sending message', { exact: true })).toBeVisible();
      await expect(notices.getByRole('button', { name: /^Undo:/ })).toBeVisible();
      await advance(5500);
      await expect(notices).toHaveCount(0);
      await page.getByRole('button', { name: /^Sent\s*\d*$/ }).click();
      await expect(page.locator('.mail-open', { hasText: 'Undo send check' })).toBeVisible();
    });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ native: !!native, passed: results.length, errors }));
} catch (error) {
  await page.screenshot({ path: '../checks/undo-failure.png' }).catch(() => {});
  throw error;
} finally {
  await browser.close();
}
