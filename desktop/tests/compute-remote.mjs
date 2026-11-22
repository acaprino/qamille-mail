// Compute wave 3 (spec D4, D7, A8 «401 after sign-in», EX-15): Settings → Compute in remote mode
// shows the server's models (ready, downloading with a percentage, missing) and installs nothing;
// a device the server no longer knows gets «Sign in again», which opens the guided form on
// «Sign in» for the same address; a server with another model than the index offers «Reindex».
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
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
    const { page, context } = await connected({});
    const models = page.getByRole('list', { name: 'Server models' });
    await expect(models.getByRole('listitem').filter({ hasText: 'EmbeddingGemma 2' })).toContainText('ready');
    await expect(models.getByRole('listitem').filter({ hasText: 'Whisper base' })).toContainText('downloading 40%');
    await expect(models.getByRole('listitem').filter({ hasText: 'Whisper small' })).toContainText('missing');
    await expect(page.getByText('Models on the server are installed by its administrator.')).toBeVisible();
    await expect(page.getByRole('button', { name: /Install/ })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Sign in again' })).toHaveCount(0);
    console.log('PASS the remote server\'s models: ready, downloading with a percentage, missing; nothing to install');
    await context.close();
  }
  {
    const { page, context } = await connected({ unlinked: true });
    await expect(page.getByRole('alert').filter({ hasText: 'This device is no longer connected to the server' })).toBeVisible();
    await expect(page.getByRole('list', { name: 'Server models' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Sign in again', exact: true }).click();
    const form = page.getByRole('region', { name: 'Server connection' });
    await expect(form.getByRole('heading', { name: 'Sign in' })).toBeVisible();
    await expect(form.getByLabel('Server address')).toHaveCount(0);
    await form.getByLabel('Username or email').fill('mario');
    await form.getByLabel('Password', { exact: true }).fill('una password lunga');
    await form.getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(page.getByRole('list', { name: 'Server models' })).toBeVisible();
    await expect(page.getByText('This device is no longer connected to the server')).toHaveCount(0);
    console.log('PASS after a 401 «Sign in again» opens the form on «Sign in» for the same server (EX-15)');
    await context.close();
  }
  {
    const { page, context } = await connected({ mismatch: true });
    const alert = page.getByRole('alert').filter({ hasText: 'The server uses a different model from the index' });
    await expect(alert).toContainText('nomic-embed-text, 768 dimensions');
    await expect(alert).toContainText('EmbeddingGemma 2, 256 dimensions');
    await alert.getByRole('button', { name: 'Reindex', exact: true }).click();
    await expect(page.getByText('Reindexing started')).toBeVisible();
    await expect(page.getByText('The server uses a different model from the index')).toHaveCount(0);
    console.log('PASS another model than the index (D4) is said in Compute with «Reindex»');
    await context.close();
  }
  {
    const { page, context } = await connected({ mismatch: true });
    await page.locator('.settings-nav').getByRole('button', { name: 'Search', exact: true }).click();
    await expect(page.getByText('Server model differs from the index').first()).toBeVisible();
    await page.getByRole('button', { name: 'Reindex', exact: true }).first().click();
    await expect(page.getByText('Server model differs from the index')).toHaveCount(0);
    console.log('PASS the Search section shows the blocked indexing and «Reindex»');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
