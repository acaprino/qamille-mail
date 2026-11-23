// Settings > Meetings on the browser preview (en-US, 1440x900), sample data only: how a recording is
// transcribed (language, engine, model), which devices it uses, the meeting reminders and the
// one-shot audio cleanup. The form is the Settings form: nothing is saved until the Save bar says so.
// Run: node scripts/test-browser.mjs meetings-settings
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

const url = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
const page = await context.newPage();
page.setDefaultTimeout(15000);
const errors = [];
page.on('pageerror', (error) => errors.push(error.stack || error.message));

// The meeting commands do not go through the preview's `__previewCalls`, so the test counts them
// itself: it adds a line to `meetingCall` as the module is served (and a hook that makes the next
// `window.__failModels` reads of the model list fail).
await page.route(/\/src\/lib\/meetings\.ts(\?.*)?$/, async (route) => {
  const response = await route.fetch();
  const source = await response.text();
  const head = 'export async function meetingCall(command, args = {}) {';
  assert.ok(source.includes(head), 'meetingCall is where the call counter expects it');
  await route.fulfill({
    response,
    body: source.replace(
      head,
      `${head}
  (window.__meetingCalls ??= []).push(command);
  if (command === 'meeting_models' && window.__failModels > 0) {
    window.__failModels--;
    throw new Error('The model list is unavailable.');
  }`,
    ),
  });
});

const calls = (command) =>
  page.evaluate((name) => (window.__meetingCalls ?? []).filter((c) => c === name).length, command);
const meetingApi = (command, args = {}) =>
  page.evaluate(
    async ({ command, args }) =>
      (await import('/src/lib/meetings.ts')).meetingCall(command, args),
    { command, args },
  );
const workspaceSettings = () =>
  page.evaluate(async () => (await (await import('/src/lib/api.ts')).call('workspace_snapshot')).settings);

const content = page.locator('.settings-content');
const nav = page.locator('.settings-nav');
async function openMeetings() {
  await page.goto(url);
  await page.locator('.compose-main').waitFor();
  await page.locator('.sidebar .profile').click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await nav.getByRole('button', { name: 'Meetings', exact: true }).click();
  await expect(content.getByRole('heading', { name: 'Meetings', exact: true })).toBeVisible();
}
const combo = (name) => content.getByRole('combobox', { name, exact: true });
const options = async (name) => {
  await combo(name).click();
  // A list that was closed may stay in the page, hidden: the one looked at is the one that shows.
  const popup = page.locator('[data-slot="select-content"]:visible');
  await popup.getByRole('option').first().waitFor();
  const listed = await popup.getByRole('option').allTextContents();
  await page.keyboard.press('Escape');
  await expect(combo(name)).toHaveAttribute('aria-expanded', 'false');
  return listed.map((text) => text.trim());
};
const megabytes = (bytes) => `${(bytes / 1_048_576).toFixed(1)} MB`;

try {
  // 1
  await openMeetings();
  const entries = await nav.getByRole('button').allTextContents();
  assert.deepEqual(entries.slice(-3), ['Contacts', 'Meetings', 'Agent access'], 'Meetings sits between Contacts and Agent access');

  await expect(combo('Transcription language')).toContainText('Italiano');
  assert.deepEqual(await options('Transcription language'), ['Italiano', 'English', 'Deutsch', 'Español', 'Français']);
  await expect(combo('Transcription engine')).toContainText('Compute server');
  assert.deepEqual(await options('Transcription engine'), ['Compute server', 'External OpenAI-compatible']);
  await expect(content).toContainText('Without a server the recordings wait to be transcribed and are transcribed when you connect one.');
  assert.equal(await content.getByText('on this computer').count(), 0, 'nothing runs on this computer any more');
  await expect(combo('Model')).toContainText('Whisper Base');
  await expect(combo('Model')).toContainText('141 MB');
  await expect(content.getByRole('button', { name: 'Download model', exact: true })).toHaveCount(0);
  await expect(content.getByRole('progressbar')).toHaveCount(0);
  await expect(combo('Microphone')).toContainText('System default');
  assert.deepEqual(await options('Microphone'), ['System default', 'Sample microphone']);
  await expect(combo('Computer audio')).toContainText('System default');
  assert.deepEqual(await options('Computer audio'), ['System default', 'Sample computer audio']);
  const reminders = content.getByRole('switch', { name: 'Meeting reminders', exact: true });
  await expect(reminders).toBeChecked();
  await expect(combo('Reminder time')).toBeEnabled();
  await content.getByText('Meeting reminders', { exact: true }).click();
  await expect(reminders).not.toBeChecked();
  await expect(combo('Reminder time')).toBeDisabled();
  await content.getByText('Meeting reminders', { exact: true }).click();
  await expect(combo('Reminder time')).toBeEnabled();
  assert.equal(await content.locator('select').count(), 0, 'no native <select>');
  assert.equal(await content.locator('input[type="checkbox"]:not([data-scope] input)').count(), 0, 'no plain checkbox');
  // The reminders left General.
  await nav.getByRole('button', { name: 'General', exact: true }).click();
  await expect(content.getByRole('switch', { name: 'Meeting reminders', exact: true })).toHaveCount(0);
  await expect(content.getByRole('combobox', { name: 'Reminder time', exact: true })).toHaveCount(0);
  console.log('PASS section_lists_language_engine_model_devices_and_reminders');

  // 2
  await nav.getByRole('button', { name: 'Meetings', exact: true }).click();
  const remoteModel = content.getByRole('textbox', { name: 'Remote model', exact: true });
  await expect(remoteModel).toHaveCount(0);
  await combo('Transcription engine').click();
  await page.getByRole('option', { name: 'External OpenAI-compatible' }).click();
  await expect(remoteModel).toBeVisible();
  await expect(content).toContainText('The audio is sent to the OpenAI-compatible provider set in Assistant');
  // The help is the field's description: a screen reader reads it with the field.
  const describedBy = await remoteModel.getAttribute('aria-describedby');
  assert.ok(describedBy, 'the remote model field points at its help');
  const described = await page.evaluate(
    (ids) => ids.split(' ').map((id) => document.getElementById(id)?.textContent?.trim() ?? null),
    describedBy,
  );
  assert.ok(described.every(Boolean), `every id it points to exists (${describedBy})`);
  assert.match(described[0], /^The audio is sent to the OpenAI-compatible provider/);
  await expect(combo('Model')).toHaveCount(0);
  await expect(content.getByRole('button', { name: 'Download model', exact: true })).toHaveCount(0);
  await expect(content.getByRole('progressbar')).toHaveCount(0);
  assert.equal(Math.round((await remoteModel.boundingBox()).height), 32, 'the remote model field is 32px high like the Selects');
  await remoteModel.fill('whisper-1');
  await combo('Transcription engine').click();
  await page.getByRole('option', { name: 'Compute server' }).click();
  await expect(remoteModel).toHaveCount(0);
  await expect(combo('Model')).toBeVisible();
  await expect(content).not.toContainText('The audio is sent');
  console.log('PASS remote_engine_shows_the_remote_model_field_and_hides_the_local_one');

  // 4 (a fresh page: the sample meeting "Pricing review" has a transcript version, so its audio is due)
  await openMeetings();
  const due = await meetingApi('meeting_audio_cleanup_preview');
  assert.ok(due.meetings >= 1 && due.bytes > 0, 'the sample data has audio the cleanup would take');
  const open = content.getByRole('button', { name: 'Delete audio…', exact: true });
  await expect(content.getByRole('heading', { name: 'Audio of meetings already transcribed' })).toBeVisible();
  await open.click();
  const dialog = page.getByRole('dialog');
  const meetingsWord = due.meetings === 1 ? 'meeting' : 'meetings';
  await expect(dialog.getByRole('heading')).toHaveText(`Delete the audio of ${due.meetings} ${meetingsWord}?`);
  await expect(dialog).toContainText(`This frees ${megabytes(due.bytes)}.`);
  await expect(dialog).toContainText('Transcripts stay.');
  await expect(dialog).toContainText('This cannot be undone.');
  assert.equal(await calls('meeting_audio_cleanup'), 0, 'asking deletes nothing');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  assert.equal(await calls('meeting_audio_cleanup'), 0, 'cancelling deletes nothing');
  assert.deepEqual(await meetingApi('meeting_audio_cleanup_preview'), due, 'the audio is still there');
  await open.click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  assert.equal(await calls('meeting_audio_cleanup'), 1, 'the confirmation deletes');
  await expect(page.getByText(`Audio deleted: ${megabytes(due.bytes)} freed`)).toBeVisible();
  assert.equal((await meetingApi('meeting_audio_cleanup_preview')).meetings, 0, 'nothing is left to delete');
  await open.click();
  await expect(page.getByText('There is no audio to delete.')).toBeVisible();
  await expect(dialog).toHaveCount(0);
  console.log('PASS cleanup_asks_with_count_and_megabytes_before_deleting');

  // 3a The transcription model of the server cannot be installed from the app: with a server
  // connected and the model missing there, the text says who installs it; no download, no polling.
  {
    const connectedContext = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
    const connected = await connectedContext.newPage();
    connected.on('pageerror', (error) => errors.push(error.stack || error.message));
    connected.setDefaultTimeout(15000);
    await connected.addInitScript(() => (window.__previewServer = { connected: true }));
    await connected.goto(url);
    await connected.locator('.compose-main').waitFor();
    await connected.locator('.sidebar .profile').click();
    await connected.locator('.settings-nav').getByRole('button', { name: 'Meetings', exact: true }).click();
    const panel = connected.locator('.settings-content');
    await expect(panel).toContainText('The server set in Settings → Compute transcribes: the meeting audio is sent to that server.');
    await expect(panel).toContainText('This model is not installed on the server: its administrator installs it.');
    await expect(panel.getByRole('button', { name: 'Download model', exact: true })).toHaveCount(0);
    await connectedContext.close();
    console.log('PASS the_server_model_is_installed_by_its_administrator_not_from_the_app');
  }

  // 3d A saved language or model the lists do not have is still shown, by its code (an older
  // version, an edited settings file): not an empty field.
  const previewSource = /\/src\/lib\/preview\.ts(\?.*)?$/;
  const unknownSettings = async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    const edited = source
      .replace(/meeting_language \?\?= (["'])it\1/, 'meeting_language ??= "pt"')
      .replace(/meeting_model \?\?= (["'])base\1/, 'meeting_model ??= "legacy-tiny"');
    assert.notEqual(edited, source, 'the preview sets the meeting language and model where the test expects');
    await route.fulfill({ response, body: edited });
  };
  await page.route(previewSource, unknownSettings);
  await openMeetings();
  await expect(combo('Transcription language')).toContainText('pt');
  assert.deepEqual(await options('Transcription language'), ['Italiano', 'English', 'Deutsch', 'Español', 'Français', 'pt']);
  await expect(combo('Model')).toContainText('legacy-tiny');
  const modelOptions = await options('Model');
  assert.equal(modelOptions.at(-1), 'legacy-tiny', 'the stray model comes last, by its id');
  assert.ok(modelOptions.length >= 2, 'the real models are still listed');
  // Choosing one of the list takes the stray value out of it.
  await combo('Transcription language').click();
  await page.getByRole('option', { name: 'English' }).click();
  assert.deepEqual(await options('Transcription language'), ['Italiano', 'English', 'Deutsch', 'Español', 'Français']);
  await page.unroute(previewSource, unknownSettings);
  console.log('PASS a_saved_language_or_model_the_list_does_not_have_is_shown_by_its_code');

  // 3e Microphones and computer audio are told apart by one rule, shared with the recording.
  const kinds = await page.evaluate(async () => {
    const view = await import('/src/lib/meetings-view.ts');
    const all = ['microphone', 'input', 'system', 'output'];
    return {
      microphone: all.map((kind) => view.isMicrophone({ kind })),
      system: all.map((kind) => view.isSystemAudio({ kind })),
    };
  });
  assert.deepEqual(kinds.microphone, [true, true, false, false]);
  assert.deepEqual(kinds.system, [false, false, true, true]);
  console.log('PASS devices_are_told_apart_by_the_shared_rule');

  // 5
  await openMeetings();
  const before = await workspaceSettings();
  assert.deepEqual(
    [before.meeting_language, before.meeting_engine, before.meeting_model, before.meeting_microphone, before.meeting_system_audio, before.meeting_reminders, before.reminder_lead_minutes],
    ['it', 'compute', 'base', '', '', true, 1],
  );
  await combo('Transcription language').click();
  await page.getByRole('option', { name: 'Deutsch' }).click();
  await combo('Transcription engine').click();
  await page.getByRole('option', { name: 'External OpenAI-compatible' }).click();
  await content.getByRole('textbox', { name: 'Remote model', exact: true }).fill('whisper-1');
  await combo('Microphone').click();
  await page.getByRole('option', { name: 'Sample microphone' }).click();
  await combo('Computer audio').click();
  await page.getByRole('option', { name: 'Sample computer audio' }).click();
  await combo('Reminder time').click();
  await page.getByRole('option', { name: '5 minutes before' }).click();
  await content.getByText('Meeting reminders', { exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible();
  assert.deepEqual(await workspaceSettings(), before, 'nothing is saved before the Save bar is used');
  // Discard puts the form back.
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(combo('Transcription language')).toContainText('Italiano');
  await expect(combo('Transcription engine')).toContainText('Compute server');
  await expect(combo('Microphone')).toContainText('System default');
  await expect(content.getByRole('switch', { name: 'Meeting reminders', exact: true })).toBeChecked();
  await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toHaveCount(0);
  // And again, this time saved.
  await combo('Transcription language').click();
  await page.getByRole('option', { name: 'Deutsch' }).click();
  await combo('Transcription engine').click();
  await page.getByRole('option', { name: 'External OpenAI-compatible' }).click();
  await content.getByRole('textbox', { name: 'Remote model', exact: true }).fill('whisper-1');
  await combo('Microphone').click();
  await page.getByRole('option', { name: 'Sample microphone' }).click();
  await combo('Computer audio').click();
  await page.getByRole('option', { name: 'Sample computer audio' }).click();
  await combo('Reminder time').click();
  await page.getByRole('option', { name: '5 minutes before' }).click();
  await content.getByText('Meeting reminders', { exact: true }).click();
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.getByText('Settings saved')).toBeVisible();
  const saved = await workspaceSettings();
  assert.equal(saved.meeting_language, 'de');
  assert.equal(saved.meeting_engine, 'external');
  assert.equal(saved.meeting_remote_model, 'whisper-1');
  assert.equal(saved.meeting_model, 'base', 'the local model is left alone');
  assert.equal(saved.meeting_microphone, 'demo-mic');
  assert.equal(saved.meeting_system_audio, 'demo-system');
  assert.equal(saved.meeting_reminders, false);
  assert.equal(saved.reminder_lead_minutes, 5);
  // The saved choice is what the section shows when it opens again, and "System default" saves as ''.
  await nav.getByRole('button', { name: 'General', exact: true }).click();
  await nav.getByRole('button', { name: 'Meetings', exact: true }).click();
  await expect(combo('Transcription language')).toContainText('Deutsch');
  await expect(combo('Microphone')).toContainText('Sample microphone');
  await combo('Microphone').click();
  await page.getByRole('option', { name: 'System default' }).click();
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect.poll(async () => (await workspaceSettings()).meeting_microphone).toBe('');
  console.log('PASS changes_are_saved_with_the_settings_bar');

  // 6 (a fresh page) Rule R14: the control that was pressed is not disabled while its own command
  // runs (Chromium would drop the keyboard focus to the page); it says it is busy and a second press
  // sends nothing. The `meeting_preview_delay` hook holds every answer back like a real backend.
  await openMeetings();
  await meetingApi('meeting_preview_delay', { ms: 700 });
  const open6 = content.getByRole('button', { name: 'Delete audio…', exact: true });
  const asked = await calls('meeting_audio_cleanup_preview');
  await open6.focus();
  await page.keyboard.press('Enter');
  await expect(open6).toHaveAttribute('aria-busy', 'true');
  await expect(open6).toHaveAttribute('aria-disabled', 'true');
  await expect(open6).toBeFocused();
  await page.keyboard.press('Enter');
  const dialog6 = page.getByRole('dialog');
  await expect(dialog6).toBeVisible({ timeout: 10_000 });
  assert.equal(await calls('meeting_audio_cleanup_preview'), asked + 1, 'a second press sent nothing');
  // The confirmation: "Delete" keeps the focus until the dialog closes.
  const confirm6 = dialog6.getByRole('button', { name: 'Delete', exact: true });
  await confirm6.focus();
  await page.keyboard.press('Enter');
  await expect(confirm6).toHaveAttribute('aria-busy', 'true');
  await expect(confirm6).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog6).toHaveCount(0, { timeout: 10_000 });
  assert.equal(await calls('meeting_audio_cleanup'), 1, 'one deletion, however many presses');
  await meetingApi('meeting_preview_delay', { ms: 0 });
  console.log('PASS pressed_controls_keep_the_focus_and_ignore_a_second_press');

  // 7 Settings saved before the compute server reach the form with their former names: `remote` is
  // the external engine, `local` the compute server. Saving writes the new name.
  for (const [former, now, label] of [
    ['remote', 'external', 'External OpenAI-compatible'],
    ['local', 'compute', 'Compute server'],
  ]) {
    const formerSettings = async (route) => {
      const response = await route.fetch();
      const source = await response.text();
      const edited = source.replace(
        /store\.settings\.meeting_engine = meetingEngine\(store\.settings\.meeting_engine\);/,
        `store.settings.meeting_engine = '${former}';`,
      );
      assert.notEqual(edited, source, 'the preview sets the meeting engine where the test expects');
      await route.fulfill({ response, body: edited });
    };
    await page.route(previewSource, formerSettings);
    await openMeetings();
    assert.equal((await workspaceSettings()).meeting_engine, former);
    await expect(combo('Transcription engine')).toContainText(label);
    await combo('Transcription language').click();
    await page.getByRole('option', { name: 'English' }).click();
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await expect(page.getByText('Settings saved')).toBeVisible();
    assert.equal((await workspaceSettings()).meeting_engine, now, `${former} is saved as ${now}`);
    await page.unroute(previewSource, formerSettings);
  }
  console.log('PASS engines_saved_with_their_former_names_read_as_the_new_ones');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
