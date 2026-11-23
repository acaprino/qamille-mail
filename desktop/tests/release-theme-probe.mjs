// Probe: the packaged Release executable ships the automatic appearance feature.
// Launches release/Qamille.exe in a disposable workspace and checks the
// Settings → Appearance select and the system-theme follow behaviour, then opens a Select, a Modal, a
// Menu, a Tooltip and a DateField in the real executable and FAILS on any Content Security Policy
// violation (console messages, `securitypolicyviolation` events, the webview Log domain).
// Safety: the same as the native tests: synthetic Sample workspace only (src/lib/sample.json), every
// outgoing HTTP(S) request pointed at a dead local proxy, a folder under checks/.
// Run from desktop/: node tests/release-theme-probe.mjs
import { chromium } from '@playwright/test';
import fs from 'node:fs/promises';
import zlib from 'node:zlib';
import net from 'node:net';
import path from 'node:path';
import { spawn } from 'node:child_process';

// Minimal PNG reader (8-bit, non-interlaced RGB/RGBA: what Chromium writes) to look at the edge pixels.
function readPng(buffer) {
  let at = 8, width = 0, height = 0, channels = 0;
  const parts = [];
  while (at < buffer.length) {
    const length = buffer.readUInt32BE(at), type = buffer.toString('ascii', at + 4, at + 8);
    const body = buffer.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') { width = body.readUInt32BE(0); height = body.readUInt32BE(4); channels = body[9] === 6 ? 4 : body[9] === 2 ? 3 : 0; }
    if (type === 'IDAT') parts.push(body);
    at += 12 + length;
  }
  if (!channels) throw new Error('Unsupported PNG colour type');
  const raw = zlib.inflateSync(Buffer.concat(parts));
  const stride = width * channels, pixels = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)], line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? pixels[y * stride + x - channels] : 0, b = y ? pixels[(y - 1) * stride + x] : 0, c = x >= channels && y ? pixels[(y - 1) * stride + x - channels] : 0;
      const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
      const add = [0, a, b, (a + b) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? b : c][filter];
      pixels[y * stride + x] = (line[x] + add) & 255;
    }
  }
  return { width, height, at: (x, y) => [...pixels.subarray(y * stride + x * channels, y * stride + x * channels + 3)] };
}

const root = path.resolve(import.meta.dirname, '../..');
const data = path.join(root, 'checks/release-theme-probe');
const executable = process.env.QAMILLE_RELEASE_EXE || path.join(root, 'release/Qamille.exe');
await fs.rm(data, { recursive: true, force: true });
await fs.mkdir(path.join(data, 'webview'), { recursive: true });
const seed = JSON.parse(await fs.readFile(path.join(root, 'desktop/src/lib/sample.json'), 'utf8'));
seed.settings = { ...seed.settings, locale: 'en-US' };
await fs.writeFile(path.join(data, 'store.json'), JSON.stringify(seed));
const dead = 'http://127.0.0.1:9';
const server = net.createServer();
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
await new Promise((resolve) => server.close(resolve));
const child = spawn(executable, [], {
  cwd: root,
  env: {
    ...process.env,
    QAMILLE_DATA: data,
    QAMILLE_DEMO: '1',
    HTTP_PROXY: dead, HTTPS_PROXY: dead, ALL_PROXY: dead, NO_PROXY: '',
    WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --lang=en-US`,
  },
  windowsHide: true,
  stdio: 'pipe',
});
let out = '';
child.stdout.on('data', (d) => (out += d));
child.stderr.on('data', (d) => (out += d));
let pageErrors = [];
const cspProblems = [];
const cspText = /Content Security Policy|Refused to (load|apply|execute|connect|frame)/i;
try {
  let ready = false;
  for (let i = 0; i < 120 && !ready; i++) {
    try {
      ready = (await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })).ok;
    } catch { /* WebView is starting. */ }
    if (!ready) await new Promise((resolve) => setTimeout(resolve, 250));
  }
  if (!ready) throw new Error(`Release window did not open: ${out}`);
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = browser.contexts()[0];
  let page = context.pages().find((candidate) => candidate.url() !== 'about:blank');
  for (let i = 0; i < 60 && !page; i++) {
    await new Promise((resolve) => setTimeout(resolve, 250));
    page = context.pages().find((candidate) => candidate.url() !== 'about:blank');
  }
  if (!page) page = context.pages()[0] || await context.waitForEvent('page');
  page.setDefaultTimeout(15000);
  page.on('pageerror', (error) => {
    pageErrors.push(error.message);
    if (cspText.test(error.message)) cspProblems.push(`pageerror: ${error.message}`);
  });
  page.on('console', (message) => {
    if (cspText.test(message.text())) cspProblems.push(`console ${message.type()}: ${message.text()}`);
  });
  // The webview's own Log domain reports CSP violations that never reach console.* (and replays the
  // ones raised before this connection).
  const cdp = await context.newCDPSession(page);
  cdp.on('Log.entryAdded', ({ entry }) => {
    if (entry.source === 'security' || cspText.test(entry.text)) cspProblems.push(`log ${entry.source}/${entry.level}: ${entry.text} ${entry.url ?? ''}`);
  });
  await cdp.send('Log.enable');
  await page.addInitScript(() => {
    window.__cspViolations = [];
    document.addEventListener('securitypolicyviolation', (event) =>
      window.__cspViolations.push(`${event.violatedDirective} ${event.blockedURI}`));
  });
  const violationsSoFar = () => page.evaluate(() => window.__cspViolations ?? []).catch(() => []);
  await page.waitForLoadState('domcontentloaded');
  console.log('page url:', page.url());
  if (!/^https?:\/\/tauri\.localhost\//.test(page.url())) throw new Error(`The page is not served by the executable (custom-protocol): ${page.url()}`);
  await page.screenshot({ path: path.join(root, 'checks/release-theme-probe-start.png') }).catch(() => {});
  await page.emulateMedia({ colorScheme: 'dark' });
  const settingsButton = page.locator('button.profile');
  await settingsButton.click();
  await page.screenshot({ path: path.join(root, 'checks/release-theme-probe-settings.png') }).catch(() => {});
  const combo = page.getByRole('combobox', { name: /Appearance|Aspetto/, exact: true });
  await combo.waitFor();
  console.log('PASS release executable exposes the Appearance select');
  await combo.click();
  await page.getByRole('option', { name: /^(Dark|Scuro)$/ }).click();
  await page.waitForFunction(() => document.documentElement.dataset.scheme === 'dark');
  await combo.click();
  await page.getByRole('option', { name: /^(Automatic \(system\)|Automatico \(come Windows\))$/ }).click();
  await page.waitForFunction(() => document.documentElement.dataset.scheme === 'dark');
  await page.emulateMedia({ colorScheme: 'light' });
  await page.waitForFunction(() => document.documentElement.dataset.scheme === 'light');
  console.log('PASS release executable follows the system theme in Automatic mode');

  // ---- overlays: each one must open in the real webview without a CSP violation ----
  // A fresh load installs the violation listener before any script runs.
  await page.reload();
  await page.waitForFunction(() => !!window.__TAURI_INTERNALS__);
  await page.locator('.mail-row').first().waitFor();

  // ---- frame: the page fills the window and the sheet reaches the right and bottom edges ----
  // Measured through the page (CDP), never with a desktop screenshot. Pictures: checks/compatto/exe-<theme>.png.
  await fs.mkdir(path.join(root, 'checks/compatto'), { recursive: true });
  for (const scheme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForFunction((want) => document.documentElement.dataset.scheme === want, scheme);
    const frame = await page.evaluate(() => {
      const box = (element) => { const r = element.getBoundingClientRect(); return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height }; };
      const sheet = document.querySelector('.workspace-content');
      return { inner: { width: window.innerWidth, height: window.innerHeight }, root: box(document.documentElement), sheet: sheet && box(sheet) };
    });
    console.log(`frame (${scheme}):`, JSON.stringify(frame));
    if (frame.root.left !== 0 || frame.root.top !== 0 || frame.root.width !== frame.inner.width || frame.root.height !== frame.inner.height)
      throw new Error(`The page does not fill the window (${scheme}): ${JSON.stringify(frame)}`);
    if (!frame.sheet) throw new Error('.workspace-content is missing');
    if (Math.abs(frame.sheet.right - frame.inner.width) > 0.5 || Math.abs(frame.sheet.bottom - frame.inner.height) > 0.5)
      throw new Error(`.workspace-content does not reach the right and bottom edge (${scheme}): ${JSON.stringify(frame)}`);
    const shot = path.join(root, `checks/compatto/exe-${scheme}.png`);
    await page.screenshot({ path: shot });
    // No light line at the window edges: the left column is the sidebar colour, the right column and the
    // bottom row the sheet colour (computed styles of the page), at several positions.
    const colours = await page.evaluate(() => {
      const rgb = (selector) => getComputedStyle(document.querySelector(selector)).backgroundColor.match(/[\d.]+/g).slice(0, 3).map(Number);
      return { sidebar: rgb('aside.sidebar'), sheet: rgb('.workspace-content') };
    });
    const png = readPng(await fs.readFile(shot));
    const near = (a, b) => a.every((value, i) => Math.abs(value - b[i]) <= 3);
    const bad = [];
    for (const y of [120, 300, 500, 700, 850, png.height - 2]) if (!near(png.at(0, y), colours.sidebar)) bad.push(`left x=0 y=${y}: ${png.at(0, y)} (sidebar ${colours.sidebar})`);
    for (const y of [120, 300, 500, 700, 850]) if (!near(png.at(png.width - 1, y), colours.sheet)) bad.push(`right x=${png.width - 1} y=${y}: ${png.at(png.width - 1, y)} (sheet ${colours.sheet})`);
    for (const x of [400, 700, 1000, 1300]) if (!near(png.at(x, png.height - 1), colours.sheet)) bad.push(`bottom x=${x} y=${png.height - 1}: ${png.at(x, png.height - 1)} (sheet ${colours.sheet})`);
    if (bad.length) throw new Error(`A line is painted at the window edge (${scheme}):\n  ${bad.join('\n  ')}`);
  }
  console.log('PASS the page fills the window, the sheet reaches the right and bottom edges and no line is painted at the edges (light and dark)');
  await page.emulateMedia({ colorScheme: 'light' });

  const either = (it, en) => new RegExp(`^(${it}|${en})$`, 'i');
  const shown = async (locator, what) => {
    await locator.first().waitFor({ state: 'visible', timeout: 8000 }).catch(() => {
      throw new Error(`${what} did not open`);
    });
  };

  // Select: the Appearance list in Settings (Ark select content, in the portal).
  await page.locator('button.profile').click();
  const appearance = page.getByRole('combobox', { name: /Appearance|Aspetto/, exact: true });
  await appearance.click();
  await shown(page.locator('[data-slot="select-content"]'), 'The Select list');
  await page.keyboard.press('Escape');
  console.log('PASS Select opens its list');

  // Modal: the new-contact dialog.
  await page.locator('aside.sidebar button').filter({ hasText: either('Contacts', 'Rubrica') }).first().click();
  await page.locator('.contacts-page button').filter({ hasText: either('New contact', 'Nuovo contatto') }).first().click();
  await shown(page.locator('[data-slot="dialog-content"], dialog[open]'), 'The Modal');
  await page.keyboard.press('Escape');
  console.log('PASS Modal opens');

  // Mail view again: Menu and Tooltip live in the reader, the DateField in the search filters.
  await page.reload();
  await page.locator('.mail-row').first().waitFor();
  await page.locator('.mail-row').first().locator('button.mail-open').click();
  await page.locator('.message-title').waitFor();
  await page.getByRole('button', { name: either('More message actions', 'Altre azioni sul messaggio') }).first().click();
  await shown(page.locator('[data-scope="menu"][data-part="content"]'), 'The Menu');
  await page.keyboard.press('Escape');
  console.log('PASS Menu opens');

  const tip = page.locator('[data-scope="tooltip"][data-part="content"]');
  const icons = page.locator('section.reader button.icon-button');
  let tipShown = false;
  for (let i = 0, n = await icons.count(); i < n && !tipShown; i++) {
    const icon = icons.nth(i);
    if (!(await icon.isVisible()) || (await icon.isDisabled())) continue;
    await icon.hover();
    tipShown = await tip.first().waitFor({ state: 'visible', timeout: 1200 }).then(() => true, () => false);
  }
  if (!tipShown) throw new Error('No icon button of the reader shows a Tooltip');
  await page.mouse.move(0, 0);
  console.log('PASS Tooltip shows');

  await page.getByRole('button', { name: /^(When|Quando)/ }).click();
  const dateField = page.locator('[data-scope="date-picker"][data-part="input"]').first();
  await dateField.waitFor();
  await page.locator('[data-scope="date-picker"][data-part="control"]').first().getByRole('button').click();
  await shown(page.locator('[data-scope="date-picker"][data-part="content"]'), 'The DateField calendar');
  await page.keyboard.press('Escape');
  console.log('PASS DateField opens its calendar');

  // QAMILLE_PROBE_SELFTEST=1 provokes a violation (a font from another host) to prove the check bites.
  if (process.env.QAMILLE_PROBE_SELFTEST) {
    await page.evaluate(() => {
      const style = document.createElement('style');
      style.textContent = '@font-face{font-family:probe;src:url(https://fonts.example.invalid/x.woff2)}body{font-family:probe}';
      document.head.append(style);
      return document.fonts.load('12px probe').catch(() => {});
    });
  }
  await new Promise((resolve) => setTimeout(resolve, 500));
  const events = await violationsSoFar();
  for (const event of events) cspProblems.push(`securitypolicyviolation: ${event}`);
  if (cspProblems.length) throw new Error(`Content Security Policy violations:\n  ${cspProblems.join('\n  ')}`);
  console.log('PASS no Content Security Policy violation while opening Select, Modal, Menu, Tooltip and DateField');
  await browser.close();
} catch (error) {
  console.error(`PROBE FAIL: ${error.message}\npage errors: ${JSON.stringify(pageErrors ?? [])}\napp output:\n${out}`);
  throw error;
} finally {
  child.kill();
  await new Promise((resolve) => setTimeout(resolve, 2000));
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      await fs.rm(data, { recursive: true, force: true });
      break;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 1500));
    }
  }
}
