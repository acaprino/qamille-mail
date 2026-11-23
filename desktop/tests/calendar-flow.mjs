// Calendar add flows against a fake desktop backend: no real calendars are touched.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

import { dateText } from './support/dates.mjs';
const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:5187';
const shots = process.env.QAMILLE_SHOTS || '';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US', timezoneId: 'Europe/Rome', viewport: { width: 1180, height: 820 } });
const page = await context.newPage();
const errors = [];
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
  window.fakeEvents = events;
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
      if (action === 'delete_event') {
        const at = events.findIndex((event) => event.id === args.args.id);
        if (at >= 0) events.splice(at, 1);
        return { status: 'deleted' };
      }
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
const shot = async (name) => shots && (await page.screenshot({ path: `${shots}/${name}.png` }));
try {
  await page.goto(`${base}/__calendar-test`);
  const rail = page.getByRole('complementary', { name: 'Calendars' });

  // Each account says what it needs, in words, with the one action that fixes it.
  await expect(rail).toContainText('Google Calendar isn’t connected to this account yet.');
  await expect(rail).toContainText('This account needs Google sign-in for its calendar.');
  await expect(rail).not.toContainText('authorize_calendar');
  await shot('1-rail-states');
  await rail.getByRole('button', { name: 'Open account settings' }).click();
  assert.equal(await page.evaluate(() => window.notices.at(-1).message), 'settings');
  await rail.getByRole('button', { name: 'Connect Google Calendar' }).click();
  await expect(rail.getByRole('checkbox', { name: 'Primary calendar' })).toBeChecked();
  await expect(rail.getByRole('checkbox', { name: /Holidays/ })).toContainText('Read-only');
  await expect(rail.getByRole('checkbox', { name: 'All calendars' })).toBeChecked();
  console.log('PASS connecting an account lists its calendars, all shown together');

  // Dragging across empty slots opens the editor with that exact range.
  const column = page.locator('.calendar-day').nth(1);
  const slot = column.locator('.day-slot');
  const box = await slot.boundingBox();
  const at = (hour) => box.y + hour * 44;
  await page.mouse.move(box.x + 20, at(14) + 2);
  await page.mouse.down();
  await page.mouse.move(box.x + 20, at(15) + 20, { steps: 6 });
  await page.mouse.up();
  const sheet = page.getByRole('dialog', { name: 'New event' });
  await expect(sheet).toBeVisible();
  await expect(sheet.getByLabel('Start time')).toHaveValue('14:00');
  await expect(sheet.getByLabel('End time')).toHaveValue('15:30');
  await expect(sheet).toContainText('1 h 30 min');
  await expect(sheet.getByLabel('Title')).toBeFocused();
  await sheet.getByLabel('Title').fill('Design review');
  await expect(column.locator('.draft-event')).toContainText('Design review');
  await shot('2-draft-sheet');
  console.log('PASS drag-to-create opens the editor on the dragged range');

  // Moving the start keeps the duration; an end before the start blocks saving.
  await sheet.getByLabel('Start time').fill('16:00');
  await expect(sheet.getByLabel('End time')).toHaveValue('17:30');
  await sheet.getByLabel('End time').fill('15:00');
  await expect(sheet.getByRole('alert')).toHaveText('End must be after start');
  await expect(sheet.getByRole('button', { name: 'Save' })).toBeDisabled();
  await sheet.getByLabel('End time').fill('17:00');
  console.log('PASS changing the start keeps the duration and invalid ranges are named');

  // All-day shows inclusive dates and stores the exclusive end the providers expect.
  // The checkbox is Ark's: its box covers the input, so the row (its label) takes the click.
  await sheet.getByText('All day', { exact: true }).click();
  await expect(sheet.getByRole('checkbox', { name: 'All day' })).toBeChecked();
  const startDate = await dateText(sheet.getByLabel('Start date'));
  await expect(sheet.getByLabel('End date')).toHaveText(startDate);
  await expect(sheet).toContainText('1 day');
  await sheet.getByText('All day', { exact: true }).click();
  await expect(sheet.getByRole('checkbox', { name: 'All day' })).not.toBeChecked();
  // With every calendar shown, new events start on this computer; the editor offers the others.
  const calendarSelect = sheet.getByRole('combobox', { name: 'Calendar', exact: true });
  await expect(calendarSelect).toContainText('this computer');
  await calendarSelect.click();
  await page.getByRole('option', { name: /ada@example.com/ }).click();
  await sheet.getByRole('combobox', { name: 'Repeat' }).click();
  await page.getByRole('option', { name: 'Every week', exact: true }).click();
  await sheet.getByLabel('Guests').fill('lin@example.com');
  await expect(sheet.getByRole('combobox', { name: 'Notify' })).toBeVisible();
  await sheet.getByRole('button', { name: 'Save' }).click();
  await expect(sheet).toBeHidden();
  const created = await page.evaluate(() => window.calendarCalls.find((c) => c.action === 'create_event').args);
  assert.equal(created.title, 'Design review');
  assert.equal(created.calendar_id, 'ada@example.com');
  assert.equal(created.account_id, '1');
  assert.deepEqual(created.recurrence, ['RRULE:FREQ=WEEKLY']);
  assert.deepEqual(created.attendees, ['lin@example.com']);
  assert.equal(created.all_day, false);
  assert.equal(new Date(created.end) - new Date(created.start), 3600_000);
  console.log('PASS saving sends the chosen calendar, recurrence and guests');

  // A failed save keeps the editor open and names the problem next to it.
  await page.getByRole('button', { name: 'New event', exact: true }).click();
  const retry = page.getByRole('dialog', { name: 'New event' });
  await retry.getByLabel('Title').fill('Fail');
  await retry.getByRole('button', { name: 'Save' }).click();
  await expect(retry.getByRole('alert')).toContainText('Calendar quota exceeded');
  await expect(retry).toBeVisible();
  await retry.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Calendar');
  console.log('PASS save errors stay in the editor; page heading order starts at h1');

  // All-day events live in their own row, so the day's time slots stay clickable.
  await expect(page.locator('.all-day-row')).toContainText('Offsite');
  await page.getByRole('button', { name: 'New event', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'New event' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', { name: 'New event' })).toBeHidden();
  console.log('PASS all-day row and keyboard close');

  // The focus goes back where it was. An event reached with Tab and opened with Enter has it again after
  // Esc; a new event saved from the editor hands it back to the button that opened it.
  const newEventButton = page.getByRole('button', { name: 'New event', exact: true });
  await newEventButton.focus();
  let reached = false;
  for (let i = 0; i < 80 && !reached; i++) {
    await page.keyboard.press('Tab');
    reached = await page.evaluate(() => document.activeElement?.classList.contains('calendar-event'));
  }
  assert.ok(reached, 'Tab reaches an event');
  await page.evaluate(() => (window.openedEvent = document.activeElement));
  await page.keyboard.press('Enter');
  const edit = page.getByRole('dialog', { name: 'Edit event' });
  await expect(edit).toBeVisible();
  await expect(edit.getByLabel('Title')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(edit).toBeHidden();
  await expect.poll(() => page.evaluate(() => document.activeElement === window.openedEvent)).toBe(true);
  console.log('PASS Esc on an event opened with the keyboard puts the focus back on that event');
  await newEventButton.click();
  const focusSheet = page.getByRole('dialog', { name: 'New event' });
  await focusSheet.getByLabel('Title').fill('Focus check');
  await focusSheet.getByRole('button', { name: 'Save' }).click();
  await expect(focusSheet).toBeHidden();
  await expect(newEventButton).toBeFocused();
  console.log('PASS saving a new event puts the focus back on the button that opened it');
  // Deleting the event the sheet came from leaves nothing to go back to: the focus goes to New event.
  await page.evaluate(() => {
    const day = new Date().toISOString().slice(0, 10);
    window.fakeEvents.push({ id: 'disposable', summary: 'Disposable', revision: '1', start: { dateTime: `${day}T11:00:00+01:00` }, end: { dateTime: `${day}T11:30:00+01:00` } });
  });
  const disposable = page.locator('.calendar-event', { hasText: 'Disposable' }).first();
  // A second event opened with the keyboard while the sheet shows the first is a new opening: Esc goes
  // back to the second event, not to the one that opened the sheet.
  const doomed = page.getByRole('dialog', { name: 'Edit event' });
  // The calendar reads again by itself every 2 seconds, but not while a sheet is open.
  await expect(disposable).toBeVisible();
  await page.locator('.calendar-event', { hasText: 'Standup' }).first().focus();
  await page.keyboard.press('Enter');
  await expect(doomed.getByLabel('Title')).toHaveValue('Standup');
  await disposable.focus();
  await page.keyboard.press('Enter');
  await expect(doomed.getByLabel('Title')).toHaveValue('Disposable');
  await doomed.getByLabel('Title').focus();
  await page.keyboard.press('Escape');
  await expect(doomed).toBeHidden();
  await expect(disposable).toBeFocused();
  console.log('PASS Esc after opening a second event with the sheet open puts the focus on the second event');
  await page.keyboard.press('Enter');
  await expect(doomed.getByLabel('Title')).toHaveValue('Disposable');
  await doomed.getByRole('button', { name: 'Delete', exact: true }).click();
  await doomed.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(doomed).toBeHidden();
  await expect(page.locator('.calendar-event', { hasText: 'Disposable' })).toHaveCount(0);
  await expect(newEventButton).toBeFocused();
  console.log('PASS deleting the event puts the focus on New event, not on <body>');

  // A line marks the current time in today's column, on the matching hour row.
  await expect(page.locator('.now-line')).toHaveCount(1);
  const minute = await page.evaluate(() => {
    const [h, m] = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/Rome', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).format(new Date()).split(':').map(Number);
    return h * 60 + m;
  });
  const top = await page.locator('.now-line').evaluate((el) => parseFloat(el.style.top));
  assert.ok(Math.abs(top - (minute / 60) * 44) <= 44 / 60 * 2, `now line at ${top}px, expected near ${(minute / 60) * 44}px`);
  console.log('PASS the current time line sits on the current hour');

  // Day, week and month views; the choice survives a reload.
  const views = page.getByRole('group', { name: 'Calendar view' });
  await views.getByRole('button', { name: 'Day', exact: true }).click();
  await expect(page.locator('.calendar-day')).toHaveCount(1);
  await expect(page.locator('.now-line')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Next day' })).toBeVisible();
  await views.getByRole('button', { name: 'Month', exact: true }).click();
  await expect(page.locator('.month-cell')).toHaveCount(42);
  // Three calendars each return Standup and Offsite; the cell shows what fits and counts the rest.
  await expect(page.locator('.month-cell.today')).toContainText('Offsite');
  await expect(page.locator('.month-cell.today .month-more')).toHaveText(/\+\d+ more/);
  await page.locator('.month-cell.today .month-more').click();
  await expect(page.locator('.calendar-day')).toHaveCount(1);
  await expect(page.locator('.calendar-event').first()).toContainText('Standup');
  await views.getByRole('button', { name: 'Week', exact: true }).click();
  await expect(page.locator('.calendar-day')).toHaveCount(7);
  await views.getByRole('button', { name: 'Month', exact: true }).click();
  await page.reload();
  await expect(page.locator('.month-cell')).toHaveCount(42);
  await views.getByRole('button', { name: 'Week', exact: true }).click();
  await expect(page.locator('.calendar-day')).toHaveCount(7);
  console.log('PASS day, week and month views switch and are remembered');

  // Calendars can be hidden one by one or all at once.
  const everything = rail.getByRole('checkbox', { name: 'All calendars' });
  await expect(page.locator('.calendar-event').first()).toBeVisible();
  await everything.click();
  await expect(page.locator('.calendar-event')).toHaveCount(0);
  await everything.click();
  await expect(page.locator('.calendar-event').first()).toBeVisible();
  console.log('PASS all calendars can be hidden and shown together');

  if (shots) {
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.evaluate(() => document.documentElement.setAttribute('data-scheme', 'dark'));
    await page.locator('.calendar-event').first().click();
    await page.waitForTimeout(400);
    await shot('3-edit-dark');
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
