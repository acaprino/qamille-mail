// Selling Professional (spec «Vendita Professional», V3, V6): «Continue with Google» on the first
// start of the Professional build, the trial, a subscription paid on the processor's page that
// moves compute to the server, a declined card, an account without a Google sign-in, and the
// deletion of an account without password confirmed by signing in again.
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

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
async function signInWithGoogle(page) {
  await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
  await page.getByRole('button', { name: /^(Continue|Sign in) with Google$/ }).click();
}
async function openCompute(page) {
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  await page.getByRole('navigation', { name: 'Sidebar' }).getByRole('button', { name: /^Settings/ }).click();
  await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
  await expect(page.getByText('User:')).toBeVisible();
}

try {
  {
    const { page, context } = await start({ edition: 'professional', signup: true, google: true, billing: true });
    await signInWithGoogle(page);
    await openCompute(page);
    const panel = page.getByRole('region', { name: 'Subscription' });
    await expect(page.getByText('signs in with Google')).toBeVisible();
    await expect(panel.getByText('Test mode: fake payments, no charge.')).toBeVisible();
    await expect(panel.getByText(/^Free trial until/)).toBeVisible();
    assert.equal(await page.getByRole('heading', { name: 'Change password' }).count(), 0, 'no password to change');
    await expect(panel.getByText('Qamille Professional mensile')).toBeVisible();
    await expect(panel.getByText('€9.00 a month')).toBeVisible();

    await panel.getByRole('listitem').filter({ hasText: 'annuale' }).getByRole('button', { name: 'Subscribe' }).click();
    await expect(page.getByText('Subscription active: compute moves to the server.')).toBeVisible();
    await expect(panel.getByText(/^Qamille Professional annuale: active, renews on/)).toBeVisible();
    await expect(panel.getByRole('button', { name: 'Manage subscription' })).toBeVisible();
    await expect(panel.getByRole('button', { name: 'Subscribe' })).toHaveCount(0);
    await expect(page.getByRole('combobox', { name: 'Compute' })).toContainText('Qamille server');
    console.log('PASS Google sign-up gets a trial; paying on the processor page activates and moves compute to the server');

    await page.getByRole('button', { name: 'Delete account', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Delete the account?' });
    await expect(dialog.getByText(/sign in with Google again/)).toBeVisible();
    // Wave 9: a subscription that pays is ended by the server at the processor; the dialog says so first.
    await expect(dialog.getByText(/cancel it at once with the payment processor/)).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Manage subscription' })).toBeVisible();
    assert.equal(await dialog.getByLabel('Password').count(), 0);
    // The page reloads at once (Professional): the notice of the deletion goes with it, the sign-in screen is what is seen.
    await page.evaluate(() => (window.__beforeReload = true));
    await dialog.getByRole('button', { name: 'Confirm with Google and delete' }).click();
    await page.waitForFunction(() => !window.__beforeReload, null, { timeout: 15000 }).catch(() => {});
    await page.waitForLoadState('load');
    const signIn = page.getByText('Sign in to Qamille', { exact: true });
    try {
      await expect(signIn).toBeVisible({ timeout: 15000 });
    } catch (error) {
      const said = (await page.locator('body').innerText().catch(() => 'no body')).replace(/\s+/g, ' ').slice(0, 400);
      console.log(`MOTION after the reload (${page.url()}): ${said}`);
      throw error;
    }
    console.log('PASS an account without password is deleted after signing in with Google again');
    await context.close();
  }
  {
    // Wave 9: the consumption of the month, a full meter says what stops and when it comes back.
    const { page, context } = await start({ edition: 'professional', google: true, billing: true, quotas: { full: ['llm_tokens'] } });
    await signInWithGoogle(page);
    await openCompute(page);
    const panel = page.getByRole('region', { name: 'Subscription' });
    await expect(panel.getByRole('progressbar')).toHaveCount(3);
    await expect(panel.getByText('Resets on 1 December.')).toBeVisible();
    await expect(panel.getByText(/^The assistant is paused: you used all the tokens of this month\. They come back on 1 December\.$/)).toBeVisible();
    await expect(panel.getByRole('button', { name: 'Switch to a bigger plan' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Delete account', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Delete the account?' }).getByText('The trial ends with the account.')).toBeVisible();
    await page.keyboard.press('Escape');
    await panel.getByRole('listitem').filter({ hasText: 'mensile' }).getByRole('button', { name: 'Subscribe' }).click();
    await expect(panel.getByText(/^Qamille Professional mensile: active/)).toBeVisible();
    await expect(panel.getByRole('button', { name: 'Switch to a bigger plan' })).toBeVisible();
    console.log('PASS the panel shows the month, a full meter says what stops, and offers a bigger plan only where there is one');
    await context.close();
  }
  {
    const { page, context } = await start({ edition: 'professional', google: true, billing: true, trial: false, declined: true });
    await signInWithGoogle(page);
    await openCompute(page);
    const panel = page.getByRole('region', { name: 'Subscription' });
    await expect(panel.getByText('No subscription.')).toBeVisible();
    await expect(panel.getByText(/search by words remain/)).toBeVisible();
    await expect(page.getByRole('combobox', { name: 'Compute' })).toContainText('No server');
    await panel.getByRole('listitem').filter({ hasText: 'mensile' }).getByRole('button', { name: 'Subscribe' }).click();
    await expect(panel.getByText('The payment did not go through. You can try again.')).toBeVisible();
    await expect(panel.getByRole('button', { name: 'Subscribe' })).toHaveCount(2);
    console.log('PASS without a subscription compute stays local; a declined card says so and the plans stay');
    await context.close();
  }
  {
    const { page, context } = await start({ edition: 'professional', google: true, googleAnswer: 'no_account' });
    await signInWithGoogle(page);
    await expect(page.getByText(/No Qamille account uses this Google address/)).toBeVisible();
    console.log('PASS a Google address without an account is told so');
    await context.close();
  }
  {
    const { page, context } = await start({ edition: 'professional' });
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await expect(page.getByLabel('Username or email')).toBeVisible();
    assert.equal(await page.getByRole('button', { name: /with Google/ }).count(), 0, 'no Google button where the server offers none');
    console.log('PASS servers without Google sign-in show only username and password');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
