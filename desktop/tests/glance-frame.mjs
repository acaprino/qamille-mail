// The restyled frame (spec 2026-11-07 §4.1): one ground, one white sheet, IBM Plex Sans for
// everything scanned and for titles (spec 2026-11-08 §3), one blue, no resting shadows. Synthetic data only.
import { chromium, expect } from '@playwright/test';
import assert from './support/soft-assert.mjs';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const port = 28171;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');

// The window colour shows before the page paints: it is the theme's --color-paper, the colour the page paints
// first (docs/aspetto-nativo.md rule 6, kept in step by tests/native-feel.mjs).
const tauri = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
assert.equal(tauri.app.windows[0].backgroundColor.toLowerCase(), '#f6f7f9', 'window backgroundColor follows --color-paper');
console.log('PASS The window background follows --color-paper');

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', data => output += data);
server.stderr.on('data', data => output += data);
let browser;
try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const errors = [];

  for (const [width, height] of [[1440, 900], [1200, 800], [1100, 720], [1000, 640]]) {
    const page = await browser.newPage({ locale: 'en-US', viewport: { width, height } });
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(url);
    await expect(page.locator('.mail-row').first()).toBeVisible();
    // The first message opens by itself: the reading pane is on screen.
    await expect(page.locator('.message-title')).toBeVisible();
    await expect(page.locator('.add-sender')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    const at = `${width}x${height}`;

    const frame = await page.evaluate(() => {
      const style = (selector) => getComputedStyle(document.querySelector(selector));
      const token = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
      const hidden = (selector) => [...document.querySelectorAll(selector)].map(el => getComputedStyle(el).display);
      return {
        bodyFont: style('body').fontFamily,
        titleFont: style('.list-heading h1').fontFamily,
        plexLoaded: [...document.fonts].some((face) => face.family.replace(/"/g, '') === 'IBM Plex Sans' && face.status === 'loaded'),
        sheetRadius: style('.workspace-content').borderTopLeftRadius,
        sheetBorder: style('.workspace-content').borderTopWidth,
        sheetRight: window.innerWidth - document.querySelector('.workspace-content').getBoundingClientRect().right,
        sheetBottom: window.innerHeight - document.querySelector('.workspace-content').getBoundingClientRect().bottom,
        radiusToken: token('--radius-sheet'),
        ground: token('--color-paper'),
        groundPaint: style('.app-shell').backgroundColor,
        sidebarEdge: style('.sidebar').borderRightWidth,
        shadows: [...document.querySelectorAll('.primary, .compose-main, .brand-mark')].map(el => getComputedStyle(el).boxShadow),
        path: hidden('.workspace-name'),
        accountMail: hidden('.account-details small'),
        shortcutHints: hidden('.reader-reply kbd'),
        composeHint: hidden('.compose-main kbd'),
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
      };
    });
    assert.match(frame.bodyFont, /^"?IBM Plex Sans"?/, `${at}: body is set in IBM Plex Sans (${frame.bodyFont})`);
    assert.match(frame.titleFont, /^"?IBM Plex Sans"?/, `${at}: the list title is set in IBM Plex Sans (${frame.titleFont})`);
    assert.ok(frame.plexLoaded, `${at}: the face is served by the app`);
    assert.equal(frame.radiusToken, '14px');
    assert.equal(frame.sheetRadius, '0px', `${at}: the sheet is full bleed, without rounded corners`);
    assert.equal(frame.sheetBorder, '1px', `${at}: the sheet keeps only its top hairline`);
    assert.equal(Math.round(frame.sheetRight), 0, `${at}: the sheet reaches the right edge`);
    assert.equal(Math.round(frame.sheetBottom), 0, `${at}: the sheet reaches the bottom edge`);
    assert.equal(frame.ground, '#f6f7f9');
    assert.equal(frame.groundPaint, 'rgb(246, 247, 249)', `${at}: the shell is painted with the ground`);
    assert.equal(frame.sidebarEdge, '0px', `${at}: no line between the sidebar and the work`);
    assert.ok(frame.shadows.length > 0 && frame.shadows.every(s => s === 'none'), `${at}: no resting shadow on blue buttons: ${frame.shadows}`);
    assert.ok(frame.path.length > 0 && frame.path.every(d => d === 'none'), `${at}: the path in the top bar is hidden`);
    assert.ok(frame.accountMail.length > 0 && frame.accountMail.every(d => d === 'none'), `${at}: the address under the account is hidden`);
    assert.ok(frame.shortcutHints.every(d => d === 'none'), `${at}: no shortcut hint inside the reader buttons`);
    assert.ok(width < 1000 || frame.composeHint.every(d => d !== 'none'), `${at}: New message shows its N (Compatto sidebar)`);
    assert.ok(frame.scrollWidth <= frame.innerWidth, `${at}: the page does not scroll sideways (${frame.scrollWidth} > ${frame.innerWidth})`);
    console.log(`PASS Frame, type and tokens at ${at}`);

    // The reading pane (§4.5): one title, an amber priority tag before the path, one blue action.
    const color = (name) => page.evaluate((value) => {
      const probe = document.createElement('i');
      probe.style.color = `var(${value})`;
      document.body.append(probe);
      const resolved = getComputedStyle(probe).color;
      probe.remove();
      return resolved;
    }, name);
    const reading = await page.evaluate(() => {
      const style = (selector) => getComputedStyle(document.querySelector(selector));
      const box = (selector) => document.querySelector(selector).getBoundingClientRect();
      const path = document.querySelector('.message-location > span:not(.message-label)');
      return {
        titleFont: style('.message-title').fontFamily,
        titleSize: style('.message-title').fontSize,
        label: document.querySelector('.message-label').className,
        labelBackground: style('.message-label').backgroundColor,
        labelLeft: box('.message-label').left,
        pathLeft: path.getBoundingClientRect().left,
        doneBackground: style('.done-button').backgroundColor,
        doneRight: box('.done-button').right,
        readerRight: box('.reader').right,
        addOpacity: style('.add-sender').opacity,
        addVisibility: style('.add-sender').visibility,
        bodySize: parseFloat(style('.message-body').fontSize),
        bodyLine: parseFloat(style('.message-body').lineHeight),
        bodyMax: parseFloat(style('.message-body').maxWidth),
        bodyWidth: box('.message-body').width,
      };
    });
    assert.match(reading.titleFont, /^"?IBM Plex Sans"?/, `${at}: the message title is IBM Plex Sans`);
    // Compatto: the subject is 20px at every width (spec 2026-11-08 section 6).
    assert.equal(reading.titleSize, '20px', `${at}: the title size`);
    assert.match(reading.label, /priority/, `${at}: the sample message is a priority one`);
    assert.equal(reading.labelBackground, await color('--priority-bg'), `${at}: the priority tag is amber`);
    assert.ok(reading.labelLeft < reading.pathLeft, `${at}: the tag comes before the path (${reading.labelLeft} < ${reading.pathLeft})`);
    // Done is an icon button now: no fill at rest, no outline (the one blue thing stays Reply).
    assert.equal(reading.doneBackground, 'rgba(0, 0, 0, 0)', `${at}: Done has no fill`);
    assert.ok(reading.doneRight <= reading.readerRight, `${at}: Done stays inside the reading pane (${reading.doneRight} <= ${reading.readerRight})`);
    assert.equal(reading.addOpacity, '1');
    assert.equal(reading.addVisibility, 'visible', `${at}: "Add to contacts" is there without hovering`);
    assert.equal(reading.bodySize, 14, `${at}: the text is 14px`);
    assert.ok(Math.abs(reading.bodyLine / reading.bodySize - 1.6) < 0.01, `${at}: the line height is 1.6`);
    assert.equal(reading.bodyMax, 760, `${at}: the text is at most 760px wide`);
    assert.ok(reading.bodyWidth <= 760 + 1, `${at}: and is laid out that way`);

    // The summary sits close to the title and reads at 14px.
    await page.locator('.ai-summary > button').click();
    await expect(page.locator('.ai-summary p')).toBeVisible();
    assert.equal(await page.locator('.ai-summary p').evaluate((el) => getComputedStyle(el).fontSize), '14px', `${at}: the summary is 14px`);
    console.log(`PASS Reading pane at ${at}`);
    await page.close();
  }

  // Dark: the ground is dark and shared by the sidebar and the top bar.
  const dark = await browser.newPage({ locale: 'en-US', viewport: { width: 1440, height: 900 } });
  dark.on('pageerror', error => errors.push(error.message));
  await dark.addInitScript(() => localStorage.setItem('qamille:appearance', 'dark'));
  await dark.goto(url);
  await expect(dark.locator('.mail-row').first()).toBeVisible();
  const tokens = await dark.evaluate(() => {
    const token = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return { ground: token('--color-paper'), sidebar: token('--color-sidebar-paper'), surface: token('--color-raised'), wash: token('--priority-wash'), paint: getComputedStyle(document.querySelector('.app-shell')).backgroundColor };
  });
  assert.deepEqual(tokens, { ground: '#12161e', sidebar: '#12161e', surface: '#1a202c', wash: '#221d18', paint: 'rgb(18, 22, 30)' });
  console.log('PASS Dark ground, sheet and priority wash');
  await dark.close();

  assert.deepEqual(errors, []);
} finally {
  await browser?.close();
  server.kill();
}
