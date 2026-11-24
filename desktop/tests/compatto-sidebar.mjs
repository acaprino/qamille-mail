// The sidebar in the Compatto look: it is Sigrid's (its Sidebar, rows, rail and foot), so this reads what the app owes it as
// computed values at 1440x900 in en-US, and behaviour through role and name: the column's paper and its line, the rows of the
// folders and of the workspace, the current row and its count, the rail, and the user at the foot with the user menu.
// Synthetic data only (the preview's sample workspace).
import { expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { openApp, css, box } from './support/compatto.mjs';

const { browser, page, errors } = await openApp();
const token = (name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }, name);

try {
  const sidebar = page.getByRole('navigation', { name: 'Sidebar' });
  const folders = sidebar.getByRole('list', { name: 'Folders' });
  const tools = sidebar.getByRole('list', { name: 'Workspace' });

  // (a) The column is Sigrid's: its own paper, and a line against the work.
  const column = page.locator('[data-slot="sidebar"]');
  assert.equal(await css(column, 'background-color'), await token('--color-sidebar-paper'), 'the sidebar paper is Sigrid\'s');
  assert.equal(await css(column, 'border-right-width'), '1px', 'a line against the work');
  console.log('PASS The sidebar is Sigrid\'s column: its paper and a 1px line against the work');

  // (b) New message: a row of the sidebar that shows the key N.
  const compose = sidebar.getByRole('button', { name: 'New message' });
  await expect(compose.locator('kbd')).toHaveText('N');
  console.log('PASS New message is a row of the sidebar and shows N');

  // (c) The rows of a group are the same height, and not shorter than a dense control.
  for (const group of [folders, tools]) {
    const rows = group.getByRole('button');
    const count = await rows.count();
    assert.ok(count >= 3, 'the group has rows');
    const first = (await box(rows.first())).height;
    assert.ok(first >= 28, `a row is at least 28 px high (${first})`);
    for (let i = 1; i < count; i++) assert.equal((await box(rows.nth(i))).height, first, `row ${i} is as high as the first`);
  }
  console.log('PASS The rows of the folders and of the workspace are all as high, and at least 28 px');

  // (d) The current folder is marked, and its count is shown.
  const inbox = folders.getByRole('button', { name: /^Inbox/ });
  await expect(inbox).toHaveAttribute('aria-current', 'page');
  await expect(inbox.locator('[data-slot="count"]')).toBeVisible();
  assert.equal(await folders.locator('[aria-current="page"]').count(), 1, 'one current row');
  console.log('PASS The current folder is marked and shows its count');

  // (e) The foot: the user, with the user menu at its end (Settings and the activity history).
  const foot = column.locator('[data-slot="sidebar-row"]');
  await expect(foot).toContainText('Alex');
  const menu = column.getByRole('button', { name: 'User menu' });
  await menu.click();
  const items = await page.getByRole('menuitem').allInnerTexts();
  assert.deepEqual(items.map((text) => text.trim()), ['Settings', 'Activity history']);
  await page.keyboard.press('Escape');
  await expect(menu).toBeFocused();
  console.log('PASS The user is at the foot, and the user menu offers Settings and the activity history');

  // (f) The rail: the column shrinks to icons, the header and the foot go, the rows keep their names.
  const collapse = column.getByRole('button', { name: 'Collapse the sidebar' });
  await expect(collapse).toBeVisible();
  await collapse.click();
  await expect(column).toHaveAttribute('data-collapsed');
  assert.ok((await box(column)).width <= 76, 'the rail is narrow');
  await expect(column.getByRole('combobox', { name: 'Account' })).toHaveCount(0);
  await expect(column.getByRole('button', { name: 'User menu' })).toHaveCount(0);
  await expect(folders.getByRole('button', { name: /^Inbox/ })).toBeVisible();
  await column.getByRole('button', { name: 'Expand the sidebar' }).click();
  await expect(column).not.toHaveAttribute('data-collapsed');
  await expect(column.getByRole('combobox', { name: 'Account' })).toBeVisible();
  console.log('PASS The rail drops the header and the foot, keeps the rows by name, and widens again');

  assert.deepEqual(errors, [], `Page errors: ${errors.join('; ')}`);
} finally {
  await browser.close();
}
