// Access wave 3 (spec A10): «Forgot password?» when the server offers it: email → code → new
// password → back to «Sign in» with the email filled in; a wrong code; no link without SMTP.
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
async function signInStep(server) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript((value) => (window.__previewServer = value), server);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
  await page.getByRole('button', { name: 'Connect to a server', exact: true }).click();
  const form = page.getByRole('region', { name: 'Server connection' });
  await form.getByLabel('Server address').fill('https://calcolo.ufficio.it');
  await form.getByRole('button', { name: 'Next', exact: true }).click();
  await form.getByRole('button', { name: 'I agree, continue', exact: true }).click();
  return { page, context, form };
}

try {
  {
    const { context, form } = await signInStep({ password_reset: true });
    await form.getByRole('button', { name: 'Forgot password?' }).click();
    const reset = form.getByRole('region', { name: 'Forgot password' });
    await reset.getByLabel('Email').fill('mario@example.com');
    await reset.getByRole('button', { name: 'Send me the code', exact: true }).click();
    await expect(reset.getByText('If the email belongs to an account, I sent you a code.')).toBeVisible();
    await reset.getByLabel('Code').fill('11111111');
    await reset.getByLabel('New password').fill('una password nuova');
    await reset.getByRole('button', { name: 'Set the new password', exact: true }).click();
    await expect(reset.getByRole('alert')).toContainText('The code is wrong or expired');
    await reset.getByLabel('Code').fill('12345678');
    await reset.getByRole('button', { name: 'Set the new password', exact: true }).click();
    await expect(form.getByLabel('Username or email')).toHaveValue('mario@example.com');
    console.log('PASS the reset goes back to «Sign in» with the email filled in; a wrong code is refused');
    await context.close();
  }
  {
    const { context, form } = await signInStep({ password_reset: false });
    await expect(form.getByLabel('Username or email')).toBeVisible();
    assert.equal(await form.getByRole('button', { name: 'Forgot password?' }).count(), 0);
    console.log('PASS no «Forgot password?» when the server does not offer it');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
