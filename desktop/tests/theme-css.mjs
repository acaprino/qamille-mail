// «Compatto» as Sigrid's parameters (src/theme/compatto.ts) and the stylesheet generated from them
// (src/theme/compatto.css, scripts/theme-css.mjs): the engine keeps Compatto's page, sheet, ink, lines and indigo exactly
// and corrects nothing, the density is Qamille's, and the committed file is what the script writes now.
//   node --test tests/theme-css.mjs        (needs `npm run sigrid:sync`)
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { OUTPUT, RETIRED_NAMES, compattoCss, loadParameters, loadSigrid } from '../scripts/theme-css.mjs';

const sigrid = await loadSigrid();
const parameters = await loadParameters();
const { COMPATTO, COMPATTO_TONE, QAMILLE_DENSITY } = parameters;

// What Compatto is today (theme/qamille-tokens.css), by Sigrid's token names.
const KEPT = {
  light: { paper: '#f6f7f9', raised: '#ffffff', ink: '#111827', line: '#e3e6ea', accent: '#4338ca', primary: '#4f46e5' },
  dark: { paper: '#12161e', raised: '#1a202c', ink: '#e8ecf3', line: '#2a3240', accent: '#aeb0fa', primary: '#9a9cf8' },
};

for (const scheme of ['light', 'dark']) {
  test(`the engine keeps Compatto's ${scheme} page, sheet, ink, lines and indigo, and corrects nothing`, () => {
    const tokens = sigrid.themeTokens(COMPATTO, scheme, null, COMPATTO_TONE);
    assert.deepEqual(tokens.corrected, []);
    for (const [name, value] of Object.entries(KEPT[scheme])) assert.equal(tokens.colors[name], value, `${scheme} ${name}`);
  });
}

test('without the tone «as designed» the indigo would move: the tone is part of the parameters', () => {
  const vivid = sigrid.themeTokens(COMPATTO, 'light', null, null);
  assert.notEqual(vivid.colors.primary, KEPT.light.primary);
  assert.deepEqual(COMPATTO_TONE, { intensity: 'theme', light: 'theme' });
});

test('the density is Sigrid’s compact with every control 32 px high', () => {
  const density = sigrid.densityWith('compact', QAMILLE_DENSITY);
  assert.deepEqual(density.control, { md: 32, sm: 32, xs: 32 });
  assert.equal(density.row.height, 32);
  assert.equal(density.id, 'compatto');
});

test('the sidebar is the engine’s dark column, and the light lane of the surfaces is light', () => {
  assert.equal(COMPATTO.sidebar, 'scura');
  const lane = COMPATTO.surfaces.light.sidebar;
  assert.ok(sigrid.luminance(lane) > 0.5, `${lane} is not a light lane: it would push ink and red to black and white`);
});

test('compatto.css is what scripts/theme-css.mjs writes now', () => {
  const committed = readFileSync(OUTPUT, 'utf8');
  assert.equal(committed, compattoCss(sigrid, parameters), 'compatto.css is out of date: run `npm run theme:css` and read the diff');
});

test('compatto.css writes the colours for both schemes, the density and COMPATTO’s radii', () => {
  const css = readFileSync(OUTPUT, 'utf8');
  assert.match(css, /:root:not\(\[data-scheme\]\),\n:root\[data-scheme='light'\] \{/);
  assert.match(css, /:root\[data-scheme='dark'\] \{/);
  assert.match(css, /--density-control-md: 32px;/);
  // The corners are a parameter of the theme (`radii` of COMPATTO, Sigrid 3.2.1): 4, 6, 8 and 12 px in both schemes.
  for (const [name, px] of [['sm', 4], ['md', 6], ['lg', 8], ['xl', 12]]) {
    assert.equal(css.split(`  --radius-${name}: ${px}px;`).length - 1, 2, `--radius-${name} is ${px}px in both blocks`);
  }
  assert.doesNotMatch(css, /\bcursor:/);
});

test('none of the token names Qamille used before Sigrid is written any more', () => {
  const css = readFileSync(OUTPUT, 'utf8');
  const written = new Set([...css.matchAll(/^ {2}(--[\w-]+):/gm)].map((match) => match[1]));
  for (const name of RETIRED_NAMES) assert.ok(!written.has(name), `${name} is written by compatto.css: CSS reads Sigrid's names`);
  // And the ones it reads are there: the paper, the ink and the accent of both schemes.
  for (const name of ['--color-paper', '--color-raised', '--color-ink', '--color-line', '--color-primary', '--color-destructive']) {
    assert.ok(written.has(name), `${name} is not written`);
  }
});

test('the two windows carry the theme and density ids and the shape attributes the engine states', () => {
  const tokens = sigrid.themeTokens(COMPATTO, 'light', null, COMPATTO_TONE);
  const expected = { 'data-theme': COMPATTO.id, 'data-density': sigrid.densityWith('compact', QAMILLE_DENSITY).id, ...tokens.attributes };
  for (const file of ['../index.html', '../reminder.html']) {
    const html = readFileSync(new URL(file, import.meta.url), 'utf8');
    const root = html.match(/<html[^>]*>/)?.[0] ?? '';
    for (const [name, value] of Object.entries(expected)) assert.ok(root.includes(` ${name}="${value}"`), `${file}: <html> lacks ${name}="${value}"`);
    assert.ok(!root.includes('data-scheme'), `${file}: data-scheme is written by theme-boot.js, not in the file`);
  }
});
