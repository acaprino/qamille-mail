// Access wave 3 (spec A9, EX-12): «Create account» from the Professional first start with a code
// by mail; a wrong code; the self-hosted build never offers sign-up, even when the server would.
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
async function start(server) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 950 }, locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript((value) => (window.__previewServer = value), server);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  return { page, context };
}

try {
  {
    const { page, context } = await start({ edition: 'professional', signup: true });
    await page.getByRole('button', { name: 'Create account', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    const form = page.getByRole('region', { name: 'Create account' });
    await form.getByLabel('Email').fill('anna@example.com');
    await form.getByRole('button', { name: 'Send me the code', exact: true }).click();
    await expect(form.getByText('I sent you a code: check your mail.')).toBeVisible();
    await expect(form.getByRole('button', { name: /Send the code again \(\d+ s\)/ })).toBeDisabled();
    await form.getByLabel('Code').fill('00000000');
    await form.getByLabel('Password', { exact: true }).fill('una password lunga');
    await form.getByLabel('Repeat the password').fill('una password lunga');
    await form.getByRole('button', { name: 'Create account and sign in', exact: true }).click();
    await expect(form.getByRole('alert')).toContainText('The code is wrong or expired');
    console.log('PASS a wrong code is refused');
    await form.getByLabel('Code').fill('12345678');
    await form.getByRole('button', { name: 'Create account and sign in', exact: true }).click();
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await expect(page.getByText('User:')).toBeVisible();
    await expect(page.getByText('anna', { exact: true })).toBeVisible();
    console.log('PASS EX-12: an account is created with the mailed code and the app is signed in');
    await context.close();
  }
  {
    // Wave 9: a server that declares its Terms asks for the box before «Create account»; the version
    // sent back is the one it declared, and the addresses open in the browser (https only).
    const legal = { terms_url: 'https://calcolo.ufficio.it/terms', privacy_url: 'https://calcolo.ufficio.it/privacy', terms_version: '2026-12-01' };
    const { page, context } = await start({ edition: 'professional', signup: true, google: true, legal });
    await page.getByRole('button', { name: 'Create account', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    const form = page.getByRole('region', { name: 'Create account' });
    await form.getByLabel('Email').fill('anna@example.com');
    const box = form.getByRole('checkbox', { name: 'I have read the Terms and the privacy notice' });
    await expect(box).not.toBeChecked();
    await expect(form.getByRole('button', { name: 'Send me the code', exact: true })).toBeDisabled();
    await form.getByText('I have read the Terms and the privacy notice').click();
    await expect(form.getByRole('button', { name: 'Send me the code', exact: true })).toBeEnabled();
    await expect(form.getByRole('link', { name: 'Read the Terms' })).toHaveAttribute('href', 'https://calcolo.ufficio.it/terms');
    await expect(form.getByRole('link', { name: 'Read the privacy notice' })).toHaveAttribute('href', 'https://calcolo.ufficio.it/privacy');
    await form.getByRole('button', { name: 'Send me the code', exact: true }).click();
    await form.getByLabel('Code').fill('12345678');
    await form.getByLabel('Password', { exact: true }).fill('una password lunga');
    await form.getByLabel('Repeat the password').fill('una password lunga');
    await form.getByRole('button', { name: 'Create account and sign in', exact: true }).click();
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    console.log('PASS the Terms box comes first and the account is created with the declared version');
    await context.close();
  }
  {
    const legal = { terms_url: 'https://calcolo.ufficio.it/terms', terms_version: '2026-12-01' };
    const { page, context } = await start({ edition: 'professional', signup: true, google: true, legal });
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    const google = page.getByRole('button', { name: 'Continue with Google' });
    await expect(google).toBeDisabled();
    await page.getByText('I have read the Terms and the privacy notice').click();
    await expect(google).toBeEnabled();
    console.log('PASS «Continue with Google» waits for the Terms box');
    await context.close();
  }
  {
    const { page, context } = await start({ signup: true, password_reset: true });
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await page.getByRole('button', { name: 'Connect to a server', exact: true }).click();
    const form = page.getByRole('region', { name: 'Server connection' });
    await form.getByLabel('Server address').fill('https://calcolo.ufficio.it');
    await form.getByRole('button', { name: 'Next', exact: true }).click();
    await form.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await expect(form.getByRole('button', { name: 'Forgot password?' })).toBeVisible();
    assert.equal(await form.getByRole('button', { name: 'Create account' }).count(), 0, 'no sign-up in the self-hosted build');
    console.log('PASS the self-hosted build has no «Create account», even when the server offers it');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
