// Task 15 of the Ark Compatto plan: Contacts, measured in computed values (rows of --control-md,
// hairlines --color-paper-2, Ark Select and Checkbox, Modal md/sm) and driven by role and accessible name.
// Sample data only. Run: node scripts/test-browser.mjs compatto-contacts
import { expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { css, openApp } from './support/compatto.mjs';

const { browser, page, errors } = await openApp();
const results = [];
async function step(name, work) {
  try {
    await work();
  } catch (error) {
    // A step that fails is told, and the next ones still run: one pass shows every difference from Sigrid, not the first.
    process.exitCode = 1;
    console.log(`FAIL STEP ${name}: ${String(error?.message ?? error).split('\n').slice(0, 8).join(' | ').slice(0, 700)}`);
    return;
  }
  results.push(name);
  console.log(`PASS ${name}`);
}
const token = (name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }, name);
// Layout height (a dialog is still scaling in when it opens, which would skew a bounding box).
const height = (locator) => locator.first().evaluate((node) => node.offsetHeight);
const list = () => page.getByRole('list', { name: 'Contacts' });
const row = (name) => list().getByRole('button', { name: new RegExp(name) });
const dialogContent = (name) => page.getByRole('dialog', { name });
const trigger = (scope, name) => scope.getByRole('combobox', { name, exact: true });

try {
  await page.getByRole('list', { name: 'Workspace' }).getByRole('button', { name: 'Contacts' }).click();
  await expect(row('Ada Lovelace')).toHaveCount(1);
  const soft = await token('--color-paper-2');
  const controlHeight = 32;

  await step('List rows are one --control-md tall and separated by --color-paper-2 hairlines', async () => {
    const rows = await list().getByRole('button').all();
    assert.ok(rows.length > 3, 'sample data has several contacts');
    for (const one of rows) assert.equal((await height(one)), controlHeight);
    assert.equal(await css(list().getByRole('listitem').nth(1), 'border-top-color'), soft);
    assert.equal(await css(list().getByRole('listitem').nth(1), 'border-top-width'), '1px');
  });

  await step('Toolbar controls are --control-md and the filters are Ark Select and Checkbox', async () => {
    await expect(page.locator('.contacts-page select')).toHaveCount(0);
    const filters = page.locator('.contacts-filters');
    const book = trigger(filters, 'Filter by address book');
    assert.equal(await book.getAttribute('data-slot'), 'select-trigger');
    assert.equal((await height(book)), controlHeight);
    assert.equal((await height(page.getByRole('searchbox', { name: 'Search contacts' }))), controlHeight);
    assert.equal((await height(page.getByRole('button', { name: 'New contact' }))), controlHeight);
    const control = filters.locator('[data-slot="checkbox"]');
    assert.ok((await control.evaluate((node) => node.offsetWidth)) >= 16, 'the box is at least 16px');
    await filters.getByText('Show hidden').click();
    await expect(page.getByRole('checkbox', { name: 'Show hidden' })).toBeChecked();
    await filters.getByText('Show hidden').click();
    await expect(page.getByRole('checkbox', { name: 'Show hidden' })).not.toBeChecked();
    await book.click();
    await page.getByRole('option', { name: 'All address books' }).waitFor();
    assert.ok((await page.getByRole('option').count()) >= 2);
    await page.keyboard.press('Escape');
  });

  await step('The inline detail takes rows of --control-md with --color-paper-2 hairlines', async () => {
    await row('Ada Lovelace').click();
    const facts = page.locator('.contact-facts');
    const first = facts.locator('dt').first();
    assert.ok((await height(first)) >= controlHeight);
    assert.equal(await css(first, 'border-top-color'), soft);
    const source = page.getByRole('list', { name: 'Sources' }).getByRole('listitem').first();
    assert.ok((await height(source)) >= controlHeight);
    assert.equal(await css(source, 'border-top-color'), soft);
  });

  await step('The contact card is a Modal md with an Ark Select for the address book', async () => {
    await page.getByRole('button', { name: 'New contact' }).click();
    const dialog = dialogContent('New contact');
    assert.equal(await dialog.evaluate((node) => node.offsetWidth), 560);
    await expect(dialog.locator('select')).toHaveCount(0);
    const book = trigger(dialog, 'Address book');
    assert.equal((await height(book)), controlHeight);
    assert.equal((await height(dialog.getByLabel('Name', { exact: true }))), controlHeight);
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog.getByRole('alert')).toContainText('Choose an address book');
    await book.click();
    await page.getByRole('option', { name: 'Local' }).click();
    await expect(book).toContainText('Local');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);
  });

  await step('Duplicates rows are --control-md with --color-paper-2 hairlines', async () => {
    await page.getByRole('button', { name: /^Duplicates/ }).click();
    const panel = page.getByRole('region', { name: 'Duplicates' });
    const member = panel.getByRole('article').first().getByRole('listitem').first();
    assert.ok((await height(member)) >= controlHeight);
    const second = panel.getByRole('article').first().getByRole('listitem').nth(1);
    assert.equal(await css(second, 'border-top-color'), soft);
    assert.equal(await css(panel.getByRole('article').nth(1), 'border-top-color'), soft);
  });

  await step('Merge is a Modal md whose book choices are Ark Checkboxes', async () => {
    await page.getByRole('region', { name: 'Duplicates' }).getByRole('article').first().getByRole('button', { name: 'Merge…' }).click();
    const dialog = dialogContent('Merge contacts');
    const books = dialog.locator('.merge-books');
    assert.ok((await books.locator('[data-slot="checkbox"]').count()) >= 2);
    await dialog.getByRole('button', { name: 'Only in one' }).click();
    await expect(dialog.getByRole('checkbox', { checked: true })).toHaveCount(1);
    const locale = dialog.getByRole('checkbox', { name: 'Local' });
    const before = await locale.isChecked();
    await books.getByText('Local').click();
    await expect(locale).toBeChecked({ checked: !before });
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);
    await page.getByRole('button', { name: 'Close duplicates' }).click();
  });

  await step('Delete contact is a Modal sm with an Ark Checkbox per book', async () => {
    await row('Bea Verdi').click();
    await page.getByRole('button', { name: 'Delete…' }).click();
    const dialog = dialogContent('Delete contact');
    assert.equal(await dialog.evaluate((node) => node.offsetWidth), 420);
    await expect(dialog.locator('[data-slot="checkbox"]')).toHaveCount(1);
    await dialog.getByText('Local').click();
    await expect(dialog.getByRole('button', { name: 'Delete', exact: true })).toBeEnabled();
    await dialog.getByRole('button', { name: 'Cancel' }).click();
  });

  await step('Settings: the Contacts section uses Ark Checkbox and --control-md rows', async () => {
    await page.goto(process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420');
    await page.locator('.compose-main').waitFor();
    await page.getByRole('button', { name: /^Settings/ }).click();
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Contacts' }).click();
    const books = page.getByRole('list', { name: 'Connected address books' });
    await expect(books.locator('[data-slot="checkbox"]').first()).toBeVisible();
    const order = page.getByRole('list', { name: 'Order of address books' });
    assert.ok((await height(order.getByRole('listitem').first())) >= controlHeight);
    assert.equal(await css(order.getByRole('listitem').nth(1), 'border-top-color'), soft);
  });

  assert.deepEqual(errors, [], `Page errors: ${errors.join('; ')}`);
  console.log(`${results.length} compatto-contacts steps passed`);
} finally {
  await browser.close();
}
