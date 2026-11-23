// Real Composer interactions with controlled desktop replies; no user files or mail are accessed.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';

if (process.env.QAMILLE_CDP) throw new Error('Attachment lease tests require an isolated synthetic browser.');
const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420';
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
const errors = [];
const requests = [];

async function composer() {
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));
  page.on('requestfailed', request => requests.push(`${request.url()}: ${request.failure()?.errorText}`));
  page.on('response', response => {
    if (response.status() >= 400) requests.push(`${response.status()} ${response.url()}`);
  });
  await page.addInitScript(() => {
    window.isTauri = true;
    window.attachmentCalls = [];
    window.attachmentNotices = [];
    window.pendingAttachmentSaves = [];
    window.nextAttachmentImport = { paths: [], lease_id: null };
    window.__TAURI_INTERNALS__ = {
      invoke(command, args = {}) {
        const captured = structuredClone(args);
        window.attachmentCalls.push({ command, args: captured });
        if (command === 'pick_attachments') return Promise.resolve(structuredClone(window.nextAttachmentImport));
        if (command === 'save_draft') return new Promise(resolve => {
          window.pendingAttachmentSaves.push(() => resolve({ ...captured.draft, updated: captured.draft.updated + 1 }));
        });
        if (command === 'release_attachment_imports') return Promise.resolve(null);
        throw new Error(`Unexpected desktop command: ${command}`);
      },
    };
    window.finishAttachmentSave = index => window.pendingAttachmentSaves[index]();
  });
  await page.route('**/__attachment-import-leases', route => route.fulfill({
    contentType: 'text/html',
    body: `<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body>
      <main id="fixture"></main><script type="module">
        const { mount } = await import('/tests/support/react-mount.js');
        const { setLanguageChoice } = await import('/src/lib/i18n.ts');
        const { default: Composer } = await import('/src/components/Composer.tsx');
        const sample = await fetch('/src/lib/sample.json').then(response => response.json());
        setLanguageChoice('en');
        const draft = { id: 42, account_id: sample.accounts[0].id, to: '', cc: '', bcc: '', subject: '',
          body: '', reply_to: null, send_at: null, attachments: [], updated: 1, format: 'Plain',
          signature_id: null, quote: '' };
        mount(Composer, { target: document.querySelector('#fixture'), props: {
          initial: draft, workspace: { ...sample, has_ai_key: false, has_google_secret: false },
          active: true, windowClosing: false, close: () => {}, changed: () => {}, queued: () => {},
          minimize: () => {}, register: () => () => {}, stateChanged: () => {}, copied: () => {},
          notify: (message, error) => window.attachmentNotices.push({ message, error: !!error }),
        } });
      </script></body></html>`,
  }));
  await page.goto(`${base}/__attachment-import-leases`);
  try {
    await expect(page.getByRole('textbox', { name: 'Message body', exact: true })).toBeVisible();
  } catch (error) {
    throw new Error(`Composer did not mount. Page errors: ${JSON.stringify(errors)}. Failed requests: ${JSON.stringify(requests)}`, { cause: error });
  }
  await page.clock.install();
  await page.clock.pauseAt(new Date(Date.now() + 1000));
  return page;
}

const saves = page => page.evaluate(() => window.attachmentCalls
  .filter(call => call.command === 'save_draft').map(call => call.args));
const releases = page => page.evaluate(() => window.attachmentCalls
  .filter(call => call.command === 'release_attachment_imports').map(call => call.args));
const finishSave = (page, index) => page.evaluate(index => window.finishAttachmentSave(index), index);
async function importAttachment(page, path, leaseId) {
  await page.evaluate(value => { window.nextAttachmentImport = value; }, { paths: [path], lease_id: leaseId });
  await page.getByRole('button', { name: 'Attach files', exact: true }).click();
  await expect(page.getByRole('button', { name: `Remove ${path.split(/[\\/]/).pop()}`, exact: true })).toBeVisible();
}
async function autosave(page, count) {
  await page.clock.runFor(1250);
  await expect.poll(async () => (await saves(page)).length).toBe(count);
}

try {
  const race = await composer();
  const leaseA = 'a'.repeat(32);
  const pathA = 'D:\\Synthetic\\data\\draft-attachments\\first\\race.txt';
  await race.getByRole('textbox', { name: 'Subject', exact: true }).fill('Older autosave snapshot');
  await autosave(race, 1);
  assert.deepEqual((await saves(race))[0].draft.attachments, []);
  assert.deepEqual((await saves(race))[0].importLeaseIds || [], []);

  await importAttachment(race, pathA, leaseA);
  assert.equal((await saves(race)).length, 1, 'The older autosave remains pending while the picker returns');
  assert.deepEqual(await releases(race), [], 'A new import remains protected while an older save is pending');
  await finishSave(race, 0);
  await expect.poll(async () => (await saves(race)).length).toBe(2);
  const raceSaves = await saves(race);
  assert.deepEqual(raceSaves[0].importLeaseIds || [], [], 'The older snapshot cannot acknowledge a later import');
  assert.deepEqual(raceSaves[1].draft.attachments, [pathA]);
  assert.deepEqual(raceSaves[1].importLeaseIds, [leaseA], 'The snapshot containing the import acknowledges its own lease');
  await finishSave(race, 1);
  await expect(race.getByRole('status')).toHaveText('Saved to drafts');
  await race.getByRole('textbox', { name: 'Subject', exact: true }).fill('Saved attachment stays referenced');
  await autosave(race, 3);
  assert.deepEqual((await saves(race))[2].importLeaseIds || [], [], 'A completed import is acknowledged only once');
  await finishSave(race, 2);
  await expect(race.getByRole('status')).toHaveText('Saved to drafts');
  assert.deepEqual(await race.evaluate(() => window.attachmentNotices), []);
  console.log('PASS an older autosave preserves a later import lease until the matching draft snapshot is saved');
  await race.close();

  const removed = await composer();
  const leaseB = 'b'.repeat(32);
  const pathB = 'D:\\Synthetic\\data\\draft-attachments\\second\\removed.txt';
  await importAttachment(removed, pathB, leaseB);
  await removed.getByRole('button', { name: 'Remove removed.txt', exact: true }).click();
  await expect(removed.getByRole('button', { name: 'Remove removed.txt', exact: true })).toHaveCount(0);
  assert.deepEqual(await saves(removed), [], 'The attachment was removed before its first save');
  assert.deepEqual(await releases(removed), [], 'Removing a file waits for a saved snapshot before releasing its lease');
  await autosave(removed, 1);
  const removedSave = (await saves(removed))[0];
  assert.deepEqual(removedSave.draft.attachments, []);
  assert.deepEqual(removedSave.importLeaseIds, [leaseB], 'Saving removal acknowledges the import even when no attachment remains');
  await finishSave(removed, 0);
  await expect(removed.getByRole('status')).toHaveText('Saved to drafts');
  await removed.getByRole('textbox', { name: 'Subject', exact: true }).fill('Removed import already released');
  await autosave(removed, 2);
  assert.deepEqual((await saves(removed))[1].importLeaseIds || [], []);
  await finishSave(removed, 1);
  await expect(removed.getByRole('status')).toHaveText('Saved to drafts');
  assert.deepEqual(await removed.evaluate(() => window.attachmentNotices), []);
  assert.deepEqual(errors, []);
  console.log('PASS removing an imported file before autosave releases its lease with the empty saved snapshot');
} finally {
  await browser.close();
}
