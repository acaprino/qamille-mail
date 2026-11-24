// A visual fingerprint of the app: it walks the preview through its screens and overlays and, for
// each, saves a screenshot and the computed style and box of every element. Two runs compared with
// visual-compare.mjs prove that a move of code changed nothing on screen (ADR-0011, 0 pixels).
//
//   node tests/visual-snapshot.mjs <out-dir>            (synthetic data; starts its own Vite)
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { IDS, SUBJECT, withConversation } from './support/conversation-fixture.mjs';

const out = path.resolve(process.argv[2] ?? 'visual-snapshot');
fs.mkdirSync(out, { recursive: true });
const port = 28176;
const url = `http://127.0.0.1:${port}`;
const root = path.resolve(import.meta.dirname, '..');
const sample = JSON.parse(fs.readFileSync(path.join(root, 'src/lib/sample.json'), 'utf8'));
const fixture = withConversation(sample);

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root, windowsHide: true, stdio: 'pipe',
});
let output = '';
server.stdout.on('data', (d) => (output += d));
server.stderr.on('data', (d) => (output += d));

// Every element under the body: where it is, what it is and how it is styled. Svelte's old scoping
// classes are left out: they change when a rule moves, and say nothing about the screen.
const dump = () => {
  const rows = [];
  const walk = (el, at) => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style') return;
    const box = el.getBoundingClientRect();
    const css = getComputedStyle(el);
    const style = {};
    for (let i = 0; i < css.length; i++) style[css[i]] = css.getPropertyValue(css[i]);
    rows.push({
      at,
      tag,
      cls: [...el.classList].filter((c) => !c.startsWith('svelte-')).sort().join(' '),
      box: [box.x, box.y, box.width, box.height].map((v) => Math.round(v * 100) / 100),
      text: [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent.trim()).filter(Boolean).join('|').slice(0, 80),
      attrs: [...el.attributes].filter((a) => a.name !== 'class' && a.name !== 'style').map((a) => `${a.name}=${a.value}`).sort().join(' ').slice(0, 200),
      style,
    });
    if (tag === 'svg') return;
    [...el.children].forEach((child, i) => walk(child, `${at}>${i}`));
  };
  walk(document.body, 'body');
  return rows;
};

let browser;
const reached = [];
const missed = [];
async function fresh({ width = 1440, height = 900, dark = false, threads = false } = {}) {
  const context = await browser.newContext({ locale: 'en-US', timezoneId: 'Europe/Rome', viewport: { width, height }, reducedMotion: 'reduce' });
  const page = await context.newPage();
  await page.clock.setFixedTime(new Date('2026-11-07T10:00:00+01:00'));
  await page.addInitScript(([d, t]) => {
    if (d) localStorage.setItem('qamille:appearance', 'dark');
    if (t) localStorage.setItem('qamille:mail-display', 'threads');
  }, [dark, threads]);
  await page.route(/\/src\/lib\/sample\.json/, (route) =>
    route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }),
  );
  await page.goto(url);
  await page.locator('.mail-row').first().waitFor();
  await page.locator('.message-title').waitFor();
  return page;
}
async function shot(page, name, keepPointer = false) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(250);
  if (!keepPointer) await page.mouse.move(0, 0);
  await page.waitForTimeout(100);
  // Painting settles a little after layout: wait for two equal screenshots in a row.
  let last = null;
  for (let i = 0; i < 8; i++) {
    // A native <progress> without a value animates by itself; `style` applies to the picture only.
    const image = await page.screenshot({ animations: 'disabled', caret: 'hide', style: 'progress:not([value]){visibility:hidden}' });
    if (last && image.equals(last)) break;
    last = image;
    await page.waitForTimeout(250);
  }
  fs.writeFileSync(path.join(out, `${name}.png`), last);
  const rows = await page.evaluate(dump);
  fs.writeFileSync(path.join(out, `${name}.json.gz`), zlib.gzipSync(JSON.stringify(rows)));
  reached.push(`${name} (${rows.length})`);
}
const nav = (page, name) => page.locator('[data-slot="sidebar"] button', { hasText: name }).first().click();
const settings = async (page, section) => {
  await page.getByRole('list', { name: /^(Workspace|Spazio di lavoro)$/ }).getByRole('button', { name: /^(Settings|Impostazioni)/ }).click();
  await page.locator('.settings-nav').getByRole('button', { name: section, exact: true }).click();
  await page.waitForTimeout(200);
};
async function state(name, options, steps, keepPointer = false) {
  const page = await fresh(options);
  try {
    await steps(page);
    await shot(page, name, keepPointer);
  } catch (error) {
    missed.push(`${name}: ${String(error.message).split('\n')[0].slice(0, 140)}`);
  } finally {
    await page.context().close();
  }
}

try {
  for (let i = 0; i < 80; i++) {
    if (server.exitCode != null) throw new Error(output);
    try { if ((await fetch(url)).ok) break; } catch {}
    await new Promise((r) => setTimeout(r, 200));
  }
  // Software raster, one tile at a time: otherwise a rounded corner can differ by a pixel between runs.
  browser = await chromium.launch({
    channel: 'msedge',
    headless: true,
    args: ['--disable-gpu', '--disable-partial-raster', '--disable-lcd-text', '--force-color-profile=srgb'],
  });
  const narrow = { width: 1000, height: 640 };
  const dark = { dark: true };
  const openThread = async (page) => {
    await page.locator('.mail-row', { hasText: SUBJECT }).locator('.thread-toggle').click();
    await page.locator('.reply-row').first().waitFor();
  };

  // The controls under the pointer and under the keyboard, in both themes
  const controls = {
    primary: (page) => page.getByRole('button', { name: /^Reply/ }).first(),
    secondary: (page) => page.locator('.reader-forward'),
    icon: (page) => page.getByRole('button', { name: 'Archive message', exact: true }),
    text: (page) => page.locator('.add-sender'),
  };
  for (const [name, control] of Object.entries(controls)) {
    for (const theme of [{}, dark]) {
      const suffix = theme.dark ? '-dark' : '';
      await state(`hover-${name}${suffix}`, theme, async (page) => control(page).hover(), true);
      await state(`focus-${name}${suffix}`, theme, async (page) => {
        // Reached from the keyboard, so that :focus-visible applies.
        await control(page).focus();
        await page.keyboard.press('Shift+Tab');
        await page.keyboard.press('Tab');
      });
    }
  }
  // Mail
  await state('inbox', {}, async () => {});
  await state('inbox-narrow', narrow, async () => {});
  await state('inbox-dark', dark, async () => {});
  await state('groups-open', {}, async (page) => {
    for (const name of ['Notifications', 'Newsletters', 'Seen']) await page.locator('.group-heading', { hasText: name }).click();
  });
  await state('selection-bar', {}, async (page) => {
    await page.locator('.mail-row').nth(1).locator('.mail-open').hover();
    await page.locator('.mail-row').nth(1).locator('.mail-checkbox').click();
    await page.locator('.selection-toolbar').waitFor();
  });
  await state('reader-menu', {}, async (page) => {
    await page.getByRole('button', { name: 'More message actions' }).click();
    await page.locator('[data-scope="menu"][data-part="content"]').waitFor();
  });
  await state('reader-summary', {}, async (page) => {
    await page.locator('.ai-summary > button').click();
    await page.locator('.ai-summary p').waitFor();
  });
  await state('attachment-panel', {}, async (page) => {
    await page.getByRole('button', { name: /Q4-proposal.txt.*Preview/ }).click();
    await page.locator('[data-slot="dialog-content"]').waitFor();
  });
  await state('threads', { threads: true }, openThread);
  await state('threads-narrow', { ...narrow, threads: true }, openThread);
  await state('threads-dark', { ...dark, threads: true }, openThread);
  await state('sent-reply', { threads: true }, async (page) => {
    await openThread(page);
    await page.locator(`.reply-row[data-reply-id="${IDS.sent}"] .reply-open`).click();
    await page.getByText('Budget is with Maria').first().waitFor();
  });
  await state('search-filters', {}, async (page) => {
    await page.locator('.search-filter-button').first().click();
    await page.waitForTimeout(200);
  });
  await state('search-results', {}, async (page) => {
    await page.locator('.global-search input').fill('proposal');
    await page.keyboard.press('Enter');
    await page.getByRole('heading', { name: 'Search results' }).waitFor();
  });
  await state('print-preview', {}, async (page) => {
    await page.locator('.mail-row').nth(1).locator('.mail-open').hover();
    await page.locator('.mail-row').nth(1).locator('.mail-checkbox').click();
    await page.getByRole('button', { name: 'Print selected', exact: true }).click();
    await page.locator('.mail-print-preview').waitFor();
  });
  await state('composer', {}, async (page) => {
    await page.locator('button.compose-main').click();
    await page.locator('.composer-panel').waitFor();
  });
  await state('composer-reply', {}, async (page) => {
    await page.getByRole('button', { name: /^Reply/ }).first().click();
    await page.locator('.composer-panel').waitFor();
  });
  // Folders
  await state('drafts', {}, async (page) => nav(page, 'Drafts'));
  await state('sent', {}, async (page) => nav(page, 'Sent'));
  // Calendar
  await state('calendar', {}, async (page) => nav(page, 'Calendar'));
  await state('calendar-dark', dark, async (page) => nav(page, 'Calendar'));
  await state('calendar-event', {}, async (page) => {
    await nav(page, 'Calendar');
    await page.getByRole('button', { name: 'New event', exact: true }).click();
    await page.locator('.event-sheet').waitFor();
  });
  await state('calendar-narrow', narrow, async (page) => nav(page, 'Calendar'));
  // Contacts
  await state('contacts', {}, async (page) => nav(page, 'Contacts'));
  await state('contact-new', {}, async (page) => {
    await nav(page, 'Contacts');
    await page.getByRole('button', { name: 'New contact' }).click();
    await page.locator('.contact-dialog').waitFor();
  });
  await state('contact-duplicates', {}, async (page) => {
    await nav(page, 'Contacts');
    await page.getByRole('button', { name: /^Duplicates/ }).click();
    await page.getByRole('region', { name: 'Duplicates' }).waitFor();
  });
  await state('contact-merge', {}, async (page) => {
    await nav(page, 'Contacts');
    await page.getByRole('button', { name: /^Duplicates/ }).click();
    await page.getByRole('region', { name: 'Duplicates' }).getByRole('article').first().getByRole('button', { name: 'Merge…' }).click();
    await page.locator('.merge-dialog').waitFor();
  });
  await state('contact-open', {}, async (page) => {
    await nav(page, 'Contacts');
    await page.getByText('Ada Lovelace').first().click();
    await page.waitForTimeout(300);
  });
  await state('contact-delete', {}, async (page) => {
    await nav(page, 'Contacts');
    await page.getByText('Ada Lovelace').first().click();
    await page.getByRole('button', { name: /^Delete/ }).first().click();
    await page.locator('.contact-dialog').waitFor();
  });
  // Meetings and assistant
  await state('meetings', {}, async (page) => nav(page, 'Meetings'));
  await state('meetings-transcript', {}, async (page) => {
    await nav(page, 'Meetings');
    await page.locator('.meetings-page [data-row-key] .item-main', { hasText: 'Weekly sync' }).click();
    await page.getByRole('tab', { name: 'Transcript' }).click();
  });
  await state('meetings-narrow', narrow, async (page) => nav(page, 'Meetings'));
  await state('assistant', {}, async (page) => nav(page, 'Assistant'));
  // Settings: every section, then the dialogs that hang from it
  const sections = await (async () => {
    const page = await fresh();
    await page.getByRole('list', { name: /^(Workspace|Spazio di lavoro)$/ }).getByRole('button', { name: /^(Settings|Impostazioni)/ }).click();
    await page.locator('.settings-nav').waitFor();
    const names = await page.locator('.settings-nav button').allInnerTexts();
    await page.context().close();
    return names.map((n) => n.trim()).filter(Boolean);
  })();
  for (const section of sections) {
    await state(`settings-${section.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}`, {}, async (page) => settings(page, section));
  }
  await state('settings-accounts-dark', dark, async (page) => settings(page, 'Accounts'));
  await state('google-sign-in', {}, async (page) => {
    await page.evaluate(() => (window.__previewGoogleSignIn = 'hold'));
    await settings(page, 'Accounts');
    await page.getByRole('button', { name: 'Sign in with Google', exact: true }).first().click();
    await page.locator('.google-sign-in-dialog').waitFor({ timeout: 2500 });
  });
  await state('google-setup', {}, async (page) => {
    await settings(page, 'Accounts');
    await page.getByRole('button', { name: /How to set up Google sign-in/ }).first().click();
    await page.locator('.google-setup-guide').waitFor({ timeout: 2500 });
  });
  await state('unsaved', {}, async (page) => {
    await page.getByRole('list', { name: /^(Workspace|Spazio di lavoro)$/ }).getByRole('button', { name: /^(Settings|Impostazioni)/ }).click();
    await page.locator('.settings-nav').waitFor();
    const field = page.locator('.settings-layout input[type="text"], .settings-layout input:not([type])').first();
    await field.fill('Changed for the snapshot');
    await nav(page, 'Calendar');
    await page.locator('.unsaved-dialog').waitFor({ timeout: 2500 });
  });

  fs.writeFileSync(path.join(out, 'report.json'), JSON.stringify({ reached, missed }, null, 1));
  console.log(`reached ${reached.length}: ${reached.map((r) => r.split(' ')[0]).join(', ')}`);
  console.log(`missed ${missed.length}:\n${missed.join('\n')}`);
} finally {
  await browser?.close();
  server.kill();
}
