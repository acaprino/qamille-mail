// Access wave 2 (spec A6, A8): the sign-in screen of the Professional build. The Professional app needs an
// account (owner, 2026-11-19): signing in with an existing account, no way past the screen without one (not
// with the server down either: «Retry»), «Cancel» back to the screen, a start with a session straight to the
// mail, signing out back to the screen, no address field anywhere; the self-hosted build has no sign-in screen.
import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { chromium, expect } from '@playwright/test';

// With QAMILLE_SHOTS_DIR set (CI), the screens are also saved there to be looked at.
const shots = process.env.QAMILLE_SHOTS_DIR;
if (shots) mkdirSync(shots, { recursive: true });
async function shoot(page, name) {
  if (!shots) return;
  for (const [width, height] of [[1280, 800], [960, 680]]) {
    await page.setViewportSize({ width, height });
    await page.screenshot({ path: path.join(shots, `${name}-${width}x${height}.png`) });
  }
  await page.setViewportSize({ width: 1280, height: 900 });
}

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
async function start(server) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript((value) => (window.__previewServer = value), server);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  return { page, context };
}
const noAccountButton = (page) => page.getByRole('button', { name: 'Continue without an account', exact: true }).count();

try {
  {
    const { page, context } = await start({ edition: 'professional' });
    await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
    await expect(page.getByText('The server is ready.')).toBeVisible();
    await shoot(page, 'signin-choice');
    assert.equal(await noAccountButton(page), 0, 'no way past the screen without an account');
    assert.equal(await page.getByLabel('Server address').count(), 0, 'no address field in the Professional build');
    // «Cancel» in the form goes back to the screen, never into the app.
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.locator('.server-connect').getByRole('button', { name: 'Cancel', exact: true }).first().click();
    await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
    assert.equal(await page.getByRole('button', { name: /^New message/ }).count(), 0, 'Cancel does not open the app');
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await shoot(page, 'signin-consent');
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await shoot(page, 'signin-form');
    await page.getByLabel('Username or email').fill('mario');
    await page.getByLabel('Password', { exact: true }).fill('una password lunga');
    await page.locator('.server-connect').getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await expect(page.getByText('User:')).toBeVisible();
    assert.equal(await page.getByText('https://qamille.example').count(), 0, 'the address is not shown');
    assert.equal(await page.getByRole('button', { name: 'Change server' }).count(), 0);
    await page.getByRole('button', { name: 'Sign out of this device', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
    console.log('PASS signing in with an existing account; Cancel stays on the screen; signing out goes back to it');
    await context.close();
  }
  {
    const { page, context } = await start({ edition: 'professional', probe: 'unreachable' });
    await expect(page.getByText('The server does not answer.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
    await shoot(page, 'signin-server-down');
    assert.equal(await noAccountButton(page), 0, 'no way past the screen with the server down');
    assert.equal(await page.getByRole('button', { name: /^New message/ }).count(), 0, 'the mail does not open without an account');
    console.log('PASS with the server down the screen stays, with «Retry»');
    await context.close();
  }
  {
    const { page, context } = await start({ edition: 'professional', connected: true });
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    assert.equal(await page.getByRole('heading', { name: 'Sign in to Qamille' }).count(), 0);
    console.log('PASS a start with a session goes straight to the mail');
    await context.close();
  }
  {
    const { page, context } = await start({});
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    assert.equal(await page.getByRole('heading', { name: 'Sign in to Qamille' }).count(), 0);
    console.log('PASS the self-hosted build has no sign-in screen');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
