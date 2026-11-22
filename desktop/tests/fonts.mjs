// The typeface of the app: IBM Plex Sans, the one Sigrid ships (weights 400, 500 and 600, latin and latin-ext, with
// Inter for the glyphs outside Latin), bundled by Vite as files of the app (CSP font-src 'self'). Static checks on the
// files and the CSS, then one browser check that the font is really loaded and is the one used to render the text.
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
const sigridFonts = path.join(desktop, 'vendor', 'sigrid', 'src', 'styles', 'fonts');
const results = [];
function step(name, work) {
  return Promise.resolve(work()).then(() => {
    results.push(name);
    console.log(`PASS ${name}`);
  });
}

await step("Sigrid's Plex files (latin and latin-ext, 400, 500, 600) are real woff2 files", () => {
  for (const weight of [400, 500, 600]) {
    for (const range of ['latin', 'latin-ext']) {
      const name = `ibm-plex-sans-${range}-${weight}.woff2`;
      const file = path.join(sigridFonts, name);
      assert.ok(fs.existsSync(file), `${name} is missing: run npm run sigrid:sync`);
      assert.equal(fs.readFileSync(file).subarray(0, 4).toString('latin1'), 'wOF2', `${name} is not a woff2 file`);
    }
  }
});

await step('the app takes its typeface from Sigrid and declares none of its own', () => {
  const tailwind = fs.readFileSync(path.join(desktop, 'src', 'ds', 'styles', 'tailwind.css'), 'utf8');
  assert.match(tailwind, /@import '[^']*vendor\/sigrid\/src\/styles\/fonts\.css';/, "tailwind.css must import Sigrid's fonts.css");
  for (const file of ['src/ds/styles/system.css', 'src/reminder/reminder.css']) {
    const css = fs.readFileSync(path.join(desktop, file), 'utf8');
    assert.ok(!/@font-face/.test(css), `${file} declares a @font-face of its own`);
  }
  assert.ok(!fs.existsSync(path.join(desktop, 'public', 'fonts')), 'public/fonts still holds a font of the app');
  const reminder = fs.readFileSync(path.join(desktop, 'src', 'reminder', 'reminder.css'), 'utf8');
  assert.match(reminder, /vendor\/sigrid\/src\/styles\/fonts\.css/, "the reminder window must import Sigrid's fonts.css");
});

await step("--font-text and --font-title follow --font-sans of Sigrid, which starts with IBM Plex Sans", () => {
  const generated = fs.readFileSync(path.join(desktop, 'src', 'theme', 'compatto.css'), 'utf8');
  for (const token of ['--font-text', '--font-title']) {
    assert.match(generated, new RegExp(`${token}:\\s*var\\(--font-sans\\);`), `${token} must be an alias of --font-sans`);
  }
  const theme = fs.readFileSync(path.join(desktop, 'vendor', 'sigrid', 'src', 'styles', 'theme.css'), 'utf8');
  assert.match(theme, /--font-sans:\s*"IBM Plex Sans"/, '--font-sans must start with IBM Plex Sans');
});

await step('index.html preloads no font: the files are hashed by the build', () => {
  const html = fs.readFileSync(path.join(desktop, 'index.html'), 'utf8');
  assert.ok(!/<link[^>]*rel="preload"[^>]*as="font"/.test(html), 'index.html still preloads a font');
});

const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  const context = await browser.newContext({
    locale: 'en-US',
    viewport: { width: 1440, height: 900 },
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const failed = [];
  page.on('requestfailed', (request) => {
    if (/\.woff2(\?|$)/.test(request.url())) failed.push(request.url());
  });
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  await page.locator('.compose-main').waitFor();

  await step('the font is loaded and the body uses it', async () => {
    const state = await page.evaluate(async () => {
      await document.fonts.ready;
      const faces = [...document.fonts]
        .filter((face) => face.family.replace(/['"]/g, '') === 'IBM Plex Sans')
        .map((face) => ({ weight: face.weight, status: face.status }));
      return {
        check: document.fonts.check('14px "IBM Plex Sans"'),
        bodyFamily: getComputedStyle(document.body).fontFamily,
        faces,
      };
    });
    assert.ok(state.check, 'document.fonts.check is false');
    assert.ok(
      state.bodyFamily.startsWith('"IBM Plex Sans"'),
      `body font-family is ${state.bodyFamily}`,
    );
    // document.fonts.check is also true when no @font-face matches, so the faces are counted too.
    assert.equal(state.faces.length, 6, 'the document should hold six IBM Plex Sans faces (latin and latin-ext, three weights)');
    assert.deepEqual(
      [...new Set(state.faces.map((face) => face.weight))].sort(),
      ['400', '500', '600'],
      `weights: ${JSON.stringify(state.faces)}`,
    );
    assert.ok(
      state.faces.some((face) => face.status === 'loaded'),
      `no IBM Plex Sans face is loaded: ${JSON.stringify(state.faces)}`,
    );
  });

  await step('the title font resolves to IBM Plex Sans too', async () => {
    const families = await page.evaluate(() => {
      const probe = document.createElement('h1');
      probe.style.fontFamily = 'var(--font-title)';
      document.body.append(probe);
      const family = getComputedStyle(probe).fontFamily;
      probe.remove();
      return family;
    });
    assert.ok(families.startsWith('"IBM Plex Sans"'), `--font-title resolves to ${families}`);
  });

  await step('the browser renders the text with the platform font IBM Plex Sans', async () => {
    // The first element that holds visible text of its own.
    const marked = await page.evaluate(() => {
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const element = node.parentElement;
        if (!node.textContent.trim() || !element || ['SCRIPT', 'STYLE'].includes(element.tagName))
          continue;
        const box = element.getBoundingClientRect();
        if (box.width < 4 || box.height < 4 || getComputedStyle(element).visibility === 'hidden')
          continue;
        element.setAttribute('data-font-probe', '');
        return true;
      }
      return false;
    });
    assert.ok(marked, 'no visible text found');
    const session = await context.newCDPSession(page);
    await session.send('DOM.enable');
    await session.send('CSS.enable');
    const { root } = await session.send('DOM.getDocument', { depth: 0 });
    const { nodeId } = await session.send('DOM.querySelector', {
      nodeId: root.nodeId,
      selector: '[data-font-probe]',
    });
    const { fonts } = await session.send('CSS.getPlatformFontsForNode', { nodeId });
    assert.ok(fonts.length > 0, 'no platform font reported');
    // isCustomFont: the glyphs come from the file served by the app, not from a copy installed on the machine.
    assert.ok(
      fonts.every((font) => /IBM Plex Sans/.test(font.familyName) && font.isCustomFont),
      `the text is rendered with ${fonts.map((font) => `${font.familyName} (custom: ${font.isCustomFont})`).join(', ')}`,
    );
  });

  assert.deepEqual(failed, [], `font requests failed: ${failed.join(', ')}`);
  assert.deepEqual(errors, [], `page errors: ${errors.join(' | ')}`);
} finally {
  await browser.close();
}
console.log(`\n${results.length} checks passed`);
