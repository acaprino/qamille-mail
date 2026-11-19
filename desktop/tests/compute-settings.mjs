// Access wave 2 (spec A7): Settings → Compute once connected. «Sign out of this device» with the
// server offline says so; «Delete account» asks for the password behind a red confirmation.
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
async function connected(server) {
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
  await form.getByLabel('Username or email').fill('mario');
  await form.getByLabel('Password', { exact: true }).fill('una password lunga');
  await form.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByText('User:')).toBeVisible();
  return { page, context };
}

try {
  {
    const { page, context } = await connected({ offline: true });
    await expect(page.getByText('· this device')).toBeVisible();
    await page.getByRole('listitem').filter({ hasText: 'Portatile di casa' }).getByRole('button', { name: 'Disable' }).click();
    await expect(page.getByText('Portatile di casa')).toHaveCount(0);
    await page.getByRole('button', { name: 'Sign out of this device', exact: true }).click();
    await expect(page.getByText(/the server did not answer/)).toBeVisible();
    await expect(page.getByRole('button', { name: 'Connect to a server', exact: true })).toBeVisible();
    console.log('PASS devices are listed and disabled; signing out offline says the device stays listed');
    await context.close();
  }
  {
    const { page, context } = await connected({});
    await page.getByRole('button', { name: 'Delete account', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Delete the account?' });
    await expect(dialog.getByRole('button', { name: 'Delete account' })).toBeDisabled();
    await dialog.getByLabel('Password').fill('una password lunga');
    await dialog.getByRole('button', { name: 'Delete account' }).click();
    await expect(page.getByText('Account deleted')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Connect to a server', exact: true })).toBeVisible();
    console.log('PASS «Delete account» asks for the password and signs out');
    await context.close();
  }
  // Not connected: the two modes, the legacy saved value and the guide.
  async function unconnected(server) {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
    const page = await context.newPage();
    page.on('pageerror', (error) => errors.push(error.message));
    page.setDefaultTimeout(15000);
    await page.addInitScript((value) => (window.__previewServer = value), server);
    await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    return { page, context };
  }
  {
    const { page, context } = await unconnected({});
    const mode = page.getByRole('combobox', { name: 'Compute', exact: true });
    await expect(mode).toContainText('No server');
    await mode.click();
    const listed = (await page.getByRole('listbox', { name: 'Compute' }).getByRole('option').allTextContents()).map((t) => t.trim());
    assert.deepEqual(listed, ['No server', 'A Qamille server'], 'two modes, nothing on this computer');
    await page.keyboard.press('Escape');
    await expect(page.getByText(/Without a server the mail and the search by words remain\./)).toBeVisible();
    await expect(page.getByRole('link', { name: 'Server on your PC with Docker' })).toBeVisible();
    console.log('PASS the compute modes are «No server» and «A Qamille server»; there is no «On this computer»');
    await context.close();
  }
  {
    const { page, context } = await unconnected({ savedMode: 'local' });
    await expect(page.getByRole('combobox', { name: 'Compute', exact: true })).toContainText('No server');
    console.log('PASS a legacy local setting shows «No server» selected');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
