// Phase 3, Task 7: the key of the credentials. A data folder from another Windows user or computer
// opens with a dialog that asks for the transfer passphrase or offers a new sign-in, which always
// asks first and sets the old credentials aside; Settings → Accounts → Credentials sets, changes and
// removes the transfer passphrase. Synthetic data only; the preview plays the vault.
// Run against the browser preview: QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

const browser = await chromium.launch({ channel: 'msedge', headless: true });
const errors = [];
async function open(vault, locale = 'en-US') {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript((value) => (window.__previewVault = value), vault);
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.getByRole('button', { name: /^(New message|Nuovo messaggio)/ }).waitFor();
  return { page, context };
}
const vault = (page) => page.evaluate(() => window.__previewVault);
const toast = (page) => page.locator('[data-scope="toast"][data-part="root"]');
async function credentials(page) {
  await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
  await page.locator('.settings-nav').getByRole('button', { name: 'Accounts', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Credentials', exact: true })).toBeVisible();
}

try {
  {
    const { page, context } = await open({ state: 'locked', transfer: true, passphrase: 'correct horse battery' });
    const dialog = page.getByRole('dialog', { name: 'Credentials from another computer' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText('the account credentials open with the transfer passphrase chosen there');
    const field = dialog.getByLabel('Transfer passphrase');
    await expect(dialog.getByRole('button', { name: 'Open the credentials' })).toBeDisabled();
    await field.fill('wrong horse battery');
    await field.press('Enter');
    await expect(toast(page).filter({ hasText: 'The transfer passphrase is not correct.' })).toBeVisible();
    await expect(dialog).toBeVisible();
    assert.equal((await vault(page)).state, 'locked', 'a wrong passphrase changes nothing');
    await field.fill('correct horse battery');
    await dialog.getByRole('button', { name: 'Open the credentials' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(toast(page).filter({ hasText: 'Credentials opened: Windows now protects them for this user.' })).toBeVisible();
    assert.equal((await vault(page)).state, 'ready');
    assert.equal((await vault(page)).startedOver, undefined, 'opening with the passphrase sets nothing aside');
    console.log('PASS a folder from another computer asks for the transfer passphrase; a wrong one changes nothing');
    await context.close();
  }
  {
    const { page, context } = await open({ state: 'locked', transfer: false });
    const dialog = page.getByRole('dialog', { name: 'Credentials from another computer' });
    await expect(dialog).toContainText(
      'The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings. Mail and settings stay.',
    );
    await expect(dialog.getByLabel('Transfer passphrase')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Sign in to the accounts again' }).click();
    await expect(dialog).toContainText('without being deleted');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    assert.equal((await vault(page)).startedOver, undefined, 'cancel sets nothing aside');
    await dialog.getByRole('button', { name: 'Sign in to the accounts again' }).click();
    await dialog.getByRole('button', { name: 'Sign in to the accounts again' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(toast(page).filter({ hasText: 'The previous credentials are in data\\credentials-set-aside-20261121-101500' })).toBeVisible();
    assert.equal((await vault(page)).startedOver, 1);
    console.log('PASS without a passphrase the dialog says to sign in again, and starting over asks first');
    await context.close();
  }
  {
    const { page, context } = await open({ state: 'locked', transfer: true, passphrase: 'correct horse battery' });
    const dialog = page.getByRole('dialog', { name: 'Credentials from another computer' });
    await dialog.getByRole('button', { name: 'Later', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    // Settings shows the same controls; the mail is there meanwhile.
    await credentials(page);
    await expect(page.getByLabel('Transfer passphrase')).toBeVisible();
    await page.getByRole('button', { name: 'Forgotten: sign in again' }).click();
    await expect(page.getByText('without being deleted')).toBeVisible();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.getByLabel('Transfer passphrase').fill('correct horse battery');
    await page.getByRole('button', { name: 'Open the credentials' }).click();
    await expect(page.getByRole('button', { name: 'Change the passphrase' })).toBeVisible();
    assert.equal((await vault(page)).state, 'ready');
    console.log('PASS «Later» closes the dialog; Settings → Accounts opens the credentials too');
    await context.close();
  }
  {
    const { page, context } = await open({});
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await credentials(page);
    await expect(page.getByText('encrypted with a key that Windows protects for this user')).toBeVisible();
    await page.getByRole('button', { name: 'Set a transfer passphrase' }).click();
    const save = page.getByRole('button', { name: 'Save the passphrase' });
    await page.getByLabel('New transfer passphrase').fill('short');
    await page.getByLabel('Repeat the passphrase').fill('short');
    await expect(save).toBeDisabled();
    await page.getByLabel('New transfer passphrase').fill('a passphrase for the move');
    await page.getByLabel('Repeat the passphrase').fill('a passphrase for the mvoe');
    await save.click();
    await expect(page.getByRole('alert').filter({ hasText: 'The two passphrases differ.' })).toBeVisible();
    assert.equal((await vault(page)).transfer, undefined, 'two different passphrases save nothing');
    await page.getByLabel('Repeat the passphrase').fill('a passphrase for the move');
    await page.getByLabel('Repeat the passphrase').press('Enter');
    await expect(toast(page).filter({ hasText: 'Transfer passphrase saved.' })).toBeVisible();
    assert.equal((await vault(page)).passphrase, 'a passphrase for the move');
    await expect(page.getByLabel('New transfer passphrase')).toHaveCount(0);
    await expect(page.locator('.tag').filter({ hasText: /^Set$/ })).toBeVisible();
    await page.getByRole('button', { name: 'Remove the passphrase' }).click();
    await expect(toast(page).filter({ hasText: 'Transfer passphrase removed.' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Set a transfer passphrase' })).toBeVisible();
    assert.equal((await vault(page)).transfer, false);
    console.log('PASS Settings sets the transfer passphrase (twice, at least 8 characters) and removes it');
    await context.close();
  }
  {
    const { page, context } = await open({ state: 'locked', transfer: false }, 'it-IT');
    const dialog = page.getByRole('dialog', { name: 'Credenziali di un altro computer' });
    await expect(dialog).toContainText(
      'La chiave delle credenziali appartiene a un altro utente di Windows o a un altro computer: accedi di nuovo agli account in Impostazioni.',
    );
    await expect(dialog.getByRole('button', { name: 'Più tardi', exact: true })).toBeVisible();
    console.log('PASS the dialog speaks Italian');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
