// Wave 7 (sync spec S6, S9): the consent line of «Before you connect», Settings → Accounts →
// «Mailbox synchronization» and the choice to revoke the access when a device is disabled.
// Synthetic data only; the preview plays the server.
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
async function open(server) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript((value) => (window.__previewServer = value), server);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^New message/ }).waitFor();
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
  return { page, context };
}
async function connect(page) {
  await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
  await page.getByRole('button', { name: 'Connect to a server', exact: true }).click();
  const form = page.getByRole('region', { name: 'Server connection' });
  await form.getByLabel('Server address').fill('https://calcolo.ufficio.it');
  await form.getByRole('button', { name: 'Next', exact: true }).click();
  return form;
}
async function signIn(page, form) {
  await form.getByRole('button', { name: 'I agree, continue', exact: true }).click();
  await form.getByLabel('Username or email').fill('mario');
  await form.getByLabel('Password', { exact: true }).fill('una password lunga');
  await form.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByText('User:')).toBeVisible();
}

try {
  {
    // A server without sync says nothing about mailboxes.
    const { page, context } = await open({});
    const form = await connect(page);
    await expect(form.getByText(/mailboxes you connect/)).toHaveCount(0);
    console.log('PASS a server without sync adds no line to the consent');
    await context.close();
  }
  {
    const { page, context } = await open({ sync: { available: true, gmail: true, removals: ['old@gmail.com'] } });
    const form = await connect(page);
    await expect(form.getByText(/The mailboxes you connect, with their credentials, are saved on this server/)).toBeVisible();
    await signIn(page, form);
    await page.locator('.settings-nav').getByRole('button', { name: 'Accounts', exact: true }).click();
    const section = page.getByRole('region', { name: 'Mailbox synchronization' });
    await expect(section.getByRole('status')).toContainText(/On\./);
    await expect(section.getByRole('alert')).toContainText('old@gmail.com was removed or switched off on another computer');
    await section.getByRole('button', { name: 'Keep it only here' }).click();
    await expect(section.getByRole('alert')).toHaveCount(0);
    const general = section.getByRole('switch', { name: /Sync my mailboxes between my computers/ });
    await expect(general).toBeChecked();
    await general.uncheck();
    await expect(section.getByRole('status')).toContainText(/Off: your mailboxes stay on this computer only/);
    await general.click();
    await expect(general).toBeChecked();
    await expect(section.getByText('alex@studio-example.com')).toBeVisible();
    console.log('PASS consent line, status, removal notice and the switches');
    // Disabling another device offers to revoke the access.
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await page.getByRole('listitem').filter({ hasText: 'Portatile di casa' }).getByRole('button', { name: 'Disable' }).click();
    const group = page.getByRole('group', { name: 'Disable the device' });
    await expect(group.getByRole('checkbox', { name: "Revoke the mailboxes' access" })).toBeChecked();
    await group.getByRole('button', { name: 'Disable', exact: true }).click();
    await expect(page.getByText('Portatile di casa')).toHaveCount(0);
    await expect(page.getByText(/access revoked/)).toBeVisible();
    console.log('PASS disabling a device revokes the mailboxes and says what to do next');
    await context.close();
  }
  {
    const { page, context } = await open({ sync: { available: true, gmail: true, revoke: 'failed' } });
    const form = await connect(page);
    await signIn(page, form);
    await page.getByRole('listitem').filter({ hasText: 'Portatile di casa' }).getByRole('button', { name: 'Disable' }).click();
    await page.getByRole('group', { name: 'Disable the device' }).getByRole('button', { name: 'Disable', exact: true }).click();
    await expect(page.getByText('Portatile di casa')).toHaveCount(0);
    await expect(page.getByText(/revocation not done/)).toBeVisible();
    await expect(page.getByRole('link', { name: 'Permissions of your Google account' })).toBeVisible();
    console.log('PASS a failed revocation is reported as not done, with the way out');
    await context.close();
  }
  if (errors.length) throw new Error(`page errors: ${errors.join(' | ')}`);
  console.log('mailbox sync UI: all checks pass');
} finally {
  await browser.close();
}
