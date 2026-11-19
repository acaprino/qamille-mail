// The typeface of the app: IBM Plex Sans (variable, weight 100-700), self-hosted from public/fonts
// (CSP font-src 'self'). Static checks on the files and the CSS, then one browser check that the
// font is really loaded and is the one used to render the text.
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
const fontDir = path.join(desktop, 'public', 'fonts', 'ibm-plex-sans');
const results = [];
function step(name, work) {
  return Promise.resolve(work()).then(() => {
    results.push(name);
    console.log(`PASS ${name}`);
  });
}

await step('the two woff2 files and the licence are in public/fonts/ibm-plex-sans', () => {
  for (const name of ['IBMPlexSans-latin-wght.woff2', 'IBMPlexSans-latin-ext-wght.woff2']) {
    const file = path.join(fontDir, name);
    assert.ok(fs.existsSync(file), `${name} is missing`);
    // A real woff2 starts with the signature "wOF2".
    const head = fs.readFileSync(file).subarray(0, 4).toString('latin1');
    assert.equal(head, 'wOF2', `${name} is not a woff2 file`);
  }
  assert.ok(fs.existsSync(path.join(fontDir, 'OFL.txt')), 'OFL.txt is missing');
  assert.ok(
    fs.existsSync(path.join(fontDir, 'README.txt')),
    'README.txt (origin and version) is missing',
  );
});

await step('system.css declares IBM Plex Sans, weight 100 700, with local urls only', () => {
  const css = fs.readFileSync(path.join(desktop, 'src', 'ds', 'styles', 'system.css'), 'utf8');
  const blocks = [...css.matchAll(/@font-face\s*\{([^}]*)\}/g)].map((match) => match[1]);
  const plex = blocks.filter((block) => /font-family:\s*['"]IBM Plex Sans['"]\s*;/.test(block));
  assert.equal(
    plex.length,
    2,
    'expected two @font-face blocks for IBM Plex Sans (latin, latin-ext)',
  );
  for (const block of plex) {
    assert.match(block, /font-weight:\s*100 700\s*;/, 'the weight range must be 100 700');
    assert.match(block, /font-display:\s*swap\s*;/);
    assert.match(block, /unicode-range:/);
    const urls = [...block.matchAll(/url\(\s*['"]?([^'")]+)['"]?\s*\)/g)].map((match) => match[1]);
    assert.ok(urls.length > 0, 'the block has no url()');
    for (const url of urls) {
      assert.ok(!/^https?:/i.test(url) && !url.startsWith('//'), `${url} is not a local file`);
      assert.ok(
        url.startsWith('/fonts/ibm-plex-sans/'),
        `${url} is not under /fonts/ibm-plex-sans/`,
      );
      assert.ok(
        fs.existsSync(path.join(desktop, 'public', url)),
        `${url} does not exist in public/`,
      );
    }
  }
  // No @font-face of the app reaches a host.
  assert.ok(
    !blocks.some((block) => /url\(\s*['"]?https?:/i.test(block)),
    'a @font-face points to a host',
  );
});

await step('theme.css points --font-text and --font-title to IBM Plex Sans', () => {
  const css = fs.readFileSync(path.join(desktop, 'src', 'ds', 'tokens', 'theme.css'), 'utf8');
  for (const token of ['--font-text', '--font-title']) {
    const match = css.match(new RegExp(`${token}:\\s*([^;]+);`));
    assert.ok(match, `${token} is not declared`);
    assert.ok(
      match[1].trim().startsWith("'IBM Plex Sans'"),
      `${token} must start with 'IBM Plex Sans', got ${match[1]}`,
    );
  }
});

await step('index.html preloads the latin file only', () => {
  const html = fs.readFileSync(path.join(desktop, 'index.html'), 'utf8');
  const preloads = [...html.matchAll(/<link[^>]*rel="preload"[^>]*>/g)].map((match) => match[0]);
  const fonts = preloads.filter((tag) => /as="font"/.test(tag));
  assert.equal(fonts.length, 1, 'expected one font preload');
  assert.match(fonts[0], /href="\/fonts\/ibm-plex-sans\/IBMPlexSans-latin-wght\.woff2"/);
  assert.match(fonts[0], /type="font\/woff2"/);
  assert.match(fonts[0], /crossorigin/);
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
    if (request.url().includes('/fonts/')) failed.push(request.url());
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
        check: document.fonts.check('13.5px "IBM Plex Sans"'),
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
    assert.equal(state.faces.length, 2, 'the document should hold two IBM Plex Sans faces');
    assert.ok(
      state.faces.every((face) => face.weight === '100 700'),
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
