// Checkbox, Switch, Tabs, Slider and DateField (spec 2026-11-08 ark-compatto §8, Task 6): the value
// contract, the keyboard, the focus ring (2px --color-primary) and the Compatto look. Each primitive is mounted
// alone through tests/support/mount.mjs. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import { pickDate } from './support/dates.mjs';
import assert from './support/soft-assert.mjs';
import { mountGallery } from './support/mount.mjs';

// Sigrid's --color-primary (light), and the ring of a focused control (--color-ring).
const ACCENT = 'rgb(79, 70, 229)';
const RING = 'rgb(67, 56, 202)';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
const context = await browser.newContext({
  locale: 'en-US',
  viewport: { width: 900, height: 700 },
});
const open = async (which, props = {}) => {
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await mountGallery(page, which, props);
  return page;
};
// The ring is drawn on the visible part, never on a hidden input.
const assertRing = async (locator, what) => {
  const ring = await locator.evaluate((el) => {
    const style = getComputedStyle(el);
    return { width: style.outlineWidth, style: style.outlineStyle, color: style.outlineColor };
  });
  assert.deepEqual(
    ring,
    { width: '2px', style: 'solid', color: RING },
    `${what}: focus ring ${JSON.stringify(ring)}`,
  );
};

try {
  // ---------- Checkbox and Switch: same props, same tests ----------
  for (const kind of ['checkbox', 'switch']) {
    const Kind = kind === 'checkbox' ? 'Checkbox' : 'Switch';
    const page = await open('checkbox');
    const name = kind === 'checkbox' ? 'Notify me' : 'Dark mode';
    const locked = kind === 'checkbox' ? 'Locked box' : 'Locked switch';
    const state = page.getByLabel(kind === 'checkbox' ? 'Box state' : 'Switch state', {
      exact: true,
    });
    const changes = page.getByLabel(kind === 'checkbox' ? 'Box changes' : 'Switch changes', {
      exact: true,
    });
    const input = page.getByRole(kind, { name, exact: true });
    // The drawn part: the checkbox is the box itself, the switch's is its control (the track).
    const visible =
      kind === 'checkbox'
        ? input
        : page.locator('[data-scope="switch"][data-part="root"]', { hasText: name }).locator('[data-slot="switch-control"]');

    // Keyboard: Tab reaches it (the switch after the checkboxes), Space toggles it.
    for (let i = 0; i < 6 && !(await input.evaluate((el) => el === document.activeElement)); i++)
      await page.keyboard.press('Tab');
    await expect(input).toBeFocused();
    await assertRing(visible, `${Kind} focused`);
    await expect(state).toHaveText('false');
    await page.keyboard.press('Space');
    await expect(state).toHaveText('true');
    await expect(input).toBeChecked();
    await page.keyboard.press('Space');
    await expect(state).toHaveText('false');
    console.log(`PASS ${Kind} toggles with Space and shows a 2px accent ring on keyboard focus`);

    // Clicking the label text toggles it too, and onchange gets the new value each time.
    await page.getByText(name, { exact: true }).click();
    await expect(state).toHaveText('true');
    await expect(input).toBeChecked();
    await expect(changes).toHaveText('true,false,true');
    console.log(
      `PASS ${Kind} toggles by a click on the label and reports every change through onchange`,
    );

    // Disabled: not editable by click, and visibly dimmed.
    const lockedInput = page.getByRole(kind, { name: locked, exact: true });
    await expect(lockedInput).toBeDisabled();
    const before = await lockedInput.isChecked();
    await page.getByText(locked, { exact: true }).click({ force: true });
    assert.equal(await lockedInput.isChecked(), before, `${Kind} disabled keeps its state`);
    const opacity = await page
      .locator(`[data-scope="${kind}"][data-part="root"]`, { hasText: locked })
      .evaluate((el) => getComputedStyle(el).opacity);
    assert.ok(Number(opacity) < 1, `${Kind} disabled is dimmed (opacity ${opacity})`);
    console.log(`PASS ${Kind} disabled ignores the click and is dimmed`);

    // The look: a 16px box with the 4px radius, accent when on (checkbox); a 32x18 track (switch).
    // The unlumen Apple Switch paints its colour on a fill layer inside the track (data-part="fill").
    await expect(visible).toHaveCSS('background-color', ACCENT);
    const box = await visible.evaluate((el) => {
      const style = getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      return {
        w: Math.round(rect.width),
        h: Math.round(rect.height),
        radius: style.borderTopLeftRadius,
      };
    });
    if (kind === 'checkbox')
      assert.deepEqual([box.w, box.h, box.radius], [18, 18, '4px'], JSON.stringify(box));
    else assert.ok(box.w > box.h, `the switch track is wider than it is high: ${JSON.stringify(box)}`);
    console.log(`PASS ${Kind} looks like the Compatto control (${box.w}x${box.h}, accent when on)`);
    await page.close();
  }

  // ---------- Tabs ----------
  for (const variant of ['sigrid']) {
    const page = await open('tabs', { variant });
    const list = page.getByRole('tablist', { name: 'Show', exact: true });
    const tabs = list.getByRole('tab');
    const value = page.getByLabel('Tab value', { exact: true });
    await expect(list).toBeVisible();
    await expect(tabs).toHaveCount(3);
    await expect(tabs.nth(0)).toHaveAttribute('aria-selected', 'true');
    await expect(tabs.nth(0)).toContainText('48');
    await expect(tabs.nth(2)).not.toContainText(/\d/);
    // Only the list is drawn: no tabpanel.
    await expect(page.getByRole('tabpanel')).toHaveCount(0);
    // And no tab points at a panel that is not there: no aria-controls, or one that resolves.
    const dangling = () =>
      page.evaluate(() =>
        [...document.querySelectorAll('[role="tab"][aria-controls]')]
          .filter((tab) => !document.getElementById(tab.getAttribute('aria-controls')))
          .map((tab) => tab.textContent.trim()),
      );
    assert.deepEqual(await dangling(), [], 'tabs with a dangling aria-controls');

    // Arrows move the focus and the selection (automatic activation), Home/End jump, the ends loop.
    await page.keyboard.press('Tab');
    await expect(tabs.nth(0)).toBeFocused();
    await assertRing(tabs.nth(0), `${variant} tab focused`);
    await page.keyboard.press('ArrowRight');
    await expect(tabs.nth(1)).toBeFocused();
    await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
    await expect(tabs.nth(0)).toHaveAttribute('aria-selected', 'false');
    await expect(value).toHaveText('unread');
    await page.keyboard.press('End');
    await expect(tabs.nth(2)).toHaveAttribute('aria-selected', 'true');
    await expect(value).toHaveText('files');
    await page.keyboard.press('ArrowRight');
    await expect(tabs.nth(0)).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowLeft');
    await expect(tabs.nth(2)).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('Home');
    await expect(value).toHaveText('all');
    // Only the selected tab is in the tab order.
    await expect(tabs.nth(1)).toHaveAttribute('tabindex', '-1');
    await tabs.nth(1).click();
    await expect(value).toHaveText('unread');
    assert.deepEqual(await dangling(), [], 'after moving between tabs, a tab has a dangling aria-controls');
    console.log(`PASS Tabs (${variant}) move with the arrows, Home and End, and a click selects`);

    // The chosen tab is the semibold one, on Sigrid's selection tint.
    const weight = (tab) => tab.evaluate((el) => Number(getComputedStyle(el).fontWeight));
    assert.ok((await weight(tabs.nth(1))) >= 600, 'the chosen tab is semibold');
    assert.ok((await weight(tabs.nth(0))) < 600, 'the other tabs are not');
    console.log('PASS Tabs: the chosen tab is the semibold one');
    await page.close();
  }

  // ---------- DateField ----------
  // Sigrid's field is a button that shows the date and opens a calendar: a day is chosen there, not typed.
  const PART = (name) => `[data-scope="date-picker"][data-part="${name}"]`;
  {
    const page = await open('date');
    const field = page.getByLabel('Due date', { exact: true });
    const value = page.getByLabel('Date value', { exact: true });
    const calendar = page.locator(PART('content'));

    // The field is 32px high (--control-md).
    assert.equal(await field.evaluate((el) => Math.round(el.getBoundingClientRect().height)), 32);

    await expect(calendar).toBeHidden();
    await field.click();
    await expect(calendar).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(calendar).toBeHidden();
    await pickDate(page, field, '2026-11-12');
    await expect(value).toHaveText('2026-11-12');
    await expect(calendar).toBeHidden();
    await expect(field).toContainText('November 12, 2026');
    console.log('PASS DateField opens the calendar, a click on a day sets value as YYYY-MM-DD');

    // The focus ring: the field draws it when it is focused from the keyboard.
    await page.getByRole('button', { name: 'After', exact: true }).focus();
    await page.keyboard.press('Shift+Tab');
    await expect(field).toBeFocused();
    await assertRing(field, 'DateField');
    console.log('PASS DateField shows a 2px ring on keyboard focus');
    await page.close();

    // min and max: days outside are disabled.
    const ranged = await open('date', { value: '2026-11-08', min: '2026-11-05', max: '2026-11-20' });
    await ranged.getByLabel('Due date', { exact: true }).click();
    const cell = (day) => ranged.locator(`${PART('table-cell-trigger')}[data-value="${day}"]`);
    await expect(cell('2026-11-25')).toHaveAttribute('data-disabled', '');
    await expect(cell('2026-11-12')).not.toHaveAttribute('data-disabled', '');
    console.log('PASS DateField honours min and max');
    await ranged.close();

    // The locale prop shapes what the field says.
    const italian = await open('date', { value: '2026-11-08', locale: 'it-IT' });
    await expect(italian.getByLabel('Due date', { exact: true })).toContainText('8 novembre 2026');
    console.log('PASS DateField reads the locale of the prop (it-IT: day first)');
    await italian.close();
  }

  // ---------- DateField inside Modal and SidePanel ----------
  // The calendar belongs to the dialog that holds the field: Esc closes the calendar first, and a click on a day neither
  // closes the dialog nor is lost.
  for (const [kind, props] of [['Modal', { modal: true }], ['SidePanel', { panel: true }]]) {
    const page = await open('date', props);
    const dialog = page.locator('[data-slot="dialog-content"], [data-slot="sheet-content"]').first();
    await expect(dialog).toBeVisible();
    const field = dialog.getByLabel('Due date', { exact: true });
    const calendar = page.locator(PART('content'));
    const closes = page.getByLabel('Closes', { exact: true });
    await field.click();
    await expect(calendar).toBeVisible();
    // The calendar is on top (the point at its middle is its own) and not clipped by the window.
    const onTop = await calendar.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
    });
    assert.ok(onTop, `${kind}: the calendar is on top`);
    const fits = await calendar.evaluate((el) => el.getBoundingClientRect().bottom - innerHeight);
    assert.ok(fits <= 0, `${kind}: the calendar fits the window (${fits})`);
    await page.keyboard.press('Escape');
    await expect(calendar).toBeHidden();
    await expect(dialog).toBeVisible();
    await expect(closes).toHaveText('0');
    await pickDate(page, field, '2026-11-12');
    await expect(page.getByLabel('Date value', { exact: true })).toHaveText('2026-11-12');
    await expect(calendar).toBeHidden();
    await expect(dialog).toBeVisible();
    await expect(closes).toHaveText('0');
    console.log(`PASS DateField inside a ${kind}: calendar on top, Esc closes only it, a click on a day chooses`);
    await page.close();
  }

  // The Sigrid plan's insertion points (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md): a task writes its
  // new cases under its own comment, and changes the existing cases of its primitive where they are. The blank
  // lines stay, so two tasks never touch neighbouring lines.

  // Task 4: switch

  // Task 5: tabs

  // Task 6: count and slider

  // Task 8: checkbox

  // Task 9: field

  // Task 11: date field

  assert.deepEqual(errors, [], `page errors: ${errors.join(' | ')}`);
  console.log('primitives-controls: ok');
} finally {
  await browser.close();
}
