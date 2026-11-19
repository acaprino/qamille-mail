// Access wave 2 (spec A8): the guided form in Settings → Compute of the self-hosted build.
// EX-11 (temporary password, then in), «Next» off while the server is silent, the device limit,
// «Cancel» in «Change server», and no password field left in the page once signed in.
// Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
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
  await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
  await page.getByRole('button', { name: 'Connect to a server', exact: true }).click();
  return { page, context };
}
const form = (page) => page.getByRole('region', { name: 'Server connection' });

try {
  {
    const { page, context } = await open({ probe: 'unreachable' });
    await form(page).getByLabel('Server address').fill('https://calcolo.ufficio.it');
    await expect(form(page).getByText('The server does not answer')).toBeVisible();
    await expect(form(page).getByRole('button', { name: 'Next', exact: true })).toBeDisabled();
    await form(page).getByLabel('Server address').fill('http://calcolo.ufficio.it');
    await expect(form(page).getByText('The address must start with https://')).toBeVisible();
    await form(page).getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(form(page)).toHaveCount(0);
    console.log('PASS «Next» stays off while the server is silent; «Cancel» closes the form');
    await context.close();
  }
  {
    const { page, context } = await open({ login: 'temporary' });
    await form(page).getByLabel('Server address').fill('https://calcolo.ufficio.it');
    // From the keyboard: Next disappears with its step, and the focus goes to the new step's
    // heading, not to <body> (native audit T15).
    await expect(form(page).getByRole('button', { name: 'Next', exact: true })).toBeEnabled();
    await form(page).getByRole('button', { name: 'Next', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect(form(page).getByText(/will be sent to this server/)).toBeVisible();
    await expect(form(page).getByRole('heading', { name: 'Before you connect' })).toBeFocused();
    await form(page).getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await form(page).getByLabel('Username or email').fill('mario');
    await form(page).getByLabel('Password', { exact: true }).fill('provvisoria-123');
    await form(page).getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(form(page).getByRole('heading', { name: 'New password' })).toBeVisible();
    await form(page).getByLabel('New password', { exact: true }).fill('una password nuova');
    await form(page).getByLabel('Repeat the new password').fill('una password nuova');
    await form(page).getByRole('button', { name: 'Change and sign in', exact: true }).click();
    await expect(page.getByText('User:')).toBeVisible();
    await expect(page.getByText('https://calcolo.ufficio.it')).toBeVisible();
    assert.equal(await page.locator('.server-connect input[type="password"]').count(), 0, 'no password field once signed in');
    console.log('PASS EX-11: a temporary password is replaced and the device signs in');

    await page.getByRole('button', { name: 'Change server', exact: true }).click();
    await form(page).getByLabel('Server address').fill('https://nuovo.ufficio.it');
    await form(page).getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByText('https://calcolo.ufficio.it')).toBeVisible();
    console.log('PASS «Cancel» in «Change server» keeps the session in use');
    await context.close();
  }
  {
    const { page, context } = await open({ login: 'limit' });
    await form(page).getByLabel('Server address').fill('https://calcolo.ufficio.it');
    await form(page).getByRole('button', { name: 'Next', exact: true }).click();
    await form(page).getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await form(page).getByLabel('Username or email').fill('mario');
    await form(page).getByLabel('Password', { exact: true }).fill('una password lunga');
    await form(page).getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(form(page).getByText('Vecchio portatile')).toBeVisible();
    await form(page).getByRole('listitem').filter({ hasText: 'Vecchio portatile' }).getByRole('button', { name: 'Disable' }).click();
    await form(page).getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(page.getByText('User:')).toBeVisible();
    console.log('PASS EX-14: at the device limit one device is disabled, then the sign-in goes through');
    await context.close();
  }
  {
    const { page, context } = await open({ local: true });
    const suggestion = form(page).getByRole('button', { name: 'Server on this PC (Docker)', exact: true });
    await expect(suggestion).toBeVisible();
    await suggestion.click();
    await expect(form(page).getByLabel('Server address')).toHaveValue('http://127.0.0.1:8787');
    await expect(form(page).getByText('Qamille server found')).toBeVisible();
    await expect(form(page).getByRole('button', { name: 'Next', exact: true })).toBeEnabled();
    await expect(suggestion).toHaveCount(0, { timeout: 5000 });
    console.log('PASS the Server step offers «Server on this PC (Docker)» when http://127.0.0.1:8787 answers, and the click fills the address and probes it');
    await context.close();
  }
  {
    const { page, context } = await open({});
    await expect(form(page).getByRole('button', { name: 'Server on this PC (Docker)' })).toHaveCount(0);
    await expect(form(page).getByText("Don't have it? The guide explains how to start it with Docker")).toBeVisible();
    console.log('PASS without a local server the step offers nothing and says where the guide is');
    await context.close();
  }
  {
    const { page, context } = await open({ local: true });
    await form(page).getByLabel('Server address').fill('https://calcolo.ufficio.it');
    await expect(form(page).getByRole('button', { name: 'Server on this PC (Docker)' })).toHaveCount(0);
    console.log('PASS a typed address hides the local suggestion');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
