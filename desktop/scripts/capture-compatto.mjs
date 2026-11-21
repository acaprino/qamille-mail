// Screenshots of the views of the Compatto look, for the user to look at (spec 2026-11-08 ark-compatto,
// §10.5; plan Tasks 12 and 20). Sample data only, in a Vite server of its own on a free port (or the one
// named with --url). Output goes to checks/compatto/, which git ignores.
//
//   node scripts/capture-compatto.mjs                 every view, both themes, 1440x900 and 1000x700
//   node scripts/capture-compatto.mjs --only mail     one view (or several: --only mail,reader)
//   node scripts/capture-compatto.mjs --out <dir>     write there instead of checks/compatto/
//   node scripts/capture-compatto.mjs --scales        also the mail view and a dialog at Windows scales
//                                                      100, 125 and 150% (deviceScaleFactor 1, 1.25, 1.5)
//   node scripts/capture-compatto.mjs --locale it-IT  the language of the app (default en-US)
//   node scripts/capture-compatto.mjs --url <base>    use a running preview instead of starting Vite
//
// Files: <view>-<theme>-<width>x<height>.png, and with --scales scale-<theme>-<factor>-<view>.png.
// A view the preview cannot show yet (a screen not restyled, a control not built) is skipped with a
// note and the others are still taken.
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import { withConversation } from '../tests/support/conversation-fixture.mjs';

const desktop = path.resolve(import.meta.dirname, '..');
const repo = path.resolve(desktop, '..');

// ---------- Options ----------
const args = process.argv.slice(2);
const option = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
};
const known = new Set(['--out', '--only', '--scales', '--locale', '--url']);
for (const arg of args.filter((a) => a.startsWith('--')))
  if (!known.has(arg)) {
    console.error(`Unknown option ${arg}. Options: ${[...known].join(' ')}`);
    process.exit(2);
  }
const outDir = path.resolve(option('--out') ?? path.join(repo, 'checks', 'compatto'));
const onlyViews = option('--only')?.split(',').map((v) => v.trim()).filter(Boolean);
const withScales = args.includes('--scales');
const locale = option('--locale') ?? 'en-US';
const baseOption = option('--url');

const THEMES = ['light', 'dark'];
const SIZES = [
  { width: 1440, height: 900 },
  { width: 1000, height: 700 },
];
const SCALES = [1, 1.25, 1.5];
const SCALE_VIEWS = ['mail', 'modal'];

// ---------- Views ----------
// A view is reached from a fresh load of the app. `skip(reason)` ends it without a screenshot.
class Skip extends Error {}
const skip = (reason) => {
  throw new Skip(reason);
};

const either = (it, en) => new RegExp(`^(${it}|${en})$`, 'i');
const sidebarGo = async (page, label) => {
  const button = page.locator('aside.sidebar button').filter({ hasText: label }).first();
  if (!(await button.count())) skip(`no sidebar entry ${label}`);
  await button.click();
};
const visible = async (locator, ms = 4000) => {
  try {
    await locator.first().waitFor({ state: 'visible', timeout: ms });
    return true;
  } catch {
    return false;
  }
};

const VIEWS = {
  // The list with a conversation open and a message of it in the reader.
  mail: {
    conversation: true,
    async go(page) {
      const row = page.locator('.mail-row', { hasText: 'Launch plan' }).first();
      if (!(await visible(row))) skip('the conversation fixture is not in the list');
      const toggle = row.locator('.thread-toggle');
      if (await toggle.count()) await toggle.click();
      await row.locator('button.mail-open').click();
      await page.locator('.message-title').waitFor();
    },
  },
  // A single message with an attachment, as the reader shows it.
  reader: {
    async go(page) {
      const withFile = page.locator('.mail-row', { hasText: 'Q4' }).first();
      const row = (await withFile.count()) ? withFile : page.locator('.mail-row').first();
      await row.locator('button.mail-open').click();
      await page.locator('.message-title').waitFor();
    },
  },
  composer: {
    async go(page) {
      await page.locator('button.compose-main').click();
      await page.locator('.composer-panel').waitFor();
    },
  },
  // Settings with the list of a Select open.
  settings: {
    async go(page) {
      await page.locator('button.profile').click();
      const content = page.locator('.settings-content');
      await content.waitFor();
      const trigger = content.locator('[data-scope="select"][data-part="trigger"]').first();
      if (!(await trigger.count())) {
        console.log('  note: Settings has no Select yet, captured without one');
        return;
      }
      await trigger.click();
      if (!(await visible(page.locator('[data-scope="select"][data-part="content"]')))) skip('the Select list did not open');
    },
  },
  // A centred dialog: the new-contact form.
  modal: {
    async go(page) {
      await sidebarGo(page, either('Contacts', 'Rubrica'));
      await page.locator('.contacts-page').waitFor();
      const create = page.locator('.contacts-page button').filter({ hasText: either('New contact', 'Nuovo contatto') }).first();
      if (!(await create.count())) skip('no "New contact" button');
      await create.click();
      const dialog = page.locator('[data-scope="dialog"][data-variant="modal"][data-part="content"], dialog[open]');
      if (!(await visible(dialog))) skip('the dialog did not open');
    },
  },
  // The menu behind "More actions" in the reader.
  menu: {
    async go(page) {
      await page.locator('.mail-row').first().locator('button.mail-open').click();
      await page.locator('.message-title').waitFor();
      const more = page.getByRole('button', { name: either('More message actions', 'Altre azioni sul messaggio') });
      if (!(await more.count())) skip('no "More message actions" button');
      await more.first().click();
      if (!(await visible(page.locator('[data-scope="menu"][data-part="content"]')))) skip('the menu did not open');
    },
  },
  // The hint of an icon button: hover the buttons of the reader until one shows.
  tooltip: {
    async go(page) {
      await page.locator('.mail-row').first().locator('button.mail-open').click();
      await page.locator('.message-title').waitFor();
      const tip = page.locator('[data-scope="tooltip"][data-part="content"]');
      const buttons = page.locator('section.reader button.icon-button');
      const count = await buttons.count();
      for (let i = 0; i < count; i++) {
        const button = buttons.nth(i);
        if (!(await button.isVisible()) || (await button.isDisabled())) continue;
        await button.hover();
        if (await visible(tip, 900)) return;
      }
      skip('no icon button shows a hint yet');
    },
  },
  calendar: {
    async go(page) {
      await sidebarGo(page, either('Calendar', 'Calendario'));
      await page.locator('.calendar-page').waitFor();
    },
  },
  meetings: {
    async go(page) {
      await sidebarGo(page, either('Meetings', 'Meeting'));
      await page.locator('section.meetings-page').waitFor();
    },
  },
  contacts: {
    async go(page) {
      await sidebarGo(page, either('Contacts', 'Rubrica'));
      await page.locator('section.contacts-page').waitFor();
    },
  },
};

const unknownViews = (onlyViews ?? []).filter((v) => !(v in VIEWS));
if (unknownViews.length) {
  console.error(`Unknown view: ${unknownViews.join(', ')}. Views: ${Object.keys(VIEWS).join(', ')}`);
  process.exit(2);
}
const viewNames = onlyViews ?? Object.keys(VIEWS);

// ---------- Server ----------
const freePort = () =>
  new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.once('error', reject);
    probe.listen(0, '127.0.0.1', () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });

let server;
let base = baseOption;
if (!base) {
  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
    cwd: desktop,
    windowsHide: true,
    stdio: 'pipe',
  });
  let output = '';
  server.stdout.on('data', (data) => (output += data));
  server.stderr.on('data', (data) => (output += data));
  for (let attempt = 0; ; attempt++) {
    if (server.exitCode !== null) throw new Error(`Vite stopped: ${output}`);
    try {
      if ((await fetch(base)).ok) break;
    } catch {
      // Still starting.
    }
    if (attempt === 150) {
      server.kill();
      throw new Error(`Vite did not start: ${output}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

// ---------- Capture ----------
const sample = JSON.parse(fs.readFileSync(path.join(desktop, 'src/lib/sample.json'), 'utf8'));
const withThread = withConversation(sample);
fs.mkdirSync(outDir, { recursive: true });

const saved = [];
const skipped = [];
const failed = [];
let browser;

// One fresh page per shot: the app starts from its sample data every time.
async function shoot(view, { theme, size, scale = 1 }, file) {
  const context = await browser.newContext({
    locale,
    viewport: size,
    deviceScaleFactor: scale,
    colorScheme: theme,
    reducedMotion: 'reduce',
  });
  try {
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
    // Swapped in place of the sample, as the tests do: it adds one conversation to the list.
    await page.route(/\/src\/lib\/sample\.json/, (route) =>
      route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(VIEWS[view].conversation ? withThread : sample)};` }),
    );
    await page.goto(base);
    await page.locator('.mail-row').first().waitFor({ timeout: 60000 });
    if (theme === 'dark') await page.evaluate(() => (document.documentElement.dataset.theme = 'dark'));
    await VIEWS[view].go(page);
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(350);
    await page.screenshot({ path: file });
    if (errors.length) console.log(`  note: ${errors.length} page error(s), the first: ${errors[0]}`);
    return true;
  } finally {
    await context.close();
  }
}

async function take(view, shot, file) {
  const name = path.relative(outDir, file);
  try {
    if (await shoot(view, shot, file)) {
      saved.push(name);
      console.log(`saved   ${name}`);
    }
  } catch (error) {
    if (error instanceof Skip) {
      skipped.push(`${view}: ${error.message}`);
      console.log(`skipped ${view} (${shot.theme}${shot.size ? ` ${shot.size.width}x${shot.size.height}` : ''}): ${error.message}`);
    } else {
      failed.push(`${view}: ${String(error.message).split('\n')[0]}`);
      console.log(`FAILED  ${view} (${shot.theme}): ${String(error.message).split('\n')[0]}`);
    }
  }
}

try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  for (const view of viewNames) {
    for (const theme of THEMES) {
      for (const size of SIZES) {
        await take(view, { theme, size }, path.join(outDir, `${view}-${theme}-${size.width}x${size.height}.png`));
      }
    }
  }
  if (withScales) {
    const size = SIZES[0];
    for (const view of SCALE_VIEWS.filter((v) => viewNames.includes(v))) {
      for (const theme of THEMES) {
        for (const scale of SCALES) {
          await take(view, { theme, size, scale }, path.join(outDir, `scale-${theme}-${scale}-${view}.png`));
        }
      }
    }
  }
} finally {
  await browser?.close();
  server?.kill();
}

console.log(`\n${saved.length} saved, ${skipped.length} skipped, ${failed.length} failed -> ${outDir}`);
for (const line of skipped) console.log(`  skipped ${line}`);
for (const line of failed) console.log(`  failed  ${line}`);
process.exit(failed.length ? 1 : 0);
