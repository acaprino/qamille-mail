// Small behaviours of the simple Meetings page that the larger scenarios do not reach, on the
// browser preview (en-US, 1440x900), synthetic data only, no audio device:
//   - Rename starts from the name the meeting has: empty for an untitled one (and refused empty),
//     not from the words the page falls back on.
//   - Several boxes ticked while one save is on its way are all saved, one per round trip, each
//     with the revision the answer before it brought: none is lost, none is sent twice.
//   - "Delete audio" pressed while another command is on its way sends nothing and keeps the
//     dialog open, instead of closing as if it had worked.
// The meetings are made in the preview's own pipeline and waited for in real time (about five
// seconds each); `meeting_preview_delay` holds the answers back so a test can act while one waits.
// Run: node scripts/test-browser.mjs meetings-sweep
import { expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { openApp } from './support/compatto.mjs';

const results = [];
// SWEEP_STEPS=a,b runs only those scenarios.
const only = process.env.SWEEP_STEPS?.split(',');
const step = async (name, work) => {
  if (only && !only.includes(name)) return;
  await work();
  results.push(name);
  console.log(`PASS ${name}`);
};

const { browser, page, errors } = await openApp();
const workspace = page.getByRole('navigation', { name: 'Workspace' });
const root = page.locator('.meetings-page');
const pane = page.locator('.meetings-page .detail-pane');
const api = (command, args = {}) =>
  page.evaluate(
    async ({ command, args }) => (await import('/src/lib/meetings.ts')).meetingCall(command, args),
    { command, args },
  );
const read = (id) => api('meeting_read', { id });
const rowButton = (title) =>
  root
    .locator('[data-row-key]')
    .filter({ has: page.locator('.item-title', { hasText: new RegExp(`^${title}$`) }) })
    .getByRole('button');
let nextRequest = 0;
/** Records and stops a meeting, and waits for the whole chain (transcript, summary, audio verdict). */
async function makeReady(title, { keepAudio = false } = {}) {
  if (keepAudio) await api('meeting_preview_readiness', { transcription_blocker: 'No engine yet.' });
  const started = await api('meeting_start', {
    request: { request_id: `sweep-${++nextRequest}`, title, microphone: 'demo-mic', system: 'demo-system' },
  });
  const id = started.meeting.id;
  await api('meeting_stop');
  if (keepAudio) {
    // A recording with gaps keeps its audio after the transcript.
    await api('meeting_preview_interrupt', { id });
    await api('meeting_preview_readiness', { transcription_blocker: null });
    await api('meeting_recover', { id });
  }
  for (let i = 0; i < 100; i++) {
    if ((await read(id)).active_summary) return id;
    await page.waitForTimeout(200);
  }
  throw new Error('The preview chain did not finish');
}
/** Away and back: the page reads the meetings once, when it opens. */
async function openPage() {
  await page.getByRole('navigation', { name: 'Mail folders' }).getByRole('button', { name: /^Inbox\s*\d*$/ }).click();
  await workspace.getByRole('button', { name: 'Meetings', exact: true }).click();
  await expect(root.locator('[data-list-ready]')).toBeAttached();
}
async function openMeeting(title) {
  await rowButton(title).click();
  await expect(pane.getByRole('heading', { name: title, level: 2 })).toBeVisible();
}
/** The meeting without a name: its row says "Untitled meeting", like the header of its detail. */
async function openUntitled() {
  await root
    .locator('[data-row-key]')
    .filter({ has: page.locator('.item-title', { hasText: /^Untitled meeting$/ }) })
    .getByRole('button')
    .click();
  await expect(pane.getByRole('heading', { name: 'Untitled meeting', level: 2 })).toBeVisible();
}
const ACTIONS = ['Preparare la revisione del progetto.', 'Inviare il verbale a tutti.', 'Fissare la prossima riunione.'];

try {
  const named = await makeReady('Sweep named');

  await step('rename_starts_from_the_name_the_meeting_has', async () => {
    await openPage();
    await openMeeting('Sweep named');
    await pane.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('button', { name: 'Rename', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Rename meeting' });
    await expect(dialog.getByRole('textbox', { name: 'Name' })).toHaveValue('Sweep named');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toHaveCount(0);

    // An untitled meeting (the page calls it "Untitled meeting") starts empty: that is not a name
    // to save, and the Save button says so until one is typed. Deleting still names it as shown.
    await api('meeting_preview_untitle', { id: named });
    assert.equal((await read(named)).meeting.title, '');
    await openPage();
    await openUntitled();
    await pane.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('button', { name: 'Rename', exact: true }).click();
    const name = dialog.getByRole('textbox', { name: 'Name' });
    await expect(name).toHaveValue('');
    await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
    await name.fill('Sweep renamed');
    await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(pane.getByRole('heading', { name: 'Sweep renamed', level: 2 })).toBeVisible();
    assert.equal((await read(named)).meeting.title, 'Sweep renamed');

    // The delete sentence uses the title the page shows.
    await api('meeting_preview_untitle', { id: named });
    await openPage();
    await openUntitled();
    await pane.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Delete meeting' })).toContainText('Delete “Untitled meeting”?');
    await page.getByRole('dialog', { name: 'Delete meeting' }).getByRole('button', { name: 'Cancel', exact: true }).click();
  });

  const ticking = await makeReady('Sweep ticks');
  await api('meeting_preview_more_actions', { id: ticking, texts: ACTIONS.slice(1) });

  await step('a_single_press_is_saved_once_and_keeps_the_focus', async () => {
    await openPage();
    await openMeeting('Sweep ticks');
    const view = pane.locator('.detail-view');
    const box = pane.getByRole('checkbox', { name: ACTIONS[0] });
    const before = (await read(ticking)).revision;
    await api('meeting_preview_delay', { ms: 700 });
    await box.focus();
    await page.keyboard.press('Space');
    await expect(view).toHaveAttribute('aria-busy', 'true');
    await expect(box).toBeChecked();
    await expect(box).toBeFocused();
    await expect(view).not.toHaveAttribute('aria-busy', 'true');
    await expect(box).toBeFocused();
    await page.waitForTimeout(1200); // a second save, if one was queued by mistake, would be here
    const after = await read(ticking);
    assert.equal(after.revision, before + 1, 'one press, one save');
    assert.equal(after.summaries[0].actions[0].done, true);
    await api('meeting_preview_delay', { ms: 0 });
  });

  await step('boxes_ticked_during_one_save_are_all_saved_one_per_round_trip', async () => {
    await openPage();
    await openMeeting('Sweep ticks');
    const view = pane.locator('.detail-view');
    const boxes = ACTIONS.map((text) => pane.getByRole('checkbox', { name: text }));
    const firstDone = !(await boxes[0].isChecked()); // what the first press makes of it
    const before = (await read(ticking)).revision;
    await api('meeting_preview_delay', { ms: 1500 });
    await boxes[0].focus();
    await page.keyboard.press('Space'); // the save goes out, and takes 1.5 s
    await expect(view).toHaveAttribute('aria-busy', 'true');
    await pane.getByText(ACTIONS[1], { exact: true }).click(); // two different boxes, pressed while it is on its way
    await pane.getByText(ACTIONS[2], { exact: true }).click();
    await expect(boxes[1]).toBeChecked();
    await expect(boxes[2]).toBeChecked();
    await expect.poll(async () => (await read(ticking)).revision, { timeout: 15_000 }).toBe(before + 3);
    await expect(view).not.toHaveAttribute('aria-busy', 'true');
    await page.waitForTimeout(1200); // nothing more goes out
    const after = await read(ticking);
    assert.equal(after.revision, before + 3, 'three presses, three saves, no more');
    assert.deepEqual(
      after.summaries[0].actions.map((action) => action.done),
      [firstDone, true, true],
      'the meeting has what was pressed on each box',
    );
    for (const [i, expected] of [firstDone, true, true].entries()) {
      if (expected) await expect(boxes[i]).toBeChecked();
      else await expect(boxes[i]).not.toBeChecked();
    }
    // Each save went out with the revision the answer before it brought: none was refused.
    await expect(page.locator('.toast, [role="alert"]')).toHaveCount(0);
    await api('meeting_preview_delay', { ms: 0 });
  });

  const kept = await makeReady('Sweep kept audio', { keepAudio: true });
  await step('delete_audio_pressed_while_another_command_runs_keeps_the_dialog', async () => {
    await openPage();
    await openMeeting('Sweep kept audio');
    const view = pane.locator('.detail-view');
    const note = pane.locator('.audio-note');
    await expect(note).toContainText('Recording with gaps: the audio is kept.');
    const confirm = page.getByRole('dialog', { name: 'Delete audio' });
    await api('meeting_preview_delay', { ms: 1500 });
    // A command is on its way (a box ticked) when the dialog is opened and confirmed.
    await pane.getByText(ACTIONS[0], { exact: true }).click();
    await expect(view).toHaveAttribute('aria-busy', 'true');
    await note.getByRole('button', { name: 'Delete audio', exact: true }).click();
    await expect(confirm).toBeVisible();
    await confirm.getByRole('button', { name: 'Delete audio', exact: true }).click();
    await page.waitForTimeout(300);
    await expect(confirm, 'nothing was sent, so the dialog is still there').toBeVisible();
    assert.equal((await read(kept)).audio_removed, false);
    // Once the other command is over, the same press goes through.
    await expect(view).not.toHaveAttribute('aria-busy', 'true');
    await confirm.getByRole('button', { name: 'Delete audio', exact: true }).click();
    await expect(confirm).toHaveCount(0);
    assert.equal((await read(kept)).audio_removed, true);
    await expect(note).toContainText('The audio was deleted after it was transcribed.');
    await api('meeting_preview_delay', { ms: 0 });
  });

  assert.deepEqual(errors, []);
  console.log(`${results.length} scenarios passed`);
} finally {
  await browser.close();
}
