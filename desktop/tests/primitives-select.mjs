// Select primitive (spec 2026-11-08 ark-compatto section 8, Task 5): an Ark Select with a button
// (role combobox, --control-md) and a listbox in a portal. Mounted alone through
// tests/support/mount.mjs; the gallery listens to window keydown like App.tsx does, through the
// same guard, so a key typed in the list is seen not to reach the page shortcuts. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { mountGallery } from './support/mount.mjs';

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
try {
  const context = await browser.newContext({ locale: 'en-US' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await mountGallery(page, 'select');

  const select = page.getByRole('combobox', { name: 'Language' });
  const listbox = page.getByRole('listbox');
  const value = page.locator('#value');
  const changes = page.locator('#changes');

  // (a) The button names itself by the label and shows the chosen item's label, not its value.
  await expect(select).toBeVisible();
  await expect(select).toContainText('Italiano');
  await expect(listbox).toHaveCount(0);
  const box = await select.boundingBox();
  assert.equal(Math.round(box.height), 32, 'the button is --control-md high');
  console.log('PASS The select shows the label of the chosen item');

  // aria-required and aria-invalid reach the button (a required form field says so to a screen reader).
  const needed = page.getByRole('combobox', { name: 'Needed' });
  await expect(needed).toHaveAttribute('aria-invalid', 'true');
  console.log('PASS aria-required and aria-invalid pass through to the button');

  // (b) Enter opens the list, arrows move, Enter chooses and updates the value.
  await page.locator('#before').focus();
  await page.keyboard.press('Tab');
  await expect(select).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(listbox).toBeVisible();
  await expect(listbox.getByRole('option')).toHaveCount(5);
  await expect.poll(() => listbox.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  // Only the chosen item carries the check mark.
  await expect(listbox.getByRole('option', { selected: true })).toHaveCount(1);
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(listbox).toBeHidden();
  await expect(select).toContainText('English');
  await expect(value).toHaveText('en');
  await expect(changes).toHaveText('1');
  await expect(select).toBeFocused();
  // Space opens it as well.
  await page.keyboard.press('Space');
  await expect(listbox).toBeVisible();
  console.log('PASS Enter and Space open the list, arrows move, Enter chooses and updates the value');

  // (c) Escape closes it and the focus is back on the button.
  await page.keyboard.press('Escape');
  await expect(listbox).toBeHidden();
  await expect(select).toBeFocused();
  await expect(value).toHaveText('en');
  console.log('PASS Escape closes the list and the focus stays on the button');

  // (d) A disabled item cannot be chosen, by keyboard or by click.
  await page.keyboard.press('Enter');
  await expect.poll(() => listbox.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  const deutsch = listbox.getByRole('option', { name: 'Deutsch' });
  await expect(deutsch).toHaveAttribute('data-disabled', '');
  await deutsch.click({ force: true });
  await expect(listbox).toBeVisible();
  await expect(value).toHaveText('en');
  await page.keyboard.press('Escape');
  await expect(listbox).toBeHidden();
  await page.keyboard.press('Enter');
  await expect.poll(() => listbox.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  assert.notEqual(await value.textContent(), 'de');
  console.log('PASS A disabled item is not selectable');

  // The popup sits on the toast layer (ruling R7), above Modal and SidePanel; the next test opens it
  // inside both. A disabled select is inert.
  await page.keyboard.press('Enter');
  await expect(listbox).toBeVisible();
  const layers = await page.evaluate(() => {
    const token = (name) => Number(getComputedStyle(document.documentElement).getPropertyValue(name));
    const z = (part) => Number(getComputedStyle(document.querySelector(`[data-scope="select"][data-part="${part}"]`)).zIndex);
    return { toast: token('--z-toast'), panel: token('--z-panel'), modal: token('--z-modal'), positioner: z('positioner'), content: z('content') };
  });
  assert.ok(layers.toast > layers.modal && layers.modal > layers.panel && layers.panel > 0, 'toast is above modal, modal above panel');
  assert.equal(layers.positioner, layers.toast, 'the list is at --z-toast');
  assert.equal(layers.content, layers.toast);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('combobox', { name: 'Spare' })).toBeDisabled();
  console.log('PASS The list is on the toast layer above modal and panel; a disabled select is inert');

  // Inside the real Modal (Ark Dialog, focus trapped, scrim) and inside a SidePanel the list must be
  // reachable: it opens, the arrows and Enter choose, Escape closes only the list (the modal and the
  // panel stay), a click on an option works, and the list is on top (the click lands on it).
  for (const [kind, props] of [['Modal', { modal: true }], ['SidePanel', { panel: true }]]) {
    const host = await context.newPage();
    host.on('pageerror', (error) => errors.push(error.message));
    await mountGallery(host, 'select', props);
    const dialog = host.locator('[data-slot="dialog-content"]');
    await expect(dialog).toBeVisible();
    const field = dialog.getByRole('combobox', { name: 'Language' });
    const options = host.getByRole('listbox');
    await field.click();
    await expect(options).toBeVisible();
    await expect(options).toBeFocused();
    // The list is not hidden behind the dialog: the point at its middle belongs to the list.
    const onTop = await options.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
    });
    assert.ok(onTop, `${kind}: the list is on top of the dialog`);
    await host.keyboard.press('ArrowDown');
    await host.keyboard.press('Enter');
    await expect(options).toBeHidden();
    await expect(field).toContainText('English');
    await expect(host.locator('#value')).toHaveText('en');
    await expect(field).toBeFocused();
    // Escape closes the list only; a second Escape reaches the dialog.
    await host.keyboard.press('Enter');
    await expect(options).toBeFocused();
    await host.keyboard.press('Escape');
    await expect(options).toBeHidden();
    await expect(dialog).toBeVisible();
    await expect(field).toBeFocused();
    await expect(host.locator('#closes')).toHaveText('0');
    // A click on an option chooses it and does not count as a click outside the dialog.
    await field.click();
    await expect(options).toBeVisible();
    await options.getByRole('option', { name: 'Español' }).click();
    await expect(options).toBeHidden();
    await expect(field).toContainText('Español');
    await expect(host.locator('#value')).toHaveText('es');
    await expect(dialog).toBeVisible();
    await expect(host.locator('#closes')).toHaveText('0');
    // The trap does not pull the focus away from the open list while the arrows move in it.
    await field.focus();
    await host.keyboard.press('Enter');
    await host.keyboard.press('ArrowDown');
    await host.keyboard.press('ArrowDown');
    await expect(options).toBeFocused();
    await host.keyboard.press('Escape');
    await expect(options).toBeHidden();
    await expect(dialog).toBeVisible();
    if (kind === 'Modal') {
      // After the list is closed Escape closes the modal as usual.
      await host.keyboard.press('Escape');
      await expect(host.locator('#closes')).toHaveText('1');
    }
    console.log(`PASS Inside a ${kind} the list opens on top, arrows and Enter choose, Escape closes only the list, a click chooses`);
  }

  // (e) Review Focus 3: with the list open, typing e / n / r searches by first letter and does not
  // fire the page shortcuts (N compose, R reply, E archive); the keys never leave the Select.
  const fresh = await context.newPage();
  fresh.on('pageerror', (error) => errors.push(error.message));
  await mountGallery(fresh, 'select');
  const trigger = fresh.getByRole('combobox', { name: 'Language' });
  await trigger.focus();
  await fresh.keyboard.press('Enter');
  await expect(fresh.getByRole('listbox')).toBeFocused();
  await fresh.keyboard.press('e');
  const inside = await fresh.evaluate(() => !!document.activeElement?.closest('[data-scope="select"]'));
  assert.ok(inside, 'the focus is still inside the Select after typing');
  await expect(fresh.getByRole('option', { name: 'English' })).toHaveAttribute('data-highlighted', '');
  await fresh.keyboard.press('Enter');
  await expect(fresh.locator('#value')).toHaveText('en');
  await expect(trigger).toContainText('English');
  await fresh.keyboard.press('Enter');
  await expect(fresh.getByRole('listbox')).toBeFocused();
  await fresh.keyboard.press('n');
  await fresh.keyboard.press('r');
  await fresh.keyboard.press('e');
  await expect(fresh.getByRole('listbox')).toBeVisible();
  await fresh.keyboard.press('Escape');
  await expect(trigger).toBeFocused();
  await expect(fresh.locator('#shortcuts')).toHaveText('0');
  // The same key outside any widget does reach the page handler: the counter works.
  await fresh.locator('#before').focus();
  await fresh.keyboard.press('e');
  await expect(fresh.locator('#shortcuts')).toHaveText('1');
  console.log('PASS Typing e, n, r in the open list searches by first letter and fires no page shortcut');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
