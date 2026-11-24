// Notices stay readable and closable while a Modal is open (native audit T14,
// src/ui/Toaster.tsx). An open Ark Modal hides everything outside it (aria-hidden) except
// live regions at <body> level: the toast group is one, in its own Portal, so a notice raised from a
// dialog (a failed save, an error) is announced and its close button works.
// Run: node tests/toaster-layer.mjs (Vite preview on 127.0.0.1:1420). Synthetic sample data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
try {
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1280, height: 860 } });
  await page.goto(base);
  await page.locator('.mail-row').first().waitFor();
  await page.getByRole('list', { name: 'Workspace' }).getByRole('button', { name: 'Contacts' }).click();
  await page.getByRole('button', { name: 'New contact' }).click();
  await expect(page.getByRole('dialog', { name: 'New contact' })).toBeVisible();
  const layers = await page.evaluate(() => {
    const group = document.querySelector('[data-scope="toast"][data-part="group"]');
    return {
      group: !!group,
      onBody: group?.parentElement === document.body || group?.parentElement?.parentElement === document.body,
      live: group?.getAttribute('aria-live'),
      groupHidden: !!group?.closest('[aria-hidden="true"], [inert]'),
      // Base UI marks what is behind a modal `data-base-ui-inert` (and the window says aria-modal): that is hidden for it.
      appHidden: !!document.getElementById('app')?.closest('[aria-hidden="true"], [inert], [data-base-ui-inert]'),
      bodyChildren: [...document.body.children].map((node) => `${node.tagName.toLowerCase()}#${node.id}.${node.className} aria-hidden=${node.getAttribute('aria-hidden')} inert=${node.hasAttribute('inert')} ${[...node.attributes].map((a) => a.name).filter((n) => /base-ui|data-/.test(n)).join(',')}`),
    };
  });
  assert.ok(layers.group, 'the toast group is mounted');
  assert.ok(layers.appHidden, `the page behind the Modal is hidden (the case this test is about): ${JSON.stringify(layers.bodyChildren)}`);
  assert.ok(layers.onBody, 'the toast group is a layer of its own on <body>');
  assert.equal(layers.live, 'polite', 'the toast group is a live region');
  assert.equal(layers.groupHidden, false, 'the toast group is not hidden under an open Modal');
  console.log('PASS a notice raised while a Modal is open is outside the hidden page');

  // Outside #app the group cannot follow `.app-shell.has-composers` as a sibling: the dock reports its height (`--bottom-bar-room`,
  // lib/bottom-room.ts) and Sigrid's notices keep that clear.
  await page.keyboard.press('Escape');
  await page.locator('[data-slot="sidebar"] button', { hasText: 'Inbox' }).first().click();
  await page.locator('.mail-row').first().waitFor();
  const room = () => page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--bottom-bar-room')) || 0);
  assert.equal(await room(), 0, 'no room kept without a composer');
  await page.locator('.mail-row .mail-open').first().focus();
  await page.keyboard.press('n');
  await expect(page.locator('.app-shell.has-composers')).toHaveCount(1);
  await expect.poll(room).toBeGreaterThan(0);
  console.log('PASS with a composer open the notices keep clear of the dock');

  // A notice may carry actions (Sigrid's Toasts draws them as one group of buttons): saved files offer Open and Show in folder.
  // `notify` is called from the page the way the app calls it.
  const toast = page.locator('[data-scope="toast"][data-part="root"]');
  const raise = (message, labels) =>
    page.evaluate(
      async ({ message, labels }) => {
        const { notify } = await import('/src/ui/Toaster.tsx');
        window.noticeRuns ??= [];
        notify(
          message,
          false,
          labels.map((label) => ({ label, run: () => window.noticeRuns.push(label) })),
        );
      },
      { message, labels },
    );
  const runs = () => page.evaluate(() => window.noticeRuns ?? []);
  const focused = () =>
    page.evaluate(() => {
      const el = document.activeElement;
      return el?.getAttribute('aria-label') || el?.textContent?.trim() || el?.tagName;
    });

  await raise('File saved', ['Open', 'Show in folder']);
  const saved = toast.filter({ hasText: 'File saved' });
  await expect(saved).toHaveCount(1);
  const actions = saved.locator('[data-slot="toast-actions"] button');
  assert.deepEqual((await actions.allTextContents()).map((text) => text.trim()), ['Open', 'Show in folder'], 'two actions, before the close button');
  // From the close button (the first of the notice), Tab walks the actions.
  await saved.getByRole('button', { name: /^Close the notice/ }).focus();
  const walked = [];
  for (let step = 0; step < 2; step += 1) {
    await page.keyboard.press('Tab');
    walked.push(await focused());
  }
  assert.deepEqual(walked, ['Open', 'Show in folder'], 'both actions are reachable with Tab');
  // Nothing takes them out of the tab order: not hidden, not inert, no tabindex of -1.
  assert.deepEqual(
    await actions.evaluateAll((buttons) => buttons.map((b) => ({ tab: b.tabIndex, disabled: b.disabled, hidden: !!b.closest('[inert], [aria-hidden="true"]') }))),
    [
      { tab: 0, disabled: false, hidden: false },
      { tab: 0, disabled: false, hidden: false },
    ],
  );
  console.log('PASS a notice with two actions shows two buttons reachable with Tab');

  // A click runs the action.
  await actions.nth(1).click();
  assert.deepEqual(await runs(), ['Show in folder']);
  console.log('PASS a click on an action runs it');

  // Enter on a focused action runs it too.
  await raise('Second file', ['Open']);
  const second = toast.filter({ hasText: 'Second file' });
  await second.getByRole('button', { name: 'Open' }).focus();
  await page.keyboard.press('Enter');
  assert.deepEqual(await runs(), ['Show in folder', 'Open']);
  console.log('PASS Enter on an action runs it');
} finally {
  await browser.close();
}
