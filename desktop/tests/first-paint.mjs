// The first paint of the production build (docs/aspetto-nativo.md rule 5, the Sigrid plan's Task 2): before any module of
// the app runs, with the entry chunk blocked, the page already has its scheme (`data-scheme`, written by
// public/theme-boot.js) and the ground of that scheme, so a dark workspace never starts on a white page.
//   node tests/first-paint.mjs        (builds into desktop/.cache, serves it on a free port, removes it at the end)
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
fs.mkdirSync(path.join(desktop, '.cache'), { recursive: true });
const outDir = fs.mkdtempSync(path.join(desktop, '.cache', 'first-paint-'));

// Compatto's page colour in each scheme: the --color-paper of src/theme/compatto.css (Sigrid's paper).
const GROUND = { light: 'rgb(246, 247, 249)', dark: 'rgb(18, 22, 30)' };

let server;
let browser;
try {
  const { build, preview } = await import('vite');
  await build({ root: desktop, configFile: path.join(desktop, 'vite.config.ts'), logLevel: 'error', build: { outDir, emptyOutDir: true, reportCompressedSize: false } });
  server = await preview({ root: desktop, configFile: path.join(desktop, 'vite.config.ts'), logLevel: 'error', build: { outDir }, preview: { port: 0, strictPort: false, host: '127.0.0.1' } });
  const base = server.resolvedUrls?.local?.[0]?.replace(/\/$/, '');
  assert.ok(base, 'vite preview did not report its address');

  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  for (const scheme of ['dark', 'light']) {
    const context = await browser.newContext({ colorScheme: scheme, viewport: { width: 1280, height: 800 } });
    const page = await context.newPage();
    // The entry chunk (and every other script of the app) never arrives: what shows is the HTML, the CSS and the boot script.
    await page.route(/\/assets\/.*\.js(\?.*)?$/, (route) => route.abort());
    await page.goto(`${base}/index.html`, { waitUntil: 'domcontentloaded' });
    const state = await page.evaluate(() => ({
      scheme: document.documentElement.dataset.scheme,
      theme: document.documentElement.dataset.theme,
      density: document.documentElement.dataset.density,
      body: getComputedStyle(document.body).backgroundColor,
      html: getComputedStyle(document.documentElement).backgroundColor,
    }));
    assert.equal(state.scheme, scheme, `${scheme}: data-scheme before the app runs`);
    assert.equal(state.theme, 'compatto');
    assert.equal(state.density, 'compatto');
    assert.equal(state.body, GROUND[scheme], `${scheme}: the body ground before the app runs`);
    assert.equal(state.html, GROUND[scheme], `${scheme}: the html ground before the app runs`);
    console.log(`PASS ${scheme}: data-scheme and the ground ${GROUND[scheme]} are there before any script of the app`);
    await context.close();
  }
} finally {
  await browser?.close();
  await server?.close();
  fs.rmSync(outDir, { recursive: true, force: true });
}
