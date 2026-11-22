// The composer screen of the Compatto restyle (spec 2026-11-08 ark-compatto §8, Task 10): the
// RecipientsInput primitive on Ark TagsInput + Combobox (mounted alone through the gallery, and in the
// real composer), and the window itself (radius, border, shadow, 32px controls, Tabs for the dock,
// Select for the pickers, DateField for the scheduled send). Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { mountGallery } from './support/mount.mjs';
import { openApp, css, box } from './support/compatto.mjs';

const results = [];
const step = async (name, work) => {
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
};
const errors = [];

// The computed value of `property` when it is set to `var(token)`, so the test reads the token and not a copy of it.
const resolved = (page, property, token) =>
  page.evaluate(
    ([name, value]) => {
      const probe = document.createElement('div');
      probe.style.setProperty(name, `var(${value})`);
      document.body.append(probe);
      const out = getComputedStyle(probe).getPropertyValue(name);
      probe.remove();
      return out;
    },
    [property, token],
  );

// ---------- RecipientsInput alone (gallery) ----------
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 900, height: 700 } });
const gallery = async (props = {}) => {
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  await mountGallery(page, 'recipients', props);
  return page;
};
const chips = (page) => page.locator('[data-scope="tags-input"][data-part="item"]');
const field = (page) => page.getByRole('combobox', { name: 'To', exact: true });
const value = (page) => page.getByLabel('Recipients value', { exact: true });
const listed = (page) => page.getByRole('listbox', { name: 'Suggestions' });
// What Zag reads when text arrives by paste: the input event with inputType insertFromPaste.
const paste = (page, text) =>
  field(page).evaluate((input, pasted) => {
    input.focus();
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(input, pasted);
    input.dispatchEvent(new InputEvent('input', { inputType: 'insertFromPaste', data: pasted, bubbles: true }));
  }, text);

try {
  await step('RecipientsInput: typing "ann" lists the matching contacts', async () => {
    const page = await gallery();
    await field(page).click();
    await field(page).pressSequentially('ann');
    await expect(listed(page)).toBeVisible();
    await expect(listed(page).getByRole('option')).toHaveCount(2);
    await expect(listed(page).getByRole('option', { name: /Anna Bianchi/ })).toBeVisible();
    await page.close();
  });

  await step('RecipientsInput: Enter on a suggestion adds a chip; Enter on a typed address adds a chip', async () => {
    const page = await gallery();
    await field(page).click();
    await field(page).pressSequentially('ann');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(chips(page)).toHaveCount(1);
    await expect(chips(page).first()).toContainText('Anna Bianchi');
    await expect(value(page)).toHaveText('["Anna Bianchi <anna@acme.test>"]');
    await expect(field(page)).toHaveValue('');
    await expect(listed(page)).toHaveCount(0);
    await field(page).pressSequentially('new@x.it');
    await page.keyboard.press('Enter');
    await expect(chips(page)).toHaveCount(2);
    await expect(value(page)).toHaveText('["Anna Bianchi <anna@acme.test>","new@x.it"]');
    await page.close();
  });

  await step('RecipientsInput: comma and semicolon confirm a chip', async () => {
    const page = await gallery();
    await field(page).click();
    await field(page).pressSequentially('one@x.it,');
    await field(page).pressSequentially('two@x.it;');
    await expect(value(page)).toHaveText('["one@x.it","two@x.it"]');
    await expect(field(page)).toHaveValue('');
    await page.close();
  });

  await step('RecipientsInput: the box rests on --color-input (R16)', async () => {
    const page = await gallery();
    const control = page.locator('[data-scope="tags-input"][data-part="control"]');
    assert.equal(await css(control, 'border-top-color'), await resolved(page, 'border-top-color', '--color-input'));
    await page.close();
  });

  await step('RecipientsInput: the same address is never added twice (case ignored), the second is dropped silently', async () => {
    const page = await gallery();
    await field(page).click();
    await field(page).pressSequentially('Dup@x.it,');
    await field(page).pressSequentially('dup@X.IT,');
    await field(page).pressSequentially('Dup Again <dup@x.it>;');
    await expect(value(page)).toHaveText('["Dup@x.it"]');
    await expect(chips(page)).toHaveCount(1);
    await expect(field(page)).toHaveValue('');
    await paste(page, 'p@x.it; P@x.it; q@x.it; DUP@x.it');
    await expect(value(page)).toHaveText('["Dup@x.it","p@x.it","q@x.it"]');
    await field(page).pressSequentially('q@x.it');
    await page.keyboard.press('Tab');
    await expect(chips(page)).toHaveCount(3);
    await page.close();
  });

  await step('RecipientsInput: leaving the field keeps the typed address as a chip', async () => {
    const page = await gallery();
    await field(page).click();
    await field(page).pressSequentially('left@x.it');
    await page.keyboard.press('Tab');
    await expect(value(page)).toHaveText('["left@x.it"]');
    await field(page).click();
    await field(page).pressSequentially('click@x.it');
    await page.getByLabel('Shortcuts', { exact: true }).click();
    await expect(value(page)).toHaveText('["left@x.it","click@x.it"]');
    await page.close();
  });

  await step('RecipientsInput: Backspace on the empty field removes the last chip', async () => {
    const page = await gallery({ value: ['a@x.it', 'b@y.it'] });
    await expect(chips(page)).toHaveCount(2);
    await field(page).click();
    await page.keyboard.press('Backspace');
    await expect(value(page)).toHaveText('["a@x.it"]');
    await field(page).pressSequentially('abc');
    await page.keyboard.press('Backspace');
    await expect(field(page)).toHaveValue('ab');
    await expect(chips(page)).toHaveCount(1);
    await page.close();
  });

  await step('RecipientsInput: the remove button of a chip removes that chip', async () => {
    const page = await gallery({ value: ['a@x.it', 'b@y.it', 'c@z.it'] });
    await page.getByRole('button', { name: 'Remove b@y.it', exact: true }).click();
    await expect(value(page)).toHaveText('["a@x.it","c@z.it"]');
    await page.close();
  });

  await step('RecipientsInput: pasting "a@x.it; b@y.it" makes two chips', async () => {
    const page = await gallery();
    await field(page).click();
    await paste(page, 'a@x.it; b@y.it');
    await expect(chips(page)).toHaveCount(2);
    await expect(value(page)).toHaveText('["a@x.it","b@y.it"]');
    await expect(field(page)).toHaveValue('');
    await page.close();
  });

  await step('RecipientsInput: an address the caller calls invalid gets a marked chip', async () => {
    const page = await gallery({ value: ['good@x.it', 'not-an-address'] });
    await expect(chips(page).nth(0)).not.toHaveAttribute('data-invalid', /.*/);
    await expect(chips(page).nth(1)).toHaveAttribute('data-invalid', '');
    await page.close();
  });

  await step('RecipientsInput: Esc closes the suggestions and not the dialog around it', async () => {
    const page = await gallery({ modal: true });
    await field(page).click();
    await field(page).pressSequentially('ann');
    await expect(listed(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(listed(page)).toHaveCount(0);
    await expect(field(page)).toHaveValue('ann');
    await expect(page.getByLabel('Dialog closes', { exact: true })).toHaveText('0');
    await expect(page.getByRole('dialog')).toBeVisible();
    // With nothing open, the next Esc is the dialog's.
    await page.keyboard.press('Escape');
    await expect(page.getByLabel('Dialog closes', { exact: true })).toHaveText('1');
    await page.close();
  });

  await step('RecipientsInput: n, r and e typed in the field are not shortcuts (R3)', async () => {
    const page = await gallery();
    await field(page).click();
    await field(page).pressSequentially('nre');
    await expect(field(page)).toHaveValue('nre');
    await expect(page.getByLabel('Shortcuts', { exact: true })).toHaveText('0');
    // Typed in the list as well: the input keeps the focus while the list is open.
    await field(page).fill('');
    await field(page).pressSequentially('ann');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('e');
    await expect(page.getByLabel('Shortcuts', { exact: true })).toHaveText('0');
    await page.close();
  });

  await step('RecipientsInput: the list opens above a dialog (R13) and a click on it picks', async () => {
    const page = await gallery({ modal: true });
    await field(page).click();
    await field(page).pressSequentially('ann');
    const option = listed(page).getByRole('option', { name: /Anna Bianchi/ });
    await expect(option).toBeVisible();
    const at = await box(option);
    const top = await page.evaluate(
      ({ x, y }) => document.elementFromPoint(x, y)?.closest('[role="option"]')?.textContent ?? null,
      { x: at.x + at.width / 2, y: at.y + at.height / 2 },
    );
    assert.ok(top && top.includes('Anna Bianchi'), `the option is covered: ${top}`);
    const layer = await css(page.locator('[data-scope="combobox"][data-part="positioner"]'), 'z-index');
    assert.equal(layer, await resolved(page, 'z-index', '--z-toast'), 'suggestions sit at --z-toast');
    await option.click();
    await expect(chips(page)).toHaveCount(1);
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.close();
  });

  // ---------- The composer in the real app ----------
  const { browser: appBrowser, page, errors: appErrors } = await openApp();
  const panel = page.locator('.composer-panel');
  const open = async () => {
    await page.locator('.compose-main').click();
    await expect(panel).toBeVisible();
  };
  const to = () => panel.getByRole('combobox', { name: 'To', exact: true });

  await step('The window has --radius-xl, a --color-line border and the --elevation-lg shadow', async () => {
    await open();
    assert.equal(await css(panel, 'border-top-left-radius'), await resolved(page, 'border-top-left-radius', '--radius-xl'));
    assert.equal(await css(panel, 'border-bottom-left-radius'), await resolved(page, 'border-bottom-left-radius', '--radius-xl'));
    assert.equal(await css(panel, 'border-top-color'), await resolved(page, 'border-top-color', '--color-line'));
    assert.equal(await css(panel, 'border-top-width'), '1px');
    assert.equal(await css(panel, 'box-shadow'), await resolved(page, 'box-shadow', '--elevation-lg'));
  });

  await step('The controls are 32px high', async () => {
    const send = panel.getByRole('button', { name: 'Send', exact: true });
    for (const control of [
      send,
      panel.getByRole('button', { name: 'Schedule send', exact: true }),
      panel.getByRole('button', { name: 'Attach files', exact: true }),
      panel.getByRole('button', { name: 'Discard draft', exact: true }),
      panel.getByRole('combobox', { name: 'From', exact: true }),
      panel.getByRole('combobox', { name: 'Signature', exact: true }),
    ]) {
      assert.equal((await box(control)).height, 32, `${await control.getAttribute('aria-label') ?? 'Send'} is 32px`);
    }
    await panel.getByRole('button', { name: 'Schedule send', exact: true }).click();
    const day = panel.getByRole('textbox', { name: 'Send date', exact: true });
    const time = panel.getByLabel('Send time', { exact: true });
    assert.equal((await box(panel.locator('[data-scope="date-picker"][data-part="control"]'))).height, 32, 'the date field is 32px');
    await expect(day).toBeVisible();
    assert.equal((await box(time)).height, 32, 'the time box is 32px');
    assert.equal(await time.getAttribute('type'), 'time', 'the time stays a native time input');
    await panel.getByRole('button', { name: 'Schedule send', exact: true }).click();
  });

  await step('The pickers are Selects and the dates a DateField (no native select, no datetime-local)', async () => {
    assert.equal(await panel.locator('select').count(), 0, 'no native select in the composer');
    await panel.getByRole('button', { name: 'Schedule send', exact: true }).click();
    assert.equal(await panel.locator('input[type="datetime-local"]').count(), 0, 'no datetime-local');
    await expect(panel.locator('[data-scope="date-picker"][data-part="control"]')).toHaveCount(1);
    await panel.getByRole('button', { name: 'Schedule send', exact: true }).click();
    const signature = panel.getByRole('combobox', { name: 'Signature', exact: true });
    await signature.click();
    await expect(page.getByRole('listbox', { name: 'Signature' })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('listbox', { name: 'Signature' })).toHaveCount(0);
    // Esc inside a picker closes the picker; the window stays.
    await expect(panel).toBeVisible();
  });

  await step('The dock tabs are the Tabs primitive: a tablist with the draft tabs', async () => {
    const tablist = page.getByRole('tablist', { name: 'Draft tabs' });
    await expect(tablist).toHaveAttribute('data-scope', 'tabs');
    await expect(tablist.getByRole('tab')).toHaveCount(1);
    await expect(tablist.getByRole('tab').first()).toHaveAttribute('aria-selected', 'true');
  });

  await step('Recipients in the real composer: "an" suggests, Enter picks, Backspace removes (R3: n, r, e are text)', async () => {
    await to().click();
    await to().pressSequentially('nre');
    await expect(to()).toHaveValue('nre');
    await expect(page.getByRole('tablist', { name: 'Draft tabs' }).getByRole('tab')).toHaveCount(1);
    await to().fill('');
    await to().pressSequentially('an');
    const options = page.getByRole('listbox', { name: 'Suggestions from your contacts' });
    await expect(options.getByRole('option').first()).toBeVisible();
    // Esc closes the suggestions and the composer stays open (the window minimizes on the next Esc).
    await page.keyboard.press('Escape');
    await expect(options).toHaveCount(0);
    await expect(panel).toBeVisible();
    await expect(to()).toHaveValue('an');
    await to().pressSequentially('d');
    await expect(options.getByRole('option').first()).toBeVisible();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(panel.locator('[data-scope="tags-input"][data-part="item"]')).toHaveCount(1);
    await expect(to()).toHaveValue('');
    await page.keyboard.press('Backspace');
    await expect(panel.locator('[data-scope="tags-input"][data-part="item"]')).toHaveCount(0);
  });

  await step('Esc with nothing open minimizes the composer', async () => {
    await to().click();
    await page.keyboard.press('Escape');
    await expect(panel).toBeHidden();
    await expect(page.getByRole('tab', { name: 'New message' })).toBeFocused();
    await page.getByRole('tab', { name: 'New message' }).click();
    await expect(panel).toBeVisible();
  });

  const draftRecipients = (target) =>
    target.evaluate(async () => (await (await import('/src/lib/api.ts')).call('workspace_snapshot')).drafts.map((d) => d.to));

  await step('Typed and not confirmed: Send is enabled and the FIRST click sends that recipient', async () => {
    const fresh = await openApp();
    const area = fresh.page.locator('.composer-panel');
    await fresh.page.locator('.compose-main').click();
    await expect(area).toBeVisible();
    const send = area.getByRole('button', { name: 'Send', exact: true });
    await expect(send).toBeDisabled();
    await area.getByRole('combobox', { name: 'To', exact: true }).pressSequentially('a@b.it');
    await expect(area.locator('[data-scope="tags-input"][data-part="item"]')).toHaveCount(0);
    await expect(send).toBeEnabled();
    await send.click();
    await expect(area).toHaveCount(0);
    await expect(fresh.page.getByRole('button', { name: /^Undo: Sending message/ })).toBeVisible();
    // Undo brings the queued draft back: its To is the address that was typed.
    await fresh.page.getByRole('button', { name: /^Undo: Sending message/ }).click();
    await expect(area).toBeVisible();
    await expect(area.locator('[data-scope="tags-input"][data-part="item-preview"][title="a@b.it"]')).toHaveCount(1);
    assert.deepEqual(fresh.errors, []);
    await fresh.browser.close();
  });

  await step('Typed and not confirmed, then Esc minimizes: the address is saved with the draft and still there', async () => {
    const fresh = await openApp();
    const area = fresh.page.locator('.composer-panel');
    await fresh.page.locator('.compose-main').click();
    await expect(area).toBeVisible();
    await area.getByRole('combobox', { name: 'To', exact: true }).pressSequentially('keep@b.it');
    await fresh.page.keyboard.press('Escape');
    await expect(area).toBeHidden();
    const tab = fresh.page.getByRole('tab', { name: 'New message' });
    await expect(tab).toHaveAttribute('title', /keep@b\.it.*Saved to drafts/);
    const saved = await draftRecipients(fresh.page);
    assert.ok(saved.includes('keep@b.it'), `the persisted draft has the address: ${JSON.stringify(saved)}`);
    await tab.click();
    await expect(area).toBeVisible();
    // Leaving the box (the focus goes to the tab) made the text a chip.
    await expect(area.locator('[data-scope="tags-input"][data-part="item-preview"][title="keep@b.it"]')).toHaveCount(1);
    assert.deepEqual(fresh.errors, []);
    await fresh.browser.close();
  });

  await step('Esc priority: an open side panel takes the Esc before the composer minimizes', async () => {
    const side = await openApp();
    const sidePanel = side.page.locator('[data-scope="dialog"][data-variant="side"][data-part="content"]');
    await side.page.getByRole('button', { name: /Preview/ }).first().click();
    await expect(sidePanel).toHaveCount(1);
    await side.page.locator('.compose-main').click();
    await expect(side.page.locator('.composer-panel')).toBeVisible();
    // The panel is drawn over the composer: put the focus in it the way the keyboard would.
    await side.page.getByRole('textbox', { name: 'Subject' }).focus();
    await side.page.keyboard.press('Escape');
    await expect(sidePanel).toHaveCount(0);
    await expect(side.page.locator('.composer-panel')).toBeVisible();
    assert.deepEqual(side.errors, []);
    await side.browser.close();
  });

  await step('Italian: the labels of the fields, the calendar and the recipient chips (R14)', async () => {
    const italian = await browser.newContext({ locale: 'it-IT', viewport: { width: 1440, height: 900 } });
    const it = await italian.newPage();
    it.on('pageerror', (error) => errors.push(error.message));
    await it.goto(process.env.QAMILLE_PREVIEW_URL ?? 'http://127.0.0.1:1420');
    await it.locator('.compose-main').click();
    const area = it.locator('.composer-panel');
    await expect(area.getByRole('combobox', { name: 'Da', exact: true })).toBeVisible();
    await expect(area.getByRole('combobox', { name: 'Firma', exact: true })).toBeVisible();
    await area.getByRole('combobox', { name: 'A', exact: true }).fill('someone@x.it;');
    await expect(area.getByRole('button', { name: 'Rimuovi someone@x.it', exact: true })).toBeVisible();
    await area.getByRole('button', { name: 'Programma invio', exact: true }).click();
    const day = area.getByRole('textbox', { name: 'Data di invio', exact: true });
    await expect(day).toHaveAttribute('placeholder', 'gg/mm/aaaa');
    await expect(area.getByLabel('Ora di invio', { exact: true })).toBeVisible();
    await area.getByRole('button', { name: 'Apri il calendario', exact: true }).click();
    await expect(it.getByRole('button', { name: 'Mese successivo', exact: true })).toBeVisible();
    await expect(it.getByRole('button', { name: 'Mese precedente', exact: true })).toBeVisible();
    await italian.close();
  });

  await step('DateField takes `disabled` (the composer passes `locked`): no typing, no calendar', async () => {
    const page = await context.newPage();
    page.on('pageerror', (error) => errors.push(error.message));
    await mountGallery(page, 'date', { disabled: true });
    const day = page.getByRole('textbox', { name: 'Due date', exact: true });
    await expect(day).toBeDisabled();
    await expect(page.getByRole('button', { name: /calendar/i })).toBeDisabled();
    await page.getByRole('button', { name: /calendar/i }).click({ force: true });
    await expect(page.locator('[data-scope="date-picker"][data-part="content"]')).toBeHidden();
    assert.ok(Number(await css(page.locator('[data-scope="date-picker"][data-part="control"]'), 'opacity')) < 1, 'it looks disabled');
    await page.close();
    const source = fs.readFileSync(new URL('../src/components/Composer.tsx', import.meta.url), 'utf8');
    assert.match(source, /<DateField[^>]*disabled=\{locked\}/s, 'the schedule row passes locked to the DateField');
  });

  await step('The unsaved-changes dialog is still a small Modal', async () => {
    const source = fs.readFileSync(new URL('../src/components/UnsavedDialog.tsx', import.meta.url), 'utf8');
    assert.match(source, /<Modal\s[^>]*size="sm"/s);
  });

  assert.deepEqual(appErrors, []);
  await appBrowser.close();
  assert.deepEqual(errors, []);
  console.log(`compatto-composer: ${results.length} steps ok`);
} finally {
  await browser.close();
}
