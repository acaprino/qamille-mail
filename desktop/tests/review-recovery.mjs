// Synthetic UI and native-command-boundary regressions. No live mail/calendar/audio.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { spawn } from 'node:child_process';

if (process.env.QAMILLE_CDP) throw new Error('Use synthetic data only');
const port = 28149;
const url = process.env.QAMILLE_PREVIEW_URL || `http://127.0.0.1:${port}`;
const server = process.env.QAMILLE_PREVIEW_URL ? null : spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: path.resolve(import.meta.dirname, '..'), windowsHide: true, stdio: 'pipe' });
let output = '';
server?.stdout.on('data', d => output += d);
server?.stderr.on('data', d => output += d);
let browser;
try {
  for (let i = 0; i < 60; i++) {
    if (server?.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise(r => setTimeout(r, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 1050 } });
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(url);
  await page.locator('.mail-row').first().waitFor();
  await page.evaluate(async () => {
    const api = await import('/src/lib/api.ts');
    await api.call('mail_action', { id: 7, action: 'archive' });
  });
  const folders = page.getByRole('navigation', { name: 'Mail folders' });
  await page.getByRole('tab', { name: 'Unread', exact: true }).click();
  await folders.getByRole('button', { name: /^Archive\s*\d*$/ }).click();
  await expect(page.locator('.mail-list')).toHaveAttribute('aria-busy', 'false');
  await expect(page.locator('.mail-row')).toHaveCount(1);
  assert.equal(await page.getByRole('tab', { name: 'Unread', exact: true }).count(), 0);
  console.log('PASS Inbox Unread does not hide read messages in Archive');

  await page.locator('.compose-main').click();
  const panel = page.getByRole('tabpanel');
  await panel.getByRole('textbox', { name: 'Subject', exact: true }).fill('Local recovery');
  await panel.getByRole('textbox', { name: 'Message body' }).fill('Original body');
  await expect(panel.getByText('Saved to drafts', { exact: true })).toBeVisible();
  const originalId = Number((await panel.getAttribute('id')).replace('compose-panel-', ''));
  const editElsewhere = async (id, subject) => page.evaluate(async ({ id, subject }) => {
    const api = await import('/src/lib/api.ts');
    const drafts = await api.call('list_drafts');
    const draft = drafts.find(d => d.id === id);
    await api.call('save_draft', { draft: { ...draft, subject, body: 'External body' } });
    window.dispatchEvent(new Event('preview-workspace-changed'));
  }, { id, subject });
  await editElsewhere(originalId, 'External original');
  await panel.getByRole('textbox', { name: 'Message body' }).fill('My unsaved local content');
  await panel.getByRole('button', { name: 'Save and close draft', exact: true }).click();
  await expect(panel.getByText('Could not save', { exact: true })).toBeVisible();
  await expect(panel.getByRole('textbox', { name: 'Message body' })).toHaveText('My unsaved local content');
  await panel.getByRole('button', { name: 'Save edits as a new draft', exact: true }).click();
  await expect.poll(async () => Number((await panel.getAttribute('id')).replace('compose-panel-', ''))).not.toBe(originalId);
  const copiedId = Number((await panel.getAttribute('id')).replace('compose-panel-', ''));
  await expect(panel.getByRole('textbox', { name: 'Message body' })).toHaveText('My unsaved local content');
  const drafts = await page.evaluate(async () => (await import('/src/lib/api.ts')).call('list_drafts'));
  assert.equal(drafts.find(d => d.id === originalId).subject, 'External original');
  assert.equal(drafts.find(d => d.id === originalId).body, 'External body');
  assert.match(drafts.find(d => d.id === copiedId).body, /My unsaved local content/);
  console.log('PASS Stale composer saves a separate copy and preserves the external version');

  await editElsewhere(copiedId, 'External copy');
  await panel.getByRole('textbox', { name: 'Message body' }).fill('Discard these local edits');
  await panel.getByRole('button', { name: 'Save and close draft', exact: true }).click();
  await panel.getByRole('button', { name: 'Discard only local edits and close', exact: true }).click();
  await expect(panel).toHaveCount(0);
  const retained = await page.evaluate(async id => (await (await import('/src/lib/api.ts')).call('list_drafts')).find(d => d.id === id), copiedId);
  assert.equal(retained.subject, 'External copy');
  assert.equal(retained.body, 'External body');
  console.log('PASS Discard-local recovery closes the stale tab without deleting the saved draft');
  assert.deepEqual(errors, []);

  const native = await browser.newPage({ locale: 'en-US' });
  await native.route('**/__review_native', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html><body></body></html>' }));
  await native.goto(`${url}/__review_native`);
  const nativeResults = await native.evaluate(async () => {
    globalThis.isTauri = true;
    const commands = [];
    let resolveSave;
    let rejectSave;
    let failDelete = false;
    globalThis.__TAURI_INTERNALS__ = { invoke: (command, args) => {
      commands.push([command, args.id]);
      if (command === 'meeting_save_notes') return new Promise((resolve, reject) => { resolveSave = resolve; rejectSave = reject; });
      if (command === 'meeting_delete') return failDelete ? Promise.reject(new Error('Delete failed')) : Promise.resolve();
      throw new Error(command);
    } };
    const api = await import('/src/lib/meetings.ts');
    api.cacheMeetingDraft(42, 'Pending notes', 1);
    const save = api.saveMeetingDraft(42);
    const deletion = api.deleteMeeting(42);
    const closing = api.flushMeetingDrafts();
    await Promise.resolve();
    const beforeSaveFinishes = structuredClone(commands);
    resolveSave({ revision: 2 });
    await save; await deletion;
    const flush = await closing;
    const afterDelete = { commands: structuredClone(commands), flush, cached: !!api.cachedMeetingDraft(42) };
    api.cacheMeetingDraft(43, 'Keep if delete fails', 1);
    failDelete = true;
    try { await api.deleteMeeting(43); } catch {}
    const retainedOnFailure = api.cachedMeetingDraft(43)?.text;
    failDelete = false;
    await api.deleteMeeting(43);
    api.cacheMeetingDraft(44, 'Conflicting notes', 1);
    const failingSave = api.saveMeetingDraft(44).catch(() => null);
    const deleteAfterFailedSave = api.deleteMeeting(44);
    rejectSave(new Error('Revision conflict'));
    await failingSave; await deleteAfterFailedSave;
    return { beforeSaveFinishes, afterDelete, retainedOnFailure, finalFlush: await api.flushMeetingDrafts() };
  });
  assert.deepEqual(nativeResults.beforeSaveFinishes, [['meeting_save_notes', 42]]);
  assert.deepEqual(nativeResults.afterDelete, { commands: [['meeting_save_notes', 42], ['meeting_delete', 42]], flush: true, cached: false });
  assert.equal(nativeResults.retainedOnFailure, 'Keep if delete fails');
  assert.equal(nativeResults.finalFlush, true);
  console.log('PASS Native meeting deletion waits for in-flight saves, clears successful deletes, retains failed deletes');

  const calendarPage = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 1050 } });
  await calendarPage.route('**/__review_calendar', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body><main id="fixture" style="height:100vh;display:flex"></main></body></html>' }));
  await calendarPage.goto(`${url}/__review_calendar`);
  await calendarPage.clock.install({ time: new Date('2026-11-16T12:00:00Z') });
  await calendarPage.evaluate(async () => {
    globalThis.isTauri = true;
    globalThis.calendarUpdates = [];
    globalThis.calendarNotices = [];
    const occurrence = { id: 'weekly_16', revision: 'instance-etag', summary: 'Weekly meeting', recurringEventId: 'weekly', start: { dateTime: '2026-11-16T09:00:00Z', timeZone: 'UTC' }, end: { dateTime: '2026-11-16T10:00:00Z', timeZone: 'UTC' }, attendees: [] };
    const other = { id: 'other', revision: 'other-etag', summary: 'Another meeting', start: { dateTime: '2026-11-17T11:00:00Z' }, end: { dateTime: '2026-11-17T12:00:00Z' }, location: 'Another location', attendees: [{ email: 'other@example.test' }] };
    const master = { ...occurrence, id: 'weekly', revision: 'master-etag', recurringEventId: undefined, start: { dateTime: '2026-11-02T09:00:00Z', timeZone: 'UTC' }, end: { dateTime: '2026-11-02T10:00:00Z', timeZone: 'UTC' }, recurrence: ['RRULE:FREQ=WEEKLY;COUNT=10'] };
    globalThis.__TAURI_INTERNALS__ = { invoke: async (command, request) => {
      if (command === 'workspace_snapshot') return { accounts: [] };
      if (command !== 'calendar_mcp') throw new Error(command);
      if (request.action === 'list_calendars') return { items: [{ id: 'primary', provider_id: 'google', account_id: 1, name: 'Synthetic calendar', time_zone: 'UTC', writable: true }], errors: [] };
      if (request.action === 'list_events') return { items: request.args.calendar_id === 'primary' ? [occurrence, other] : [] };
      if (request.action === 'read_event') return globalThis.holdMaster ? new Promise(resolve => { globalThis.resolveMaster = () => resolve(master); }) : master;
      if (request.action === 'update_event') { globalThis.calendarUpdates.push(request.args); return master; }
      throw new Error(request.action);
    } };
    const { configureRegional } = await import('/src/lib/regional.ts');
    configureRegional({ time_zone: 'UTC', locale: 'en-US' });
    const { setLanguageChoice } = await import('/src/lib/i18n.ts');
    setLanguageChoice('en');
    const { mount } = await import('/tests/support/react-mount.js');
    const Calendar = (await import('/src/components/Calendar.tsx')).default;
    mount(Calendar, { target: document.querySelector('#fixture'), props: { notify: message => globalThis.calendarNotices.push(message) } });
  });
  // Every calendar is shown by default; its row is a checkbox that hides it when clicked.
  await expect(calendarPage.getByRole('checkbox', { name: 'Synthetic calendar', exact: true })).toBeChecked();
  await calendarPage.getByRole('button', { name: /Weekly meeting/ }).click();
  await calendarPage.getByRole('textbox', { name: 'Title', exact: true }).fill('Renamed series');
  await calendarPage.getByRole('combobox', { name: 'Apply to', exact: true }).click();
  await calendarPage.getByRole('option', { name: 'All events in the series', exact: true }).click();
  await calendarPage.getByRole('button', { name: 'Save', exact: true }).click();
  const updates = await calendarPage.evaluate(() => globalThis.calendarUpdates);
  assert.equal(updates.length, 1);
  assert.deepEqual(updates[0], { calendar_id: 'primary', account_id: '1', send_updates: 'all', id: 'weekly', expected_revision: 'master-etag', scope: 'series', title: 'Renamed series' });
  console.log('PASS Actual calendar form sends a title-only series update with the master identity and no dates');

  await expect(calendarPage.getByRole('dialog')).toHaveCount(0);
  await calendarPage.evaluate(() => { globalThis.holdMaster = true; });
  await calendarPage.getByRole('button', { name: /Weekly meeting/ }).click();
  await calendarPage.getByRole('textbox', { name: 'Title', exact: true }).fill('Saved while another editor opens');
  await calendarPage.getByRole('combobox', { name: 'Apply to', exact: true }).click();
  await calendarPage.getByRole('option', { name: 'All events in the series', exact: true }).click();
  await calendarPage.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => calendarPage.evaluate(() => typeof globalThis.resolveMaster)).toBe('function');
  await calendarPage.getByRole('button', { name: 'Close', exact: true }).click();
  await calendarPage.getByRole('button', { name: /Another meeting/ }).click();
  await calendarPage.getByRole('textbox', { name: 'Title', exact: true }).fill('Keep these unsaved edits');
  await calendarPage.evaluate(() => globalThis.resolveMaster());
  await expect.poll(() => calendarPage.evaluate(() => globalThis.calendarNotices.length)).toBe(2);
  const delayedUpdate = await calendarPage.evaluate(() => globalThis.calendarUpdates[1]);
  assert.deepEqual(delayedUpdate, { calendar_id: 'primary', account_id: '1', send_updates: 'all', id: 'weekly', expected_revision: 'master-etag', scope: 'series', title: 'Saved while another editor opens' });
  await expect(calendarPage.getByRole('dialog')).toBeVisible();
  await expect(calendarPage.getByRole('textbox', { name: 'Title', exact: true })).toHaveValue('Keep these unsaved edits');
  console.log('PASS Delayed series saves keep their original fields and leave a newer event editor open');
} finally {
  await browser?.close();
  server?.kill();
}
