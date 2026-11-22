// The calendar screen of the Compatto restyle (spec 2026-11-08 ark-compatto section 6/7.2, Task 13): 32px controls,
// Select/Checkbox/DateField in the event editor (time stays native), the editor as a 420px SidePanel, calendar
// colours kept. Fake desktop backend, synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:5187';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
async function openPage(locale) {
const context = await browser.newContext({ locale, timezoneId: 'Europe/Rome', viewport: { width: 1180, height: 820 } });
const page = await context.newPage();
page.on('pageerror', (error) => errors.push(error.message));
await page.addInitScript(() => {
  window.isTauri = true;
  window.calendarCalls = [];
  window.notices = [];
  let consented = false;
  const today = new Date();
  const day = (offset) => {
    const d = new Date(today);
    d.setDate(d.getDate() + offset);
    return d.toISOString().slice(0, 10);
  };
  const events = [
    { id: 'standup', summary: 'Standup', revision: '1', start: { dateTime: `${day(0)}T09:00:00+01:00` }, end: { dateTime: `${day(0)}T09:30:00+01:00` } },
    { id: 'offsite', summary: 'Offsite', revision: '1', start: { date: day(0) }, end: { date: day(1) } },
  ];
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      if (command === 'workspace_snapshot')
        return { accounts: [
          { id: 1, email: 'ada@example.com', kind: 'Gmail', provider_id: 'gmail' },
          { id: 2, email: 'grace@example.com', kind: 'Gmail', provider_id: 'gmail' },
        ] };
      if (command !== 'calendar_mcp') throw new Error(`Unexpected command: ${command}`);
      window.calendarCalls.push({ action: args.action, args: structuredClone(args.args) });
      const { action } = args;
      if (action === 'list_calendars') {
        const items = [{ id: 'local', provider_id: 'local', name: 'Qamille', time_zone: '', writable: true }];
        const errors = [{ account_id: 2, code: 'sign_in_required', message: 'Connect this account with Google OAuth, then authorize_calendar' }];
        if (consented) items.push(
          { id: 'ada@example.com', provider_id: 'google', account_id: 1, name: 'ada@example.com', time_zone: 'Europe/Rome', writable: true },
          { id: 'holidays', provider_id: 'google', account_id: 1, name: 'Holidays', time_zone: 'Europe/Rome', writable: false },
        );
        else errors.push({ account_id: 1, code: 'calendar_consent_required', message: 'Run authorize_calendar to grant calendar access for this account' });
        return { items, errors };
      }
      if (action === 'authorize_calendar') {
        consented = true;
        return { status: 'authorized' };
      }
      if (action === 'list_events') return { items: events };
      if (action === 'sync_calendar') return { status: 'complete' };
      if (action === 'create_event') {
        if (args.args.title === 'Fail') throw { message: 'Calendar quota exceeded', code: 'external' };
        return { id: 'new', revision: '1' };
      }
      throw new Error(`Unexpected calendar action: ${action}`);
    },
  };
});
await page.route('**/__calendar-test', (route) =>
  route.fulfill({
    contentType: 'text/html',
    body: '<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body><main id="fixture" style="height:100vh;display:flex"></main><script type="module">const { mount } = await import("/tests/support/react-mount.js"); const { default: Component } = await import("/src/components/Calendar.tsx"); mount(Component, { target: document.querySelector("#fixture"), props: { notify: (message, error) => window.notices.push({ message, error }), openSettings: () => window.notices.push({ message: "settings" }) } });</script></body></html>',
  }),
);
await page.goto(`${base}/__calendar-test`);
return page;
}

const steps = [];
const step = async (name, work) => {
  await work();
  steps.push(name);
  console.log(`PASS ${name}`);
};
const heightOf = async (locator) => (await locator.first().boundingBox()).height;
const widthOf = async (locator) => (await locator.first().boundingBox()).width;

try {
  const page = await openPage('en-US');
  const rail = page.getByRole('complementary', { name: 'Calendars' });
  await rail.getByRole('button', { name: 'Connect Google Calendar' }).click();
  await expect(rail.getByRole('checkbox', { name: 'Primary calendar' })).toBeChecked();

  await step('toolbar controls are 32px high; the view switch is one 32px segment row', async () => {
    for (const name of ['Today', 'New event']) assert.equal(await heightOf(page.getByRole('button', { name, exact: true })), 32, name);
    for (const name of ['Previous week', 'Next week']) {
      const found = page.getByRole('button', { name });
      assert.equal(await heightOf(found), 32, name);
      assert.equal(await widthOf(found), 32, name);
    }
    assert.equal(await heightOf(page.getByRole('group', { name: 'Calendar view' })), 32);
  });

  await step('calendar identity colours are kept on the rail dots and on the events', async () => {
    const colours = await page.evaluate(() => {
      const dot = document.querySelector('.cal-row.active[style*="--cal-color"] .cal-dot');
      const row = dot.closest('.cal-row');
      return { row: getComputedStyle(row).getPropertyValue('--cal-color').trim(), dot: getComputedStyle(dot).backgroundColor };
    });
    assert.ok(colours.row.length > 0, 'the row carries --cal-color');
    assert.notEqual(colours.dot, 'rgba(0, 0, 0, 0)');
    await expect(page.locator('.calendar-event').first()).toBeVisible();
    const chip = await page.locator('.calendar-event').first().evaluate((node) => node.style.getPropertyValue('--cal-color').trim());
    assert.ok(chip.length > 0, 'events carry --cal-color');
  });

  await page.getByRole('button', { name: 'New event', exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'New event' });
  await expect(sheet).toBeVisible();

  await step('the event editor is a 420px side panel with no native select, date or checkbox', async () => {
    assert.ok(Math.abs((await widthOf(sheet)) - 420) < 0.5, 'panel is 420px wide');
    assert.equal(await sheet.locator('select:not([data-scope])').count(), 0);
    assert.equal(await sheet.locator('input[type="date"]').count(), 0);
    const boxes = await sheet.locator('input[type="checkbox"]').count();
    assert.equal(boxes, await sheet.locator('[data-scope="checkbox"] input[type="checkbox"]').count(), 'every checkbox is an Ark Checkbox');
    await expect(sheet.getByRole('checkbox', { name: 'All day' })).toBeVisible();
  });

  await step('selects, date fields and time fields in the editor are 32px high', async () => {
    for (const name of ['Calendar']) {
      const trigger = sheet.getByRole('combobox', { name, exact: true });
      assert.equal(await heightOf(trigger), 32, name);
      assert.equal(await trigger.evaluate((n) => n.getAttribute('data-scope')), 'select');
    }
    assert.equal(await sheet.locator('[data-scope="date-picker"][data-part="control"]').count(), 2);
    for (const name of ['Start date', 'End date']) {
      const control = sheet.getByLabel(name, { exact: true }).locator('xpath=ancestor::*[@data-part="control"]');
      assert.equal(await heightOf(control), 32, name);
    }
    for (const name of ['Start time', 'End time']) {
      const field = sheet.getByLabel(name, { exact: true });
      assert.equal(await field.getAttribute('type'), 'time');
      assert.equal(await heightOf(field), 32, name);
    }
  });

  await step('the Select lists the calendars and the repeat rules, and the choice shows in the button', async () => {
    const calendar = sheet.getByRole('combobox', { name: 'Calendar', exact: true });
    await expect(calendar).toContainText('this computer');
    await calendar.click();
    await page.getByRole('option', { name: /ada@example.com/ }).click();
    await expect(calendar).toContainText('ada@example.com');
    const repeat = sheet.getByRole('combobox', { name: 'Repeat', exact: true });
    await expect(repeat).toContainText('Does not repeat');
    await repeat.click();
    await page.getByRole('option', { name: 'Every week', exact: true }).click();
    await expect(repeat).toContainText('Every week');
    assert.equal(await heightOf(repeat), 32);
    await sheet.getByLabel('Guests').fill('lin@example.com');
    const notify = sheet.getByRole('combobox', { name: 'Notify', exact: true });
    assert.equal(await heightOf(notify), 32);
    await notify.click();
    await expect(page.getByRole('option', { name: 'Nobody' })).toBeVisible();
    await page.getByRole('option', { name: 'Nobody' }).click();
    await expect(notify).toContainText('Nobody');
  });

  await step('typing a date moves the event; the calendar button opens a calendar', async () => {
    const start = sheet.getByLabel('Start date', { exact: true });
    await start.fill('12/24/2026');
    await start.press('Enter');
    await expect(start).toHaveValue('12/24/2026');
    await expect(sheet.getByLabel('End date', { exact: true })).toHaveValue('12/24/2026');
    await sheet.getByRole('button', { name: 'Open calendar' }).first().click();
    await expect(page.locator('[data-scope="date-picker"][data-part="content"]').first()).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(sheet).toBeVisible();
  });

  await step('All day is a Checkbox that hides the times and keeps the dates', async () => {
    const allDay = sheet.getByRole('checkbox', { name: 'All day' });
    const row = sheet.getByText('All day', { exact: true });
    await row.click();
    await expect(allDay).toBeChecked();
    await expect(sheet.getByLabel('Start time')).toHaveCount(0);
    await expect(sheet.getByLabel('Start date', { exact: true })).toHaveValue('12/24/2026');
    await row.click();
    await expect(allDay).not.toBeChecked();
    await expect(sheet.getByLabel('Start time')).toBeVisible();
  });

  await step('Escape closes the panel', async () => {
    await page.keyboard.press('Escape');
    await expect(sheet).toBeHidden();
  });

  await step('an existing event opens in the panel without native selects', async () => {
    await page.locator('.calendar-event').first().click();
    const edit = page.getByRole('dialog', { name: 'Edit event' });
    await expect(edit).toBeVisible();
    assert.equal(await edit.locator('select:not([data-scope])').count(), 0);
    await edit.getByRole('button', { name: 'Cancel' }).click();
    await expect(edit).toBeHidden();
  });

  await step('month and week grids keep their structure', async () => {
    const views = page.getByRole('group', { name: 'Calendar view' });
    await views.getByRole('button', { name: 'Month', exact: true }).click();
    await expect(page.locator('.month-cell')).toHaveCount(42);
    await views.getByRole('button', { name: 'Week', exact: true }).click();
    await expect(page.locator('.calendar-day')).toHaveCount(7);
  });

  await step('the selected day of the calendar shows a focus ring in --color-primary-foreground', async () => {
    await page.getByRole('button', { name: 'New event', exact: true }).click();
    const again = page.getByRole('dialog', { name: 'New event' });
    await again.getByRole('button', { name: 'Open calendar' }).first().click();
    const selected = page.locator('[data-part="table-cell-trigger"][data-selected]').first();
    await expect(selected).toBeVisible();
    await page.keyboard.press('Shift');
    await selected.focus();
    const ring = await selected.evaluate((n) => {
      const s = getComputedStyle(n);
      return { style: s.outlineStyle, colour: s.outlineColor, focused: document.activeElement === n };
    });
    const onAccent = await page.evaluate(() => {
      const probe = document.createElement('i');
      probe.style.color = 'var(--color-primary-foreground)';
      document.body.append(probe);
      const colour = getComputedStyle(probe).color;
      probe.remove();
      return colour;
    });
    assert.ok(ring.focused, 'the selected day has the focus');
    assert.notEqual(ring.style, 'none');
    assert.equal(ring.colour, onAccent);
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
  });

  const italian = await openPage('it-IT');
  await step('Italian: date field, calendar button and selects use the app language', async () => {
    await italian.getByRole('complementary', { name: 'Calendari' }).getByRole('button', { name: 'Collega Google Calendar' }).click();
    await expect(italian.getByRole('checkbox', { name: 'Tutti i calendari' })).toBeChecked();
    await italian.getByRole('button', { name: 'Nuovo evento', exact: true }).click();
    const form = italian.getByRole('dialog', { name: 'Nuovo evento' });
    await expect(form).toBeVisible();
    await expect(form.getByLabel('Data di inizio', { exact: true })).toHaveValue(/^\d\d\/\d\d\/\d{4}$/);
    await expect(form.getByRole('button', { name: 'Apri il calendario' }).first()).toBeVisible();
    await form.getByRole('combobox', { name: 'Calendario', exact: true }).click();
    await italian.getByRole('option', { name: /ada@example.com/ }).click();
    await expect(form.getByRole('combobox', { name: 'Ripeti', exact: true })).toContainText('Non si ripete');
    await expect(form.getByRole('checkbox', { name: 'Tutto il giorno' })).toBeVisible();
  });

  assert.deepEqual(errors, []);
  console.log(`compatto-calendar: ok (${steps.length} steps)`);
} finally {
  await browser.close();
}
