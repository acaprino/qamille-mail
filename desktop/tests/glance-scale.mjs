// Windows display scale other than 100% (spec 2026-11-07 §8.1, "not verified"): the restyled list and
// the conversation rows at device scale 1, 1.25, 1.5 and 2, at 1000x640 and 1440x900.
//   1. In the browser preview (Playwright `deviceScaleFactor`).
//   2. In the built executable started with `--force-device-scale-factor=1.5`, over CDP.
// Checked at each size: no sideways scroll, "Done" inside the reading pane, reply rows of one height,
// "+N" and the dates inside the list. Synthetic data only. Port 28178 for the preview.
//   node tests/glance-scale.mjs        (from desktop/)
// Phase 2 needs release/Qamille.exe (or QAMILLE_NATIVE_EXE) and no open Qamille window;
// with QAMILLE_SCALE_NATIVE=0 only the preview runs.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { IDS, SUBJECT, withConversation } from './support/conversation-fixture.mjs';
import { openAllGroups } from './support/groups.mjs';
import { launchNative } from './support/native-app.mjs';

const port = 28178;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const out = path.resolve(root, '../checks/glance-scale');
await fs.mkdir(out, { recursive: true });
const sample = JSON.parse(await fs.readFile(path.join(root, 'src/lib/sample.json'), 'utf8'));
const fixture = withConversation(sample);
for (const email of fixture.emails) email.reminder_at = null;

/** Runs in the page: every measurement the checks need, in CSS pixels. */
function geometry() {
  const box = (el) => { const r = el.getBoundingClientRect(); return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height }; };
  const list = document.querySelector('.mail-list');
  const reader = document.querySelector('.reader');
  const done = document.querySelector('.done-button');
  const replies = [...document.querySelectorAll('.reply-row')];
  const mainRow = [...document.querySelectorAll('.mail-row')].find((r) => r.querySelector('.thread-toggle'));
  const overflow = (el) => el.scrollWidth - el.clientWidth;
  return {
    dpr: window.devicePixelRatio, innerWidth: window.innerWidth, innerHeight: window.innerHeight,
    docOverflow: document.documentElement.scrollWidth - window.innerWidth,
    bodyOverflow: document.body.scrollWidth - window.innerWidth,
    listOverflow: overflow(list), readerScrollOverflow: overflow(document.querySelector('.reader-scroll') ?? reader),
    repliesOverflow: replies.length ? overflow(document.querySelector('.thread-replies')) : 0,
    list: box(list), reader: box(reader), done: done ? box(done) : null,
    replies: replies.map((row) => {
      const container = row.closest('.thread-replies').getBoundingClientRect();
      const more = row.querySelector('.to-more');
      return {
        id: Number(row.dataset.replyId), height: Math.round(row.getBoundingClientRect().height * 100) / 100,
        timeRight: row.querySelector('time').getBoundingClientRect().right,
        timeLeft: row.querySelector('time').getBoundingClientRect().left,
        moreRight: more ? more.getBoundingClientRect().right : null, moreWidth: more ? more.getBoundingClientRect().width : 0,
        foldRight: row.querySelector('.reply-fold')?.getBoundingClientRect().right ?? null,
        containerLeft: container.left, containerRight: container.right,
      };
    }),
    main: mainRow ? {
      timeRight: mainRow.querySelector('time').getBoundingClientRect().right,
      toggle: box(mainRow.querySelector('.thread-toggle')), row: box(mainRow),
    } : null,
  };
}

/** The assertions, shared by both phases. */
function check(g, at) {
  const eps = 0.75; // sub-pixel rounding at 1.25 and 1.5
  assert.ok(g.docOverflow <= 0 && g.bodyOverflow <= 0, `${at}: the page does not scroll sideways (document ${g.docOverflow}, body ${g.bodyOverflow})`);
  assert.ok(g.listOverflow <= 1, `${at}: the list does not scroll sideways (${g.listOverflow})`);
  assert.ok(g.repliesOverflow <= 1, `${at}: the reply rows do not overflow (${g.repliesOverflow})`);
  assert.ok(g.readerScrollOverflow <= 1, `${at}: the reading pane does not scroll sideways (${g.readerScrollOverflow})`);
  assert.ok(g.done, `${at}: "Done" is there`);
  assert.ok(g.done.right <= g.reader.right + eps && g.done.left >= g.reader.left - eps, `${at}: Done is inside the reading pane (${g.done.left}-${g.done.right} in ${g.reader.left}-${g.reader.right})`);
  assert.equal(g.replies.length, 4, `${at}: four replies are open`);
  assert.equal(new Set(g.replies.map((r) => r.height)).size, 1, `${at}: every reply row has the same height (${g.replies.map((r) => r.height)})`);
  for (const r of g.replies) {
    assert.ok(r.timeLeft >= r.containerLeft - eps && r.timeRight <= r.containerRight + eps, `${at}: the date of reply ${r.id} is inside the list (${r.timeLeft}-${r.timeRight} in ${r.containerLeft}-${r.containerRight})`);
    if (r.moreRight !== null) assert.ok(r.moreRight <= r.containerRight + eps && r.moreWidth > 0, `${at}: "+N" of reply ${r.id} is inside the list`);
    if (r.foldRight !== null) assert.ok(r.foldRight <= r.containerRight + eps, `${at}: the fold button of reply ${r.id} is inside the list`);
  }
  assert.ok(g.replies.find((r) => r.id === IDS.many).moreWidth > 0, `${at}: "+11" of the reply to twelve people is shown`);
  assert.ok(g.main, `${at}: the conversation row is there`);
  assert.ok(g.main.timeRight <= g.list.right + eps, `${at}: the date of the main row is inside the list`);
  assert.ok(g.main.toggle.right <= g.main.row.right + eps && g.main.toggle.left >= g.main.row.left - eps, `${at}: the arrow is inside its row`);
}

async function prepare(page) {
  const row = page.locator('.mail-row', { hasText: SUBJECT });
  const toggle = row.locator('.thread-toggle');
  await expect(toggle).toBeVisible();
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  await expect(page.locator('.reply-row')).toHaveCount(4);
  // Open a reply: the reading pane (and its "Done" button) is what some checks measure, and the app
  // does not always open a message by itself in the executable.
  await page.locator(`.reply-row[data-reply-id="${IDS.a}"] .reply-open`).click();
  await expect(page.locator('.message-title')).toBeVisible();
  await page.mouse.move(0, 0);
  await page.evaluate(() => document.fonts.ready);
}

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (data) => (output += data));
server.stderr.on('data', (data) => (output += data));
let browser;
const errors = [];
try {
  for (let i = 0; i < 60; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    if (i === 59) throw new Error(`Preview did not start: ${output}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });

  // ---- 1. the preview, at every scale and size ----
  for (const deviceScaleFactor of [1, 1.25, 1.5, 2]) {
    for (const [width, height] of [[1440, 900], [1000, 640]]) {
      const at = `preview ${width}x${height} @${deviceScaleFactor}`;
      const page = await browser.newPage({ locale: 'en-US', viewport: { width, height }, deviceScaleFactor });
      page.on('pageerror', (error) => errors.push(error.message));
      await page.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
      await page.route(/\/src\/lib\/sample\.json/, (route) =>
        route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
      );
      await page.goto(url);
      await prepare(page);
      const g = await page.evaluate(geometry);
      assert.equal(g.dpr, deviceScaleFactor, `${at}: the page really runs at that scale`);
      check(g, at);
      if (width === 1000 || deviceScaleFactor === 1.5) await page.screenshot({ path: path.join(out, `preview-${width}-${deviceScaleFactor}.png`) });
      console.log(`PASS ${at}: no sideways scroll, Done inside the pane, equal reply rows, "+N" and dates inside the list`);
      await page.close();
    }
  }

  // ---- 2. the built executable at 150% ----
  if (process.env.QAMILLE_SCALE_NATIVE === '0') {
    console.log('NOT RUN native phase (QAMILLE_SCALE_NATIVE=0)');
  } else {
    const app = await launchNative('glance-scale', { store: fixture, browserArgs: '--force-device-scale-factor=1.5' });
    try {
      const { page } = app;
      // Conversations mode through the interface, no reload; a conversation that has been read sits in the closed Seen group.
      await page.getByRole('button', { name: 'Threads', exact: true }).click();
      await openAllGroups(page);
      await prepare(page);
      const natural = await page.evaluate(geometry);
      assert.equal(natural.dpr, 1.5, `the executable really runs at 150% (devicePixelRatio ${natural.dpr})`);
      console.log(`INFO native window at 150%: ${natural.innerWidth}x${natural.innerHeight} CSS px (${Math.round(natural.innerWidth * 1.5)}x${Math.round(natural.innerHeight * 1.5)} device px)`);
      // --force-device-scale-factor scales the page but not the Tauri window: a window that is 1440x900 device
      // pixels is 960x600 CSS pixels here, under the 1000 px minimum that Windows would enforce at 150%
      // (minWidth 1000 logical = 1500 physical). Below the minimum nothing is promised, so it is only reported.
      if (natural.innerWidth >= 1000) check(natural, `native ${natural.innerWidth}x${natural.innerHeight} @1.5 (window as opened)`);
      else console.log(`INFO the window as opened is below the 1000 px minimum (an artefact of forcing the scale): not asserted; Done ends ${Math.round(natural.done.right - natural.reader.right)} px beyond the pane there`);
      await page.screenshot({ path: path.join(out, 'native-1.5-window.png') });
      const cdp = await page.context().newCDPSession(page);
      for (const [width, height] of [[1440, 900], [1000, 640]]) {
        // deviceScaleFactor 0 keeps the 1.5 the process was started with.
        await cdp.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 0, mobile: false });
        await page.waitForTimeout(300);
        const g = await page.evaluate(geometry);
        assert.equal(g.dpr, 1.5);
        assert.equal(g.innerWidth, width);
        check(g, `native ${width}x${height} @1.5`);
        await page.screenshot({ path: path.join(out, `native-${width}-1.5.png`) });
        console.log(`PASS native ${width}x${height} @1.5: no sideways scroll, Done inside the pane, equal reply rows, "+N" and dates inside the list`);
      }
      assert.deepEqual(app.pageErrors, [], 'no page error in the executable');
    } finally {
      await app.stop();
    }
  }
  assert.deepEqual(errors, [], 'no page error in the preview');
  console.log(`Screenshots: ${out}`);
} finally {
  await browser?.close();
  server.kill();
}
