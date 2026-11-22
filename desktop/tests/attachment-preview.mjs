// Attachment preview interactions: real embedded PDF engine, simulated desktop bridge.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { samplePdf, sourceFrame } from './support/sample-documents.mjs';

const port = 28164;
const url = `http://127.0.0.1:${port}`;
const server = spawn(
  process.execPath,
  ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'],
  { cwd: path.resolve(import.meta.dirname, '..'), windowsHide: true, stdio: 'pipe' },
);
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));

async function waitForServer() {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (server.exitCode != null) throw new Error(output);
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    if (attempt === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

let browser;
try {
  await waitForServer();
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1200, height: 900 } });
  await context.addInitScript(
    ({ good, bad }) => {
      const mode = new URLSearchParams(window.location.search).get('mode') || 'visual';
      window.isTauri = true;
      window.previewCalls = [];
      window.previewNotices = [];
      window.previewParentClosed = 0;
      window.resolveSlowSource = null;
      window.workers = { created: 0, terminated: 0 };
      const NativeWorker = window.Worker;
      window.Worker = class extends NativeWorker {
        constructor(...args) {
          super(...args);
          window.workers.created += 1;
        }
        terminate() {
          window.workers.terminated += 1;
          super.terminate();
        }
      };
      window.__TAURI_INTERNALS__ = {
        invoke(command, args = {}) {
          const captured = structuredClone(args);
          window.previewCalls.push({ command, args: captured });
          if (command === 'attachment_preview_source') {
            if (captured.attachmentId === 8) {
              throw { message: 'No visual preview is available for this file type.', code: 'preview_unsupported' };
            }
            const bytes = Uint8Array.from(mode === 'corrupt' ? bad : good).buffer;
            if (mode === 'slow') {
              return new Promise((resolve) => {
                window.resolveSlowSource = () => resolve(bytes);
              });
            }
            return Promise.resolve(bytes);
          }
          if (command === 'read_attachment') return Promise.resolve('Extracted text fallback');
          // The desktop answers a save with where the file went and a token that stands for it.
          if (command === 'attachment_save') {
            const save = new URLSearchParams(window.location.search).get('save');
            return save === 'cancel' ? Promise.resolve(null) : Promise.resolve({ path: 'C:\\Saved\\report.pdf', token: 41 });
          }
          if (command === 'saved_file_open' || command === 'saved_file_reveal') {
            if (new URLSearchParams(window.location.search).get('launch') === 'fail') {
              return Promise.reject({ message: 'The file is gone: it was moved or deleted.', code: 'not_found' });
            }
            return Promise.resolve(null);
          }
          throw new Error(`Unexpected desktop command: ${command}`);
        },
      };
    },
    {
      good: sourceFrame('pdf', samplePdf(['Interaction test'])),
      bad: sourceFrame('pdf', Buffer.from('%PDF-1.7 not a document')),
    },
  );

  async function openFixture(mode, attachment, extra = {}) {
    const page = await context.newPage();
    page.setDefaultTimeout(30000);
    const query = new URLSearchParams({ mode, attachment: JSON.stringify(attachment), ...extra });
    await page.goto(`${url}/tests/attachment-preview.html?${query}`, {
      timeout: 30000,
      waitUntil: 'domcontentloaded',
    });
    return page;
  }
  const file = (id, name, extra = {}) => ({
    id,
    name,
    size: 2048,
    mime: 'application/pdf',
    path: null,
    inline_text: null,
    ...extra,
  });
  const pdf = file(7, 'report.pdf');
  const secondPdf = file(9, 'second.pdf');
  const text = file(8, 'notes.txt', { mime: 'text/plain', inline_text: 'Plain sample text' });
  const commands = (page) => page.evaluate(() => window.previewCalls.map((call) => call.command));

  const visual = await openFixture('visual', pdf);
  const image = visual.locator('.preview-canvas img');
  await expect(image).toBeVisible();
  await expect(visual.getByText('Page 1 of 1')).toBeVisible();
  const width96 = await image.evaluate((img) => img.naturalWidth);
  await visual.getByRole('combobox', { name: 'Zoom' }).click();
  await visual.getByRole('option', { name: '150%' }).click();
  await expect.poll(() => image.evaluate((img) => img.naturalWidth)).toBeGreaterThan(width96);
  await visual.getByRole('button', { name: 'Text' }).click();
  await expect(visual.getByText('Extracted text fallback')).toBeVisible();
  await visual.getByRole('button', { name: 'Pages' }).click();
  await expect(image).toBeVisible();
  assert.deepEqual(await commands(visual), ['attachment_preview_source', 'read_attachment']);
  await visual.getByRole('button', { name: 'Close attachment preview' }).click();
  await expect.poll(() => visual.evaluate(() => window.previewParentClosed)).toBe(1);
  assert.deepEqual(await visual.evaluate(() => window.workers), { created: 1, terminated: 1 });
  await visual.close();
  console.log('PASS page rendering, zoom, text switch and close release the worker');

  const switched = await openFixture('visual', pdf);
  await expect(switched.locator('.preview-canvas img')).toBeVisible();
  await switched.evaluate((next) => window.setAttachment(next), secondPdf);
  await expect(switched.getByText('second.pdf')).toBeVisible();
  await expect(switched.locator('.preview-canvas img')).toBeVisible();
  await expect.poll(() => switched.evaluate(() => window.workers)).toEqual({ created: 2, terminated: 1 });
  assert.deepEqual(
    (await switched.evaluate(() => window.previewCalls)).map((call) => [call.command, call.args.attachmentId]),
    [
      ['attachment_preview_source', 7],
      ['attachment_preview_source', 9],
    ],
  );
  await switched.close();
  console.log('PASS switching attachments terminates the previous worker');

  const late = await openFixture('slow', file(11, 'slow.pdf'));
  await expect.poll(() => commands(late)).toEqual(['attachment_preview_source']);
  await late.evaluate(() => window.unmountPreview());
  await late.evaluate(() => window.resolveSlowSource());
  await late.waitForTimeout(500);
  assert.deepEqual(await late.evaluate(() => window.workers), { created: 0, terminated: 0 });
  await late.close();
  console.log('PASS a late source after the panel was destroyed starts no engine');

  const corrupt = await openFixture('corrupt', pdf);
  await expect(corrupt.getByRole('alert')).toBeVisible();
  await corrupt.getByRole('button', { name: 'Show extracted text' }).click();
  await expect(corrupt.getByText('Extracted text fallback')).toBeVisible();
  assert.deepEqual(await corrupt.evaluate(() => window.workers), { created: 1, terminated: 1 });
  await corrupt.close();
  console.log('PASS a document the engine rejects shows an error and offers the text fallback');

  const plain = await openFixture('visual', text);
  await expect(plain.getByText('Extracted text fallback')).toBeVisible();
  assert.deepEqual(await commands(plain), ['attachment_preview_source', 'read_attachment']);
  assert.deepEqual(await plain.evaluate(() => window.workers), { created: 0, terminated: 0 });
  await plain.close();
  console.log('PASS unsupported attachments ask the backend once and keep the text preview');

  // After a save the notice offers Open and Show in folder; they name the file by the token the save
  // returned, never by a path.
  const saved = await openFixture('visual', pdf);
  await expect(saved.locator('.preview-canvas img')).toBeVisible();
  await saved.getByRole('button', { name: 'Download attachment' }).click();
  await expect.poll(() => saved.evaluate(() => window.previewNotices.length)).toBe(1);
  const [notice] = await saved.evaluate(() => window.previewNotices);
  assert.deepEqual(notice, {
    message: 'Attachment saved: C:\\Saved\\report.pdf',
    error: false,
    actions: ['Open', 'Show in folder'],
  });
  await saved.evaluate(() => window.runNoticeAction('Show in folder'));
  await expect.poll(() => commands(saved)).toContain('saved_file_reveal');
  await saved.evaluate(() => window.runNoticeAction('Open'));
  await expect.poll(() => commands(saved)).toContain('saved_file_open');
  const calls = await saved.evaluate(() => window.previewCalls);
  assert.deepEqual(calls.find((call) => call.command === 'attachment_save').args.attachmentId, 7);
  assert.deepEqual(calls.find((call) => call.command === 'saved_file_reveal').args, { token: 41 });
  assert.deepEqual(calls.find((call) => call.command === 'saved_file_open').args, { token: 41 });
  assert.equal(await saved.evaluate(() => window.previewNotices.length), 1, 'an action that works says nothing more');
  await saved.close();
  console.log('PASS a saved attachment offers Open and Show in folder, which name the file by its token');

  const cancelled = await openFixture('visual', pdf, { save: 'cancel' });
  await expect(cancelled.locator('.preview-canvas img')).toBeVisible();
  await cancelled.getByRole('button', { name: 'Download attachment' }).click();
  await expect.poll(() => commands(cancelled)).toContain('attachment_save');
  await cancelled.waitForTimeout(300);
  assert.equal(await cancelled.evaluate(() => window.previewNotices.length), 0, 'a cancelled dialog offers nothing');
  await cancelled.close();
  console.log('PASS a cancelled save offers nothing');

  const gone = await openFixture('visual', pdf, { launch: 'fail' });
  await expect(gone.locator('.preview-canvas img')).toBeVisible();
  await gone.getByRole('button', { name: 'Download attachment' }).click();
  await expect.poll(() => gone.evaluate(() => window.previewNotices.length)).toBe(1);
  await gone.evaluate(() => window.runNoticeAction('Open'));
  await expect.poll(() => gone.evaluate(() => window.previewNotices.length)).toBe(2);
  assert.deepEqual((await gone.evaluate(() => window.previewNotices))[1], {
    message: 'The file is gone: it was moved or deleted.',
    error: true,
    actions: [],
  });
  await gone.close();
  console.log('PASS an action that fails says why in an error notice');
} finally {
  await browser?.close();
  server.kill();
}
