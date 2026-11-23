// P2 of the Sigrid plan («Che cosa chiedere a Sigrid»): Base UI and Ark UI live together inside Sigrid. Sigrid's
// fields that rest on Ark (DateField, DateTimeField, Combobox, ChipsInput with suggestions) inside each of Sigrid's
// surfaces on Base UI (Dialog, Sheet, Popover). In every pair:
//   - choosing a day or an item leaves the outer surface open;
//   - Esc closes only the innermost overlay, and a second Esc the surface;
//   - the focus stays in the surface, and Tab does not leave it. A Popover is not modal (a press outside or the
//     focus leaving close it, as Qamille's Popover does): there Tab goes from the field to the next control of the
//     Popover, and the Popover stays open;
//   - the open overlay is reachable: inside the surface, not inert, not hidden from screen readers, not «outside».
// And with a modal Dialog or Sheet open, a click on a notice of Toasts (its «Undo», its action, its «×») works and
// does not close the surface (`ignoreToasts`).
//
// Sigrid's primitives only (tests/support/gallery/SigridCoexistence.tsx), in the clean room (Sigrid's CSS alone,
// tests/support/clean-room.html): a failure here is Sigrid's, and the request goes to Sigrid with the pairs listed.
// Every check is run on a page of its own and all are run: the summary lists each one that does not hold. Each
// field is also tried alone in the page, with no surface, and there the value must be exactly the one chosen. A
// defect of a field alone is not one of the two libraries living together: it is listed apart, as a request of its
// own to Sigrid, and does not make P2 fail; in a pair the value must only hold what was chosen. Synthetic data only.
//   npm run test:browser -- sigrid-coexistence
//   node tests/sigrid-preconditions.mjs --require P2      (runs this)
import { chromium, expect } from '@playwright/test';
import { mountCleanRoom } from './support/sigrid-leak.mjs';

const SURFACES = ['dialog', 'sheet', 'popover'];
const FIELDS = ['date', 'datetime', 'combobox', 'chips'];
const PANEL = {
  dialog: '[data-slot="dialog-content"]',
  sheet: '[data-slot="sheet-content"]',
  popover: '[data-slot="popover-content"]',
  none: 'section[aria-label="Sigrid coexistence"]',
};
const INNER = {
  date: '[data-slot="date-field-content"]',
  datetime: '[data-slot="date-field-content"]',
  combobox: '[data-slot="combobox-content"]',
  chips: '[data-slot="chips-suggestions"]',
};
const LABEL = { date: 'Due date', datetime: 'Reminder', combobox: 'Owner', chips: 'To' };

expect.configure({ timeout: 4000 });
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 900 } });
const failures = [];
const defects = [];
const notes = [];
let passed = 0;

const output = (page, name) => page.getByLabel(name, { exact: true });
const focusIn = (locator) => locator.evaluate((el) => el.contains(document.activeElement));
const focusWithin = (locator) => expect.poll(() => focusIn(locator), { message: 'the focus is inside' }).toBe(true);

/** A fresh page with one surface (and one field in it), opened; with `none`, the field alone in the page. */
async function opened(surface, field, extra = {}) {
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await mountCleanRoom(page, 'sigrid-coexistence', { surface, field, ...extra });
  const panel = page.locator(PANEL[surface]);
  if (surface === 'none') return { page, panel, errors };
  await page.getByRole('button', { name: `Open ${surface}`, exact: true }).click();
  await expect(panel).toBeVisible();
  // Base UI moves the focus into the surface a frame after it shows; keys sent before would go to the page.
  await focusWithin(panel);
  return { page, panel, errors };
}

/** The field's own control: the button that opens the calendar, or the text box. */
const control = (panel, field) =>
  field === 'date' || field === 'datetime'
    ? panel.getByRole('button', { name: LABEL[field], exact: true })
    : panel.getByRole('combobox', { name: LABEL[field], exact: true });

/** Opens the field's own overlay (the calendar, the list, the suggestions) and waits for it. */
async function openInner(page, panel, field) {
  const inner = page.locator(INNER[field]);
  await control(panel, field).click();
  // The suggestions of the chips come with what is typed: «b» matches one address only.
  if (field === 'chips') await page.keyboard.type('b');
  await expect(inner).toBeVisible();
  return inner;
}

/** Chooses with the pointer: a day of the calendar, an option, a suggestion. Gives the value expected after it. */
async function pick(inner, field) {
  if (field === 'date' || field === 'datetime') {
    await inner.getByRole('button', { name: /November 12, 2026/ }).click();
    return field === 'date' ? '2026-11-12' : '2026-11-12T09:00';
  }
  if (field === 'combobox') {
    await inner.getByRole('option', { name: 'Bob Bianchi' }).click();
    return 'bob';
  }
  await inner.getByRole('option', { name: 'bob@example.com' }).click();
  return 'bob@example.com';
}

/** Runs one check on a page of its own; `own` marks the checks of a field alone (not P2). */
async function check(name, work, { own = false } = {}) {
  let page;
  try {
    page = await work();
    passed++;
    console.log(`PASS ${name}`);
  } catch (error) {
    // The assertion and what it found: Playwright's message, without its call log.
    const lines = String(error?.message ?? error)
      .replace(/\u001b\[[0-9;]*m/g, '')
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean);
    const end = lines.findIndex((line) => line.startsWith('Call log'));
    const reason = (end === -1 ? lines : lines.slice(0, end)).slice(0, 5).join(' / ') || 'failed';
    (own ? defects : failures).push({ name, reason });
    console.log(`${own ? 'DEFECT' : 'FAIL'} ${name}\n     ${reason}`);
  } finally {
    if (page && !page.isClosed()) await page.close();
  }
}

/** Fails on a page error, then gives the page back for check() to close. */
function clean({ page, errors }) {
  if (errors.length) throw new Error(`page errors: ${errors.join(' | ')}`);
  return page;
}

/**
 * Choosing with the pointer, then with the keyboard: the surface stays open and keeps the focus. Alone (`none`) the
 * value is exactly the one chosen; in a surface it only has to hold it.
 */
async function choosing(surface, field) {
  const exact = surface === 'none';
  const holds = (locator, text, message) => (exact ? expect(locator, message).toHaveText(text) : expect(locator, message).toContainText(text));
  const run = await opened(surface, field);
  const { page, panel } = run;
  const inner = await openInner(page, panel, field);
  const value = await pick(inner, field);
  await holds(output(page, 'Field value'), value, 'the value after a pick with the pointer');
  if (field === 'datetime') {
    // The reminder's card stays open after a day: its «Time» box and «Done» are clicks inside it too.
    await expect(inner).toBeVisible();
    await inner.getByRole('checkbox', { name: 'Time', exact: true }).click();
    await expect(inner).toBeVisible();
    await inner.getByRole('button', { name: 'Done', exact: true }).click();
  }
  await expect(inner).toBeHidden();
  await expect(panel).toBeVisible();
  await expect(output(page, 'Surface closes')).toHaveText('0');
  await focusWithin(panel);
  // Once more from the keyboard: the arrows and Enter.
  if (field === 'date') {
    await control(panel, field).click();
    await expect(inner).toBeVisible();
    await focusWithin(inner);
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('Enter');
    await expect(output(page, 'Field value'), 'the value after the arrows and Enter').toHaveText('2026-11-13');
  } else if (field === 'combobox' || field === 'chips') {
    await control(panel, field).focus();
    // «carla» matches only carla@example.com (every address has «c» in «.com»).
    if (field === 'chips') await page.keyboard.type('carla');
    else await page.keyboard.press('ArrowDown');
    await expect(inner).toBeVisible();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    if (field === 'chips') await holds(output(page, 'Field value'), exact ? `${value}, carla@example.com` : 'carla@example.com', 'the value after the arrows and Enter');
    else await expect(output(page, 'Field value'), 'the value after the arrows and Enter').not.toHaveText(value);
  }
  await expect(panel).toBeVisible();
  await expect(output(page, 'Surface closes')).toHaveText('0');
  await focusWithin(panel);
  return clean(run);
}

try {
  for (const field of FIELDS) await check(`${field} alone: a pick gives exactly the value chosen`, () => choosing('none', field), { own: true });

  for (const surface of SURFACES) {
    for (const field of FIELDS) {
      const pair = `${field} in ${surface}`;

      await check(`${pair}: choosing leaves the ${surface} open, with the focus inside`, () => choosing(surface, field));

      await check(`${pair}: Esc closes the inner overlay first, then the ${surface}`, async () => {
        const run = await opened(surface, field);
        const { page, panel } = run;
        const inner = await openInner(page, panel, field);
        if (field === 'date' || field === 'datetime') await focusWithin(inner);
        await page.keyboard.press('Escape');
        await expect(inner, 'the inner overlay after the first Esc').toBeHidden();
        await expect(panel, `the ${surface} after the first Esc`).toBeVisible();
        await expect(output(page, 'Surface closes')).toHaveText('0');
        await focusWithin(panel);
        await page.keyboard.press('Escape');
        await expect(panel, `the ${surface} after the second Esc`).toBeHidden();
        await expect(output(page, 'Surface closes')).toHaveText('1');
        return clean(run);
      });

      await check(`${pair}: the open overlay is inside the ${surface}, not inert, not hidden from screen readers`, async () => {
        const run = await opened(surface, field);
        const { page, panel } = run;
        const inner = await openInner(page, panel, field);
        const seen = await inner.evaluate(
          (el, panelSelector) => ({
            inside: el.closest(panelSelector) !== null,
            inert: el.closest('[inert]') !== null,
            hidden: el.closest('[aria-hidden="true"]') !== null,
          }),
          PANEL[surface],
        );
        if (!seen.inside || seen.inert || seen.hidden) throw new Error(`the overlay is ${JSON.stringify(seen)}`);
        // What screen readers get: the role tree reaches it (an aria-hidden part would not be found).
        const role = field === 'date' || field === 'datetime' ? 'grid' : 'listbox';
        await expect(page.getByRole(role)).toBeVisible();
        return clean(run);
      });

      await check(`${pair}: the focus does not leave the ${surface} with Tab`, async () => {
        const run = await opened(surface, field);
        const { page, panel } = run;
        // Through the inner overlay first: open it and close it, then walk.
        await openInner(page, panel, field);
        await page.keyboard.press('Escape');
        await expect(panel).toBeVisible();
        await focusWithin(panel);
        if (surface === 'popover') {
          await control(panel, field).focus();
          await page.keyboard.press('Tab');
          await expect(panel.getByRole('button', { name: 'Save', exact: true })).toBeFocused();
          await expect(panel).toBeVisible();
        } else {
          for (const key of ['Tab', 'Shift+Tab']) {
            for (let i = 0; i < 8; i++) {
              await page.keyboard.press(key);
              // Wrapping goes through a focus guard, and the focus is handed back a moment later.
              const stays = await expect.poll(() => focusIn(panel), { timeout: 1500 }).toBe(true).then(() => true, () => false);
              if (!stays) throw new Error(`${key} number ${i + 1} took the focus out of the ${surface}`);
            }
          }
          await expect(panel).toBeVisible();
        }
        return clean(run);
      });
    }
  }

  for (const surface of ['dialog', 'sheet']) {
    await check(`notices with the ${surface} open: «Undo», an action and «×» work, and the ${surface} stays`, async () => {
      const run = await opened(surface, 'none', { toasts: true });
      const { page, panel } = run;
      const notices = page.locator('[data-toasts]');
      await expect(notices).toBeVisible();
      const hidden = await notices.evaluate((el) => ({ inert: el.closest('[inert]') !== null, ariaHidden: el.closest('[aria-hidden="true"]') !== null }));
      notes.push(`notices with the ${surface} open: ${hidden.inert ? 'inert' : 'not inert'}, ${hidden.ariaHidden ? 'hidden from screen readers (aria-hidden)' : 'not hidden from screen readers'}`);
      // CSS locators: a click must work even where the role tree would not show the notice.
      await notices.locator('button', { hasText: 'Undo' }).click({ timeout: 3000 });
      await expect(output(page, 'Undo count'), '«Undo» ran').toHaveText('1');
      await notices.locator('button', { hasText: 'Inbox' }).click({ timeout: 3000 });
      await expect(output(page, 'Move count'), 'the action ran').toHaveText('1');
      await expect(panel).toBeVisible();
      await notices.locator('button[aria-label^="Close the notice"]').click({ timeout: 3000 });
      await expect(output(page, 'Notice count'), '«×» closed the notice').toHaveText('0');
      await expect(panel).toBeVisible();
      await expect(output(page, 'Surface closes')).toHaveText('0');
      return clean(run);
    });
  }

  // The Sigrid plan's insertion points: a task of wave C adds the pairs its primitive brings under its own
  // comment, and Task 16 runs them all on the merged wave.

  // Task 10: toaster

  // Task 11: date field

  // Task 12: select

  // Task 13: popover, menu and tooltip

  // Task 14: modal and side panel

  // Task 15: recipients

} finally {
  await browser.close();
}

const list = (items) => items.map(({ name, reason }) => `  - ${name}: ${reason}`).join('\n');
for (const note of notes) console.log(`NOTE ${note}`);
console.log(`\nsigrid-coexistence: ${passed} of ${passed + failures.length + defects.length} checks hold`);
if (defects.length) console.log(`Defects of a field alone, not of P2 (a request of their own to Sigrid):\n${list(defects)}`);
if (failures.length) {
  console.log(`P2 does not hold on the synced Sigrid. What does not:\n${list(failures)}`);
  process.exit(1);
}
console.log('P2 holds on the synced Sigrid');
