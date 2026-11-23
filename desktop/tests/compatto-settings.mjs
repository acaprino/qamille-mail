// Task 16 part A of the Ark Compatto plan: Settings, Local storage and Search, measured in computed values
// (spec 2026-11-08 sections 6 and 7.2) and driven through roles and accessible names.
// Sample data only. Run: node scripts/test-browser.mjs compatto-settings
import { expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { box, css, openApp } from './support/compatto.mjs';

const { browser, page, errors } = await openApp();
page.setDefaultTimeout(15000);

const token = (name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }, name);
const section = (name) => page.locator('.settings-nav').getByRole('button', { name, exact: true }).click();
const content = page.locator('.settings-content');

try {
  await page.locator('.sidebar .profile').click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();

  // (a) General: the old selects are Ark Selects (role combobox, 32px, 6px radius, field border), none native.
  assert.equal(await content.locator('select').count(), 0, 'General has no native <select>');
  assert.equal(await content.locator('input[type="checkbox"]:not([data-scope] input)').count(), 0, 'General has no plain checkbox');
  for (const name of ['Time zone', 'Language', 'Regional format', 'Undo time', 'Appearance']) {
    const trigger = content.getByRole('combobox', { name, exact: true });
    await expect(trigger).toBeVisible();
    assert.equal(Math.round((await box(trigger)).height), 32, `${name}: 32px high`);
    assert.equal(await css(trigger, 'border-top-left-radius'), '6px', `${name}: 6px radius`);
    assert.equal(await css(trigger, 'border-top-color'), await token('--color-input'), `${name}: rests on --color-input`);
  }
  // The text fields beside the Selects are 32px too.
  assert.equal(Math.round((await box(content.getByRole('textbox', { name: 'Your name' }))).height), 32, 'Your name: 32px high');
  // The automatic choice shows its words, not an empty button.
  await expect(content.getByRole('combobox', { name: 'Time zone', exact: true })).toContainText('Automatic');
  await expect(content.getByRole('combobox', { name: 'Regional format', exact: true })).toContainText('Automatic');
  console.log('PASS General selects are Ark Selects with the Compatto measures');

  // (b) A choice is made in the list, and the form notices it.
  await content.getByRole('combobox', { name: 'Undo time', exact: true }).click();
  await page.getByRole('option', { name: '10 seconds' }).click();
  await expect(content.getByRole('combobox', { name: 'Undo time', exact: true })).toContainText('10 seconds');
  await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible();
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(content.getByRole('combobox', { name: 'Undo time', exact: true })).not.toContainText('10 seconds');
  console.log('PASS choosing an option marks the form unsaved and Discard restores it');

  // (c) The on/off settings are Switches: a 32x18 track, the name on the switch, a click on the label toggles.
  for (const name of ['Organize incoming mail', 'Suggest labels']) {
    const toggle = content.getByRole('switch', { name, exact: true });
    await expect(toggle).toBeAttached();
    const track = content.locator('[data-scope="switch"][data-part="control"]').nth(
      ['Organize incoming mail', 'Suggest labels'].indexOf(name),
    );
    const size = await box(track);
    assert.deepEqual([Math.round(size.width), Math.round(size.height)], [32, 18], `${name}: 32x18 track`);
  }
  // The switch sits at the right end of its row, the name and the help text at the left.
  const row = content.locator('.setting-row').first();
  const rowBox = await box(row);
  const trackBox = await box(row.locator('[data-scope="switch"][data-part="control"]'));
  assert.ok(rowBox.x + rowBox.width - (trackBox.x + trackBox.width) < 4, 'the track is at the right end of the row');
  // The meeting reminders are in the Meetings section now, not in General.
  await expect(content.getByRole('switch', { name: 'Meeting reminders', exact: true })).toHaveCount(0);
  await expect(content.getByRole('combobox', { name: 'Reminder time', exact: true })).toHaveCount(0);
  await section('Meetings');
  const reminders = content.getByRole('switch', { name: 'Meeting reminders', exact: true });
  const lead = content.getByRole('combobox', { name: 'Reminder time', exact: true });
  await expect(reminders).toBeAttached();
  const reminderTrack = content.locator('[data-scope="switch"][data-part="control"]').first();
  const reminderSize = await box(reminderTrack);
  assert.deepEqual([Math.round(reminderSize.width), Math.round(reminderSize.height)], [32, 18], 'Meeting reminders: 32x18 track');
  assert.equal(Math.round((await box(lead)).height), 32, 'Reminder time: 32px high');
  assert.equal(await css(lead, 'border-top-left-radius'), '6px', 'Reminder time: 6px radius');
  assert.equal(await css(lead, 'border-top-color'), await token('--color-input'), 'Reminder time rests on --color-input');
  await expect(reminders).toBeChecked();
  await expect(lead).toBeEnabled();
  await content.getByText('Meeting reminders', { exact: true }).click();
  await expect(reminders).not.toBeChecked();
  await expect(lead).toBeDisabled();
  await reminders.press('Space');
  await expect(reminders).toBeChecked();
  await expect(lead).toBeEnabled();
  const reminderRow = content.locator('.setting-row').first();
  const reminderRowBox = await box(reminderRow);
  const reminderTrackBox = await box(reminderRow.locator('[data-scope="switch"][data-part="control"]'));
  assert.ok(reminderRowBox.x + reminderRowBox.width - (reminderTrackBox.x + reminderTrackBox.width) < 4, 'the reminders track is at the right end of the row');
  for (const name of ['Transcription language', 'Transcription engine', 'Model', 'Microphone', 'Computer audio']) {
    const trigger = content.getByRole('combobox', { name, exact: true });
    await expect(trigger).toBeVisible();
    assert.equal(Math.round((await box(trigger)).height), 32, `${name}: 32px high`);
    assert.equal(await css(trigger, 'border-top-left-radius'), '6px', `${name}: 6px radius`);
  }
  assert.equal(await content.locator('select').count(), 0, 'Meetings has no native <select>');
  console.log('PASS on/off settings are Switches');

  // (d) Appearance still applies at once through the Select.
  await section('General');
  await content.getByRole('combobox', { name: 'Appearance', exact: true }).click();
  await page.getByRole('option', { name: 'Dark' }).click();
  await page.waitForFunction(() => document.documentElement.dataset.scheme === 'dark');
  await content.getByRole('combobox', { name: 'Appearance', exact: true }).click();
  await page.getByRole('option', { name: 'Automatic (system)' }).click();
  await page.emulateMedia({ colorScheme: 'light' });
  await page.waitForFunction(() => document.documentElement.dataset.scheme === 'light');
  console.log('PASS Appearance applies from the Select');

  // (e) Assistant and Signatures.
  await section('Assistant');
  assert.equal(await content.locator('select').count(), 0, 'Assistant has no native <select>');
  const provider = content.getByRole('combobox', { name: 'Provider', exact: true });
  await expect(provider).toBeVisible();
  await provider.click();
  await page.getByRole('option', { name: 'Anthropic' }).click();
  await expect(content.getByRole('textbox', { name: 'Model' })).toBeVisible();
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await section('Signatures');
  const html = content.getByRole('checkbox', { name: 'This signature contains HTML' });
  await expect(html).not.toBeChecked();
  await content.getByText('This signature contains HTML').click();
  await expect(html).toBeChecked();
  assert.equal(await content.locator('input[type="checkbox"]:not([data-scope="checkbox"] input)').count(), 0, 'Signatures has no plain checkbox');
  console.log('PASS Assistant provider is a Select and the signature HTML flag a Checkbox');

  // (f) Agent access: the level is a Select.
  await section('Agent access');
  const level = content.getByRole('combobox', { name: 'Access level', exact: true });
  await expect(level).toBeVisible();
  await level.click();
  await expect(page.getByRole('listbox', { name: 'Access level' }).getByRole('option', { name: 'Read only' })).toBeVisible();
  await page.keyboard.press('Escape');
  console.log('PASS Access level is a Select');

  // (g) Local storage: the two selects and the offline flag.
  await section('Local storage');
  const account = content.getByRole('combobox', { name: 'Storage account' });
  const mode = content.getByRole('combobox', { name: 'Local storage', exact: true });
  await expect(account).toBeVisible();
  await expect(mode).toBeVisible();
  assert.equal(Math.round((await box(mode)).height), 32);
  await expect(content.locator('select')).toHaveCount(0);
  const offline = content.getByRole('switch', { name: 'Keep all mail offline', exact: true });
  await expect(offline).toBeAttached();
  await content.getByText('Keep all mail offline', { exact: true }).click();
  await expect(offline).toBeChecked();
  console.log('PASS Local storage uses Selects and a Switch');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
