// Every previewable kind renders in its embedded WebAssembly engine, under the app's real CSP.
// Only the desktop bridge is simulated: `attachment_preview_source` answers with the document.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { samplePdf, sourceFrame } from './support/sample-documents.mjs';
import { sampleDocx, samplePptx, sampleXlsx } from './support/office-documents.mjs';

const root = path.resolve(import.meta.dirname, '..');
const port = 28165;
const url = `http://127.0.0.1:${port}`;
const csp = JSON.parse(readFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), 'utf8')).app.security.csp;
assert.match(csp, /script-src 'self' 'wasm-unsafe-eval'/);
assert.doesNotMatch(csp, /'unsafe-eval'/);

const documents = {
  pdf: { name: 'report.pdf', bytes: samplePdf(['ArtCraft embedded smoke test', 'PdfCraft in WebAssembly']) },
  docx: { name: 'letter.docx', bytes: sampleDocx() },
  xlsx: { name: 'budget.xlsx', bytes: sampleXlsx() },
  pptx: { name: 'deck.pptx', bytes: samplePptx() },
};
const frames = Object.fromEntries(
  Object.entries(documents).map(([kind, { bytes }], index) => [index + 1, sourceFrame(kind, bytes)]),
);
frames[9] = sourceFrame('pdf', Buffer.from('%PDF-1.7 this is not a document'));
frames[10] = sourceFrame('docx', Buffer.from('this is not a zip archive'));

const server = spawn(
  process.execPath,
  ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'],
  { cwd: root, windowsHide: true, stdio: 'pipe' },
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
    if (attempt === 59) throw new Error(`Vite did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

let browser;
try {
  await waitForServer();
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1200, height: 900 } });
  const pageErrors = [];
  await context.route(`${url}/**`, async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, headers: { ...response.headers(), 'content-security-policy': csp } });
  });
  await context.addInitScript((sources) => {
    window.isTauri = true;
    window.previewCalls = [];
    window.previewNotices = [];
    window.previewParentClosed = 0;
    window.cspViolations = [];
    document.addEventListener('securitypolicyviolation', (event) =>
      window.cspViolations.push(`${event.violatedDirective} ${event.blockedURI} ${event.sourceFile}:${event.lineNumber} ${String(event.sample).slice(0, 120)}`),
    );
    window.__TAURI_INTERNALS__ = {
      invoke(command, args = {}) {
        window.previewCalls.push({ command, args: structuredClone(args) });
        if (command === 'attachment_preview_source') {
          return Promise.resolve(Uint8Array.from(sources[args.attachmentId]).buffer);
        }
        if (command === 'read_attachment') return Promise.resolve('Extracted text fallback');
        throw new Error(`Unexpected desktop command: ${command}`);
      },
    };
  }, frames);
  const open = async (attachment) => {
    const page = await context.newPage();
    page.setDefaultTimeout(30000);
    page.on('pageerror', (error) => pageErrors.push(String(error)));
    await page.goto(`${url}/tests/attachment-preview.html?${new URLSearchParams({ attachment: JSON.stringify(attachment) })}`, {
      waitUntil: 'domcontentloaded',
    });
    return page;
  };
  const attachment = (id, name) => ({ id, name, size: 1500, mime: 'application/octet-stream', path: null, inline_text: null });

  for (const [index, [kind, { name }]] of Object.entries(documents).entries()) {
    const page = await open(attachment(index + 1, name));
    const image = page.locator('.preview-canvas img');
    await expect(image).toBeVisible();
    await expect(page.getByText(/Page 1 of \d+/)).toBeVisible();
    const small = await image.evaluate((img) => img.naturalWidth);
    assert.ok(small > 100, `${kind}: the first page is a real image (${small}px wide)`);
    await page.getByRole('combobox', { name: 'Zoom' }).click();
    await page.getByRole('option', { name: '150%' }).click();
    await expect.poll(() => image.evaluate((img) => img.naturalWidth)).toBeGreaterThan(small);
    assert.deepEqual(
      (await page.evaluate(() => window.previewCalls)).map((call) => call.command),
      ['attachment_preview_source'],
    );
    // The page that mounts the component is a test harness under Vite, whose React refresh preamble is an inline script of the
    // page itself (`...html:4`); what this test is about are the engines, which must not break the app's CSP.
    const violations = (await page.evaluate(() => window.cspViolations)).filter((violation) => !/^script-src-elem inline \S+\.html:\d+/.test(violation));
    assert.deepEqual(violations, [], `${kind}: no CSP violation`);
    await page.close();
    console.log(`PASS ${kind} renders in WebAssembly under the app CSP`);
  }

  for (const [id, name] of [[9, 'broken.pdf'], [10, 'broken.docx']]) {
    const page = await open(attachment(id, name));
    await expect(page.getByRole('alert')).toBeVisible();
    await page.getByRole('button', { name: 'Show extracted text' }).click();
    await expect(page.getByText('Extracted text fallback')).toBeVisible();
    assert.deepEqual(await page.evaluate(() => window.cspViolations), []);
    await page.close();
    console.log(`PASS ${name}: a rejected document shows an error and keeps the text fallback`);
  }
  assert.deepEqual(pageErrors, []);
} finally {
  await browser?.close();
  server.kill();
}
