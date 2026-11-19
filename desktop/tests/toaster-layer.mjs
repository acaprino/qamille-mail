// Notices stay readable and closable while a Modal is open (native audit T14,
// src/ds/primitives/Toaster.tsx). An open Ark Modal hides everything outside it (aria-hidden) except
// live regions at <body> level: the toast group is one, in its own Portal, so a notice raised from a
// dialog (a failed save, an error) is announced and its close button works.
// Run: node tests/toaster-layer.mjs (Vite preview on 127.0.0.1:1420). Synthetic sample data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true });
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
    };
  });
  assert.ok(layers.group, 'the toast group is mounted');
  assert.ok(layers.appHidden, 'the page behind the Modal is hidden (the case this test is about)');
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
} finally {
  await browser.close();
}
