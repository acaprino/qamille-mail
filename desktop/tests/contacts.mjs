// Contacts flows against the browser preview backend: no real address book is touched.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 860 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
await page.goto(base);
await page.locator('.compose-main').waitFor();

const row = (name) => page.getByRole('list', { name: 'Contacts' }).getByRole('button', { name: new RegExp(name) });
// The Ark Checkbox draws its own box and hides the input: a click goes to the visible row.
const tick = (scope, name) => scope.locator('[data-scope="checkbox"][data-part="root"]').filter({ hasText: name });
const duplicatesButton = () => page.getByRole('button', { name: /^Duplicates/ });
const results = [];
async function step(name, work) {
  await work();
  results.push(name);
  console.log(`PASS ${name}`);
}

try {
  await step('Contacts opens from the sidebar with a heading and the unified list', async () => {
    await page.getByRole('navigation', { name: 'Workspace' }).getByRole('button', { name: 'Contacts' }).click();
    await expect(page.getByRole('heading', { level: 1, name: 'Contacts' })).toHaveCount(1);
    await expect(row('Ada Lovelace')).toHaveCount(1);
  });

  await step('A Google book without read consent shows a bar that asks for it', async () => {
    const bar = page.getByRole('status').filter({ hasText: 'needs your permission' });
    await expect(bar).toContainText('alex@studio-example.com');
    await bar.getByRole('button', { name: 'Allow reading' }).click();
    await expect(page.getByRole('status').filter({ hasText: 'needs your permission' })).toHaveCount(0);
  });

  await step('A person in two books is one row that shows both origins', async () => {
    const ada = row('Ada Lovelace');
    await expect(ada).toContainText('Locale');
    await expect(ada).toContainText('Google · alex@studio-example.com');
    await expect(page.getByRole('list', { name: 'Contacts' }).getByRole('button', { name: /Ada L\./ })).toHaveCount(0);
  });

  await step('Search narrows the list and the origin filter keeps only that book', async () => {
    await page.getByRole('searchbox', { name: 'Search contacts' }).fill('bea');
    await expect(row('Bea Verdi')).toHaveCount(1);
    await expect(row('Ada Lovelace')).toHaveCount(0);
    await page.getByRole('searchbox', { name: 'Search contacts' }).fill('');
    await page.getByRole('combobox', { name: 'Filter by address book' }).click();
    await page.getByRole('option', { name: 'Google · alex.home@example.org', exact: true }).click();
    await expect(row('Mario Rossi')).toHaveCount(1);
    await expect(row('Bea Verdi')).toHaveCount(0);
    await page.getByRole('combobox', { name: 'Filter by address book' }).click();
    await page.getByRole('option', { name: 'All address books' }).click();
  });

  await step('A new contact cannot be saved without choosing an address book', async () => {
    await page.getByRole('button', { name: 'New contact' }).click();
    const dialog = page.getByRole('dialog', { name: 'New contact' });
    await dialog.getByLabel('Name', { exact: true }).fill('Carla Neri');
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog.getByRole('alert')).toContainText('Choose an address book');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('combobox', { name: 'Address book', exact: true }).click();
    await page.getByRole('option', { name: 'Locale' }).click();
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(row('Carla Neri')).toHaveCount(1);
  });

  await step('The form warns about a similar contact before saving', async () => {
    await page.getByRole('button', { name: 'New contact' }).click();
    const dialog = page.getByRole('dialog', { name: 'New contact' });
    await dialog.getByLabel('Email 1', { exact: true }).fill('ada@example.test');
    await dialog.getByLabel('Email 1', { exact: true }).blur();
    await expect(dialog.getByRole('status')).toContainText('Ada Lovelace');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);
  });

  await step('A contact seen only in mail is read-only and can be added to a book', async () => {
    await row('Maria Rossi').click();
    await expect(page.getByRole('heading', { level: 2, name: 'Maria Rossi' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Edit' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Add to contacts' }).click();
    const dialog = page.getByRole('dialog', { name: 'New contact' });
    await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('Maria Rossi');
    await expect(dialog.getByRole('combobox', { name: 'Address book', exact: true })).toContainText('Locale');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
  });

  await step('Duplicates panel offers exactly three answers per group', async () => {
    await expect(duplicatesButton()).toContainText('2');
    await duplicatesButton().click();
    const panel = page.getByRole('region', { name: 'Duplicates' });
    const groups = panel.getByRole('article');
    await expect(groups).toHaveCount(2);
    for (const group of await groups.all()) {
      await expect(group.getByRole('button')).toHaveCount(3);
      await expect(group.getByRole('button', { name: 'Same person' })).toBeVisible();
      await expect(group.getByRole('button', { name: 'Merge…' })).toBeVisible();
      await expect(group.getByRole('button', { name: 'Different people' })).toBeVisible();
    }
  });

  await step('Same person silences the signal without removing anything', async () => {
    const panel = page.getByRole('region', { name: 'Duplicates' });
    await panel.getByRole('article').filter({ hasText: 'Ada' }).getByRole('button', { name: 'Same person' }).click();
    await expect(panel.getByRole('article')).toHaveCount(1);
    await expect(duplicatesButton()).toContainText('1');
    await page.getByRole('button', { name: 'Close duplicates' }).click();
    await expect(row('Ada Lovelace')).toContainText('Google · alex@studio-example.com');
  });

  await step('Merge asks where to keep the contact, previews each book and blocks zero books', async () => {
    await duplicatesButton().click();
    await page.getByRole('region', { name: 'Duplicates' }).getByRole('article').getByRole('button', { name: 'Merge…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Merge contacts' });
    const confirm = dialog.getByRole('button', { name: /^Merge/ }).last();
    await dialog.getByRole('button', { name: 'Only in one' }).click();
    await expect(dialog.getByRole('checkbox', { checked: true })).toHaveCount(1);
    await dialog.locator('[data-scope="checkbox"][data-part="root"][data-state="checked"]').click();
    await expect(confirm).toBeDisabled();
    await tick(dialog, 'Locale').click();
    await expect(dialog.getByRole('checkbox', { name: 'Locale' })).toBeChecked();
    const preview = dialog.getByRole('list', { name: 'What will happen' });
    await expect(preview).toContainText('Locale');
    await expect(preview).toContainText('will be removed');
    await expect(confirm).toContainText('delete');
    await confirm.click();
    await expect(dialog).toHaveCount(0);
    await expect(row('Mario Rossi')).toHaveCount(1);
    await expect(row('Mario Rossi')).not.toContainText('Google');
    await expect(duplicatesButton()).toHaveCount(0);
  });

  await step('Undo merge brings the removed copy back and the duplicate signal returns', async () => {
    const banner = page.getByRole('status').filter({ hasText: 'Contacts merged' });
    await expect(banner).toBeVisible();
    await banner.getByRole('button', { name: 'Undo merge' }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Contacts merged' })).toHaveCount(0);
    await expect(row('Mario Rossi')).toHaveCount(2);
    await expect(page.getByRole('list', { name: 'Contacts' }).getByRole('button', { name: /Mario Rossi.*Google/s })).toHaveCount(1);
    await expect(duplicatesButton()).toHaveCount(1);
  });

  await step('Delete asks per book and Hide removes a mail-only contact from the list', async () => {
    await row('Bea Verdi').click();
    await page.getByRole('button', { name: 'Delete…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Delete contact' });
    await tick(dialog, 'Locale').click();
    await dialog.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(row('Bea Verdi')).toHaveCount(0);
    await row('Sofia').click();
    await expect(page.getByRole('button', { name: 'Delete…' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Hide' }).click();
    await expect(row('Sofia')).toHaveCount(0);
  });

  await step('Show hidden lists a hidden contact and Show again restores it for good', async () => {
    await tick(page, 'Show hidden').click();
    await expect(page.getByRole('checkbox', { name: 'Show hidden' })).toBeChecked();
    await expect(row('Sofia')).toHaveCount(1);
    await row('Sofia').click();
    await expect(page.getByRole('button', { name: 'Hide' })).toHaveCount(0);
    await page.getByRole('button', { name: 'Show again' }).click();
    await tick(page, 'Show hidden').click();
    await expect(page.getByRole('checkbox', { name: 'Show hidden' })).not.toBeChecked();
    await expect(row('Sofia')).toHaveCount(1);
  });

  await step('The composer completes recipients from the unified book', async () => {
    await page.getByRole('button', { name: /^New message/ }).click();
    const to = page.getByRole('combobox', { name: 'To', exact: true });
    await to.fill('mar');
    const options = page.getByRole('listbox', { name: 'Suggestions from your contacts' });
    await expect(options.getByRole('option', { name: /Mario Rossi/ }).first()).toBeVisible();
    await options.getByRole('option', { name: /Mario Rossi/ }).first().click();
    // The pick is a chip that carries the whole entry, and the box is empty for the next address.
    await expect(page.locator('.composer-panel [data-part="item-preview"]')).toHaveAttribute('title', /^Mario Rossi <[^>]+@acme\.test>$/);
    await expect(to).toHaveValue('');
    await to.fill('x');
    await expect(options).toHaveCount(0);
  });

  await step('Settings has a Contacts section that saves the priority order', async () => {
    await page.goto(base);
    await page.locator('.compose-main').waitFor();
    await page.getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Contacts' }).click();
    await expect(page.getByRole('heading', { level: 2, name: 'Contacts' })).toBeVisible();
    const order = page.getByRole('list', { name: 'Order of address books' });
    await expect(order.getByRole('listitem').first()).toContainText('Locale');
    await order.getByRole('button', { name: /^Move down: Locale/ }).click();
    await expect(order.getByRole('listitem').first()).not.toContainText('Locale');
    await page.getByLabel('Address names never treated as one person (comma-separated)').fill('ciao, info');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Saved' })).toBeVisible();
  });

  assert.deepEqual(errors, [], `Page errors: ${errors.join('; ')}`);
  console.log(`${results.length} contacts steps passed`);
} finally {
  await browser.close();
}
