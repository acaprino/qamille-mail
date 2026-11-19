// Attached messages (.eml such as a PEC postacert.eml) open in the side panel as
// formatted emails; their own attachments open in the same panel with a way back.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { samplePdf, sourceFrame } from './support/sample-documents.mjs';

const port = 28165;
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
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1400, height: 900 } });
  await context.addInitScript((pdfFrame) => {
    window.isTauri = true;
    window.panelCalls = [];
    window.panelNotices = [];
    window.panelClosed = 0;
    const pec = {
      subject: 'Avviso di accertamento',
      from_name: 'Ufficio Tributi',
      from_addr: 'protocollo@pec.comune.example',
      to: ['Mario Rossi <mario.rossi@pec.example>'],
      cc: [],
      date: 1793790000,
      body: 'Gentile contribuente, si trasmette in allegato l’avviso.',
      body_html: '',
      attachments: [
        { index: 0, name: 'Avviso.pdf', size: 2048, mime: 'application/pdf', message: false },
        { index: 1, name: 'inoltro.eml', size: 900, mime: 'message/rfc822', message: true },
        { index: 2, name: 'rotto.eml', size: 300, mime: 'application/octet-stream', message: true },
      ],
    };
    const inner = { ...pec, subject: 'Messaggio interno', body: 'Testo interno', attachments: [] };
    window.__TAURI_INTERNALS__ = {
      invoke(command, args = {}) {
        const captured = structuredClone(args);
        window.panelCalls.push({ command, args: captured });
        if (command === 'read_attached_message') {
          const key = (captured.path || []).join('/');
          if (key === '') return Promise.resolve(pec);
          if (key === '1') return Promise.resolve(inner);
          if (key === '2') return Promise.reject({ message: 'The attachment is not an email message.', code: 'validation' });
          return Promise.reject({ message: 'The attachment is no longer available.', code: 'validation' });
        }
        if (command === 'attachment_preview_source') {
          if ((captured.path || []).join('/') !== '0')
            return Promise.reject({ message: 'No visual preview for this file type.', code: 'preview_unsupported' });
          return Promise.resolve(Uint8Array.from(pdfFrame).buffer);
        }
        if (command === 'read_attachment') return Promise.resolve('Testo grezzo del file');
        throw new Error(`Unexpected desktop command: ${command}`);
      },
    };
  }, sourceFrame('pdf', samplePdf(['Avviso di accertamento'])));

  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  await page.goto(`${url}/tests/attachment-panel.html`, { timeout: 30000, waitUntil: 'domcontentloaded' });
  const panel = page.getByRole('dialog', { name: 'postacert.eml' });

  await expect(panel.getByRole('heading', { name: 'Avviso di accertamento' })).toBeVisible();
  await expect(panel.getByText('Ufficio Tributi', { exact: true })).toBeVisible();
  await expect(panel.getByText('<protocollo@pec.comune.example>')).toBeVisible();
  // The From / To / Date box has one row per field (label, then value).
  await expect(panel.locator('.header-row', { hasText: /^To/ })).toContainText('Mario Rossi <mario.rossi@pec.example>');
  await expect(panel.getByText('Gentile contribuente')).toBeVisible();
  await expect(panel.getByRole('button', { name: /Avviso\.pdf.*Preview/ })).toBeVisible();
  await expect(panel.getByRole('button', { name: /inoltro\.eml.*Open message/ })).toBeVisible();
  assert.deepEqual(await page.evaluate(() => window.panelCalls), [
    { command: 'read_attached_message', args: { emailId: 42, attachmentId: 5 } },
  ]);
  console.log('PASS an attached .eml opens as a formatted email with its attachments');

  await panel.getByRole('button', { name: /Avviso\.pdf.*Preview/ }).click();
  const pdf = page.getByRole('dialog', { name: 'Avviso.pdf' });
  await expect(pdf.locator('.preview-canvas img')).toBeVisible();
  await expect(pdf.getByRole('button', { name: 'Close attachment preview' })).toHaveCount(0);
  const open = (await page.evaluate(() => window.panelCalls)).find((call) => call.command === 'attachment_preview_source');
  assert.deepEqual(open.args, { emailId: 42, attachmentId: 5, path: [0] });
  const focusInPanel = () => page.evaluate(() => !!document.activeElement?.closest('.attachment-panel'));
  await expect.poll(focusInPanel).toBe(true);
  console.log('PASS a document inside the attached email opens in the same panel, focus stays in it');

  await pdf.getByRole('button', { name: /postacert\.eml/ }).click();
  await expect(panel.getByRole('heading', { name: 'Avviso di accertamento' })).toBeVisible();
  await expect.poll(focusInPanel).toBe(true);
  const rootReads = (await page.evaluate(() => window.panelCalls)).filter(
    (call) => call.command === 'read_attached_message' && !call.args.path,
  );
  assert.equal(rootReads.length, 1, 'going back reuses the parsed message');
  console.log('PASS back returns to the attached email from memory and releases the document preview');

  await panel.getByRole('button', { name: /rotto\.eml.*Open message/ }).click();
  const broken = page.getByRole('dialog', { name: 'rotto.eml' });
  await expect(broken.getByRole('alert')).toContainText('The attachment is not an email message.');
  await broken.getByRole('button', { name: 'Open as a file' }).click();
  await expect(broken.getByText('Testo grezzo del file')).toBeVisible();
  const text = (await page.evaluate(() => window.panelCalls)).find((call) => call.command === 'read_attachment');
  assert.deepEqual(text.args, { emailId: 42, attachmentId: 5, path: [2] });
  await broken.getByRole('button', { name: /postacert\.eml/ }).click();
  await expect(panel.getByRole('heading', { name: 'Avviso di accertamento' })).toBeVisible();
  console.log('PASS an .eml that is not a valid email can still be opened as a file');

  await panel.getByRole('button', { name: /inoltro\.eml.*Open message/ }).click();
  const inner = page.getByRole('dialog', { name: 'inoltro.eml' });
  await expect(inner.getByRole('heading', { name: 'Messaggio interno' })).toBeVisible();
  const nested = (await page.evaluate(() => window.panelCalls)).filter((call) => call.command === 'read_attached_message');
  assert.deepEqual(nested.at(-1).args, { emailId: 42, attachmentId: 5, path: [1] });
  console.log('PASS an email nested inside the attached email opens one level deeper');

  // Focus lost to the page itself must not keep the panel from closing.
  await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  assert.equal(await focusInPanel(), false);
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => window.panelClosed)).toBe(1);
  await page.close();
  console.log('PASS Escape closes the panel');
} finally {
  await browser?.close();
  server.kill();
}
