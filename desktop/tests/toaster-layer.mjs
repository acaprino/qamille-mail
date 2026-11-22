// Notices stay readable and closable while a Modal is open (native audit T14,
// src/ui/Toaster.tsx). An open Ark Modal hides everything outside it (aria-hidden) except
// live regions at <body> level: the toast group is one, in its own Portal, so a notice raised from a
// dialog (a failed save, an error) is announced and its close button works.
// Run: node tests/toaster-layer.mjs (Vite preview on 127.0.0.1:1420). Synthetic sample data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
try {
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1280, height: 860 } });
  await page.goto(base);
  await page.locator('.mail-row').first().waitFor();
  await page.getByRole('navigation', { name: 'Workspace' }).getByRole('button', { name: 'Contacts' }).click();
  await page.getByRole('button', { name: 'New contact' }).click();
  await expect(page.getByRole('dialog', { name: 'New contact' })).toBeVisible();
  const layers = await page.evaluate(() => {
    const group = document.querySelector('[data-scope="toast"][data-part="group"]');
    return {
      group: !!group,
      onBody: group?.parentElement === document.body || group?.parentElement?.parentElement === document.body,
      live: group?.getAttribute('aria-live'),
      groupHidden: !!group?.closest('[aria-hidden="true"], [inert]'),
      appHidden: !!document.getElementById('app')?.closest('[aria-hidden="true"], [inert]'),
      bodyChildren: [...document.body.children].map((node) => `${node.tagName.toLowerCase()}#${node.id}.${node.className} aria-hidden=${node.getAttribute('aria-hidden')} inert=${node.hasAttribute('inert')} ${[...node.attributes].map((a) => a.name).filter((n) => /base-ui|data-/.test(n)).join(',')}`),
    };
  });
  assert.ok(layers.group, 'the toast group is mounted');
  assert.ok(layers.appHidden, `the page behind the Modal is hidden (the case this test is about): ${JSON.stringify(layers.bodyChildren)}`);
  assert.ok(layers.onBody, 'the toast group is a layer of its own on <body>');
  assert.equal(layers.live, 'polite', 'the toast group is a live region');
  assert.equal(layers.groupHidden, false, 'the toast group is not hidden under an open Modal');
  console.log('PASS a notice raised while a Modal is open is outside the hidden page');

  // Outside #app the group can no longer follow `.app-shell.has-composers` as a sibling: the lift
  // above the composer dock comes from :root:has(…) in styles/ark/toast.css.
  await page.keyboard.press('Escape');
  await page.locator('.sidebar nav button', { hasText: 'Inbox' }).first().click();
  await page.locator('.mail-row').first().waitFor();
  const lift = () =>
    page.evaluate(() =>
      getComputedStyle(document.querySelector('[data-scope="toast"][data-part="group"]')).getPropertyValue('--toast-bottom').trim(),
    );
  assert.equal(await lift(), '', 'no lift without a composer');
  await page.locator('.mail-row .mail-open').first().focus();
  await page.keyboard.press('n');
  await expect(page.locator('.app-shell.has-composers')).toHaveCount(1);
  assert.equal(await lift(), '62px', 'with a composer open the notice lifts above the dock');
  console.log('PASS with a composer open the notice lifts above the dock');

  // A notice may carry up to two actions (ds primitive buttons inside it): saved files offer Open and
  // Show in folder. `notify` is called from the page the way the app calls it.
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
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText('File saved');
  assert.deepEqual(
    await toast.locator('button.secondary').allTextContents(),
    ['Open', 'Show in folder'],
    'two actions, as ds buttons, before the close button',
  );
  // From the notice itself, Tab walks the actions and then the close button.
  await toast.focus();
  const walked = [];
  for (let step = 0; step < 3; step += 1) {
    await page.keyboard.press('Tab');
    walked.push(await focused());
  }
  assert.deepEqual(walked, ['Open', 'Show in folder', 'Dismiss notification'], 'both actions are reachable with Tab');
  // Nothing takes them out of the tab order: not hidden, not inert, no tabindex of -1.
  assert.deepEqual(
    await toast.locator('button.secondary').evaluateAll((buttons) =>
      buttons.map((b) => ({ tab: b.tabIndex, disabled: b.disabled, hidden: !!b.closest('[inert], [aria-hidden="true"]') })),
    ),
    [
      { tab: 0, disabled: false, hidden: false },
      { tab: 0, disabled: false, hidden: false },
    ],
  );
  console.log('PASS a notice with two actions shows two buttons reachable with Tab');

  // Focus inside keeps the notice open, past the 10 s an actions notice stays.
  await page.clock.install();
  await raise('Kept while focused', ['Open', 'Show in folder']);
  await expect(toast).toContainText('Kept while focused');
  await toast.getByRole('button', { name: 'Open' }).focus();
  await page.clock.runFor(11000);
  await expect(toast).toHaveCount(1);
  console.log('PASS a notice with actions waits while focus is on it');

  // Enter on a focused action runs it and closes the notice.
  await page.keyboard.press('Enter');
  assert.deepEqual(await runs(), ['Open']);
  await page.clock.runFor(1000);
  await expect(toast).toHaveCount(0);
  console.log('PASS Enter on an action runs it and closes the notice');

  // 10 s with actions, against 4.5 s without.
  await raise('Quick', []);
  await expect(toast).toContainText('Quick');
  await page.clock.runFor(5000);
  await expect(toast).toHaveCount(0);
  await raise('Offers', ['Open']);
  await expect(toast).toContainText('Offers');
  await page.clock.runFor(5000);
  await expect(toast).toHaveCount(1);
  await page.clock.runFor(6000);
  await expect(toast).toHaveCount(0);
  console.log('PASS a notice with actions stays 10 s, one without 4.5 s');
  await page.clock.resume();

  // A click runs the action and closes the notice; the next notice replaces the last one.
  await raise('Second file', ['Open', 'Show in folder']);
  await toast.getByRole('button', { name: 'Show in folder' }).click();
  assert.deepEqual(await runs(), ['Open', 'Show in folder']);
  await expect(toast).toHaveCount(0);
  await raise('First', ['One']);
  await expect(toast).toContainText('First');
  await raise('Second', ['Two']);
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText('Second');
  assert.deepEqual(await toast.locator('button.secondary').allTextContents(), ['Two']);
  // More than two actions never fit: the extra one is dropped.
  await raise('Crowded', ['A', 'B', 'C']);
  await expect(toast).toContainText('Crowded');
  assert.deepEqual(await toast.locator('button.secondary').allTextContents(), ['A', 'B']);
  console.log('PASS a click runs the action and closes the notice, and a new notice replaces it');
} finally {
  await browser.close();
}
