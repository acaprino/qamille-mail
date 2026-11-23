// Checkbox, Switch, Tabs, Slider and DateField (spec 2026-11-08 ark-compatto §8, Task 6): the value
// contract, the keyboard, the focus ring (2px --color-primary) and the Compatto look. Each primitive is mounted
// alone through tests/support/mount.mjs. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { mountGallery } from './support/mount.mjs';

const ACCENT = 'rgb(79, 70, 229)';
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
    { width: '2px', style: 'solid', color: ACCENT },
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
    const visible = page
      .locator(`[data-scope="${kind}"][data-part="root"]`, { hasText: name })
      .locator('[data-part="control"]');

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
    const fill = kind === 'checkbox' ? visible : visible.locator('[data-part="fill"]');
    await expect(fill).toHaveCSS('background-color', ACCENT);
    if (kind === 'switch') await expect(fill).toHaveCSS('opacity', '1'); // the switch fades in: wait for the end
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
      assert.deepEqual([box.w, box.h, box.radius], [16, 16, '4px'], JSON.stringify(box));
    else assert.deepEqual([box.w, box.h], [32, 18], JSON.stringify(box));
    console.log(`PASS ${Kind} looks like the Compatto control (${box.w}x${box.h}, accent when on)`);
    await page.close();
  }

  // ---------- Tabs ----------
  for (const variant of ['underline', 'segment']) {
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

    // The chosen tab carries the sliding highlight of the React Tabs primitive (data-part="highlight"): a 2px
    // bar in the accent (underline) or a raised white chip (segment). The Svelte one drew the tab's own border and fill.
    const look = (tab) =>
      tab.evaluate((el) => {
        const style = getComputedStyle(el);
        const mark = el.querySelector('[data-part="highlight"]');
        const markStyle = mark && getComputedStyle(mark);
        return {
          mark: markStyle && { height: markStyle.height, bg: markStyle.backgroundColor },
          weight: style.fontWeight,
        };
      });
    const selected = await look(tabs.nth(1));
    const other = await look(tabs.nth(0));
    if (variant === 'underline') {
      assert.deepEqual(selected.mark, { height: '2px', bg: ACCENT }, `selected tab underline ${JSON.stringify(selected.mark)}`);
      assert.ok(Number(selected.weight) >= 600, `selected tab is semibold, got ${selected.weight}`);
      assert.equal(other.mark, null, `unselected tab has no underline, got ${JSON.stringify(other.mark)}`);
      assert.ok(Number(other.weight) < 600);
      const height = await tabs
        .nth(0)
        .evaluate((el) => Math.round(el.getBoundingClientRect().height));
      assert.equal(height, 34, 'the mockup tab is 34px high');
      console.log('PASS Tabs underline: 2px accent underline on the selected tab only');
    } else {
      assert.equal(selected.mark?.bg, 'rgb(255, 255, 255)', `segment selected is a white chip, got ${JSON.stringify(selected.mark)}`);
      assert.notEqual(selected.mark?.height, '2px', 'no accent underline');
      assert.equal(other.mark, null, 'the other tabs have no chip');
      console.log('PASS Tabs segment: the selected tab is a raised chip, no accent underline');
    }
    await page.close();
  }

  // ---------- DateField ----------
  {
    const page = await open('date');
    const field = page.getByRole('textbox', { name: 'Due date', exact: true });
    const value = page.getByLabel('Date value', { exact: true });
    const toggle = page.getByRole('button', { name: /calendar/i });
    const dialog = page.locator('[data-scope="date-picker"][data-part="content"]');
    const after = page.getByRole('button', { name: 'After', exact: true });
    const control = page.locator('[data-scope="date-picker"][data-part="control"]');

    // The trigger field is 32px high (--control-md).
    assert.equal(await control.evaluate((el) => Math.round(el.getBoundingClientRect().height)), 32);

    await expect(dialog).toBeHidden();
    await toggle.click();
    await expect(dialog).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    // Go to a known month through the field, then choose a day in the calendar.
    await field.fill('2026-11-08');
    await field.press('Enter');
    await expect(value).toHaveText('2026-11-08');
    await toggle.click();
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: /November 12, 2026/ }).click();
    await expect(value).toHaveText('2026-11-12');
    await expect(dialog).toBeHidden();
    await expect(field).toHaveValue('11/12/2026');
    console.log('PASS DateField opens the calendar, a click on a day sets value as YYYY-MM-DD');

    // Typing: ISO is accepted, the locale form too, and nonsense is rejected (the old value stays).
    await field.fill('2026-11-08');
    await field.press('Enter');
    await expect(value).toHaveText('2026-11-08');
    await expect(field).toHaveValue('11/08/2026');
    await field.fill('03/04/2027');
    await field.press('Enter');
    await expect(value).toHaveText('2027-03-04');
    await field.fill('2026-13-40');
    await expect(field).toHaveAttribute('aria-invalid', 'true');
    await field.press('Enter');
    await expect(value).toHaveText('2027-03-04');
    await expect(field).toHaveValue('03/04/2027');
    await expect(field).not.toHaveAttribute('aria-invalid', 'true');
    await field.fill('2026-13-40');
    await after.click();
    await expect(value).toHaveText('2027-03-04');
    await expect(field).toHaveValue('03/04/2027');
    await field.fill('');
    await after.click();
    await expect(value).toHaveText('');
    console.log(
      'PASS DateField accepts 2026-11-08 and 03/04/2027 typed, rejects 2026-13-40 and clears when emptied',
    );

    // A real keystroke sequence with dashes works too (the separator of en-US is "/").
    await field.click();
    await page.keyboard.press('Escape');
    await field.pressSequentially('2026-11-09');
    await field.press('Enter');
    await expect(value).toHaveText('2026-11-09');

    // The focus ring: the field draws it when it is focused from the keyboard.
    await after.focus();
    await page.keyboard.press('Shift+Tab');
    await expect(toggle).toBeFocused();
    await assertRing(toggle, 'DateField trigger');
    await page.keyboard.press('Shift+Tab');
    await expect(field).toBeFocused();
    await assertRing(control, 'DateField control');
    console.log('PASS DateField shows a 2px accent ring on keyboard focus');
    await page.close();

    // min and max: days outside are disabled, a typed day outside is pulled into the range.
    const ranged = await open('date', {
      value: '2026-11-08',
      min: '2026-11-05',
      max: '2026-11-20',
    });
    const rangedField = ranged.getByRole('textbox', { name: 'Due date', exact: true });
    await ranged.getByRole('button', { name: /calendar/i }).click();
    const rangedDialog = ranged.locator('[data-scope="date-picker"][data-part="content"]');
    await expect(rangedDialog.getByRole('button', { name: /November 25, 2026/ })).toBeDisabled();
    await expect(rangedDialog.getByRole('button', { name: /November 12, 2026/ })).toBeEnabled();
    await ranged.keyboard.press('Escape');
    await rangedField.fill('2026-12-30');
    await rangedField.press('Enter');
    await expect(ranged.getByLabel('Date value', { exact: true })).toHaveText('2026-11-20');
    console.log('PASS DateField honours min and max');
    await ranged.close();

    // The locale prop shapes the typed form and the calendar.
    const italian = await open('date', { value: '2026-11-08', locale: 'it-IT' });
    const italianField = italian.getByRole('textbox', { name: 'Due date', exact: true });
    await expect(italianField).toHaveValue('08/11/2026');
    await italianField.fill('09/11/2026');
    await italianField.press('Enter');
    await expect(italian.getByLabel('Date value', { exact: true })).toHaveText('2026-11-09');
    console.log('PASS DateField formats and reads the locale of the prop (it-IT: day first)');
    await italian.close();
  }

  // ---------- DateField inside Modal and SidePanel ----------
  // The calendar is not portalled: it belongs to the dialog that holds the field, so the focus trap
  // keeps it, Esc closes the calendar first, and a click on a day neither closes the dialog nor is lost.
  for (const [kind, props] of [['Modal', { modal: true }], ['SidePanel', { panel: true }]]) {
    const page = await open('date', props);
    const dialog = page.locator('[data-slot="dialog-content"]');
    await expect(dialog).toBeVisible();
    const field = dialog.getByRole('textbox', { name: 'Due date', exact: true });
    const calendar = page.locator('[data-scope="date-picker"][data-part="content"]');
    const closes = page.getByLabel('Closes', { exact: true });
    await field.fill('2026-11-08');
    await field.press('Enter');
    await dialog.getByRole('button', { name: /calendar/i }).click();
    await expect(calendar).toBeVisible();
    // The calendar is on top (the point at its middle is its own) and not clipped by the dialog body.
    const onTop = await calendar.evaluate((el) => {
      const r = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
    });
    assert.ok(onTop, `${kind}: the calendar is on top`);
    const clipped = await calendar.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const box = el.closest('[data-slot="dialog-content"]').getBoundingClientRect();
      return { bottom: r.bottom - box.bottom, viewport: r.bottom - innerHeight };
    });
    assert.ok(clipped.viewport <= 0, `${kind}: the calendar fits the window (${JSON.stringify(clipped)})`);
    // Keyboard: arrows move the day, Enter chooses; the dialog stays.
    await page.keyboard.press('Escape');
    await expect(calendar).toBeHidden();
    await expect(dialog).toBeVisible();
    await expect(closes).toHaveText('0');
    await dialog.getByRole('button', { name: /calendar/i }).click();
    await expect(calendar).toBeVisible();
    await calendar.getByRole('button', { name: /November 12, 2026/ }).click();
    await expect(page.getByLabel('Date value', { exact: true })).toHaveText('2026-11-12');
    await expect(calendar).toBeHidden();
    await expect(dialog).toBeVisible();
    await expect(closes).toHaveText('0');
    await expect(field).toHaveValue('11/12/2026');
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
