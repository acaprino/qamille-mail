// Access wave 2 (spec A6, A8): the first start of the Professional build. Signing in with an
// existing account, EX-19 (server down: in locally, «Retry»), a first start that does not come
// back, no address field anywhere; the self-hosted build has no first start.
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
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

try {
  {
    const { page, context } = await start({ edition: 'professional' });
    await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
    await expect(page.getByText('The server is ready.')).toBeVisible();
    assert.equal(await page.getByLabel('Server address').count(), 0, 'no address field in the Professional build');
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await page.getByLabel('Username or email').fill('mario');
    await page.getByLabel('Password', { exact: true }).fill('una password lunga');
    await page.locator('.server-connect').getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.locator('.settings-nav').getByRole('button', { name: 'Compute', exact: true }).click();
    await expect(page.getByText('User:')).toBeVisible();
    assert.equal(await page.getByText('https://qamille.example').count(), 0, 'the address is not shown');
    assert.equal(await page.getByRole('button', { name: 'Change server' }).count(), 0);
    await page.reload();
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    console.log('PASS signing in with an existing account; no address, no «Change server»');
    await context.close();
  }
  {
    const { page, context } = await start({ edition: 'professional', probe: 'unreachable' });
    await expect(page.getByText('The server does not answer.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Continue without an account', exact: true }).click();
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    console.log('PASS EX-19: with the server down the mail opens as a mail client');
    await context.close();
  }
  {
    const { page, context } = await start({});
    await page.getByRole('button', { name: /^New message/ }).waitFor();
    assert.equal(await page.getByRole('heading', { name: 'Sign in to Qamille' }).count(), 0);
    console.log('PASS the self-hosted build has no first start');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
