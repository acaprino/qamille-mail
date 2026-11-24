// «Compatto» as Sigrid's parameters (src/theme/compatto.ts) and the stylesheet generated from them
// (src/theme/compatto.css, scripts/theme-css.mjs): Qamille hands Sigrid one colour, the accent, and the engine gives the
// grays, the sidebar, the surfaces and the corners; the density is Qamille's; and the committed file is what the script
// writes now.
//   node --test tests/theme-css.mjs        (needs `npm run sigrid:sync`)
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { OUTPUT, RETIRED_NAMES, compattoCss, loadParameters, loadSigrid } from '../scripts/theme-css.mjs';

const sigrid = await loadSigrid();
const parameters = await loadParameters();
const { COMPATTO_PICK, COMPATTO_TONE, QAMILLE_DENSITY } = parameters;
const COMPATTO = sigrid.colorTheme(COMPATTO_PICK);

test('Qamille picks only the accent: the grays, the sidebar and the surfaces are Sigrid’s', () => {
  assert.deepEqual(Object.keys(COMPATTO_PICK).sort(), ['color', 'id', 'label']);
  assert.equal(COMPATTO_PICK.color, '#4f46e5');
});

for (const scheme of ['light', 'dark']) {
  test(`in ${scheme} the engine keeps the text where it was designed and the sidebar a step below the page`, () => {
    const tokens = sigrid.themeTokens(COMPATTO, scheme, null, COMPATTO_TONE);
    for (const role of ['ink', 'inkSoft', 'inkMute']) assert.ok(!tokens.corrected.includes(role), `${scheme}: the engine moved ${role}`);
    assert.ok(sigrid.luminance(tokens.colors['sidebar-paper']) < sigrid.luminance(tokens.colors.paper), `${scheme}: the sidebar is not below the page`);
  });
}

test('in light the engine keeps the accent as picked, and corrects nothing', () => {
  const tokens = sigrid.themeTokens(COMPATTO, 'light', null, COMPATTO_TONE);
  assert.deepEqual(tokens.corrected, []);
  assert.equal(tokens.colors.primary, COMPATTO_PICK.color);
});

test('without the tone «as designed» the indigo would move: the tone is part of the parameters', () => {
  const vivid = sigrid.themeTokens(COMPATTO, 'light', null, null);
  assert.notEqual(vivid.colors.primary, COMPATTO_PICK.color);
  assert.deepEqual(COMPATTO_TONE, { intensity: 'theme', light: 'theme' });
});

test('the density is Sigrid’s compact as it is: 32, 28 and 24 px controls, under the app’s id', () => {
  const density = sigrid.densityWith('compact', QAMILLE_DENSITY);
  assert.deepEqual(density.control, sigrid.densityWith('compact', {}).control);
  assert.deepEqual(density.control, { md: 32, sm: 28, xs: 24 });
  assert.equal(density.id, 'compatto');
});

test('compatto.css is what scripts/theme-css.mjs writes now', () => {
  const committed = readFileSync(OUTPUT, 'utf8');
  assert.equal(committed, compattoCss(sigrid, parameters), 'compatto.css is out of date: run `npm run theme:css` and read the diff');
});

test('compatto.css writes the colours for both schemes, the density and the corners, the same in both', () => {
  const css = readFileSync(OUTPUT, 'utf8');
  assert.match(css, /:root:not\(\[data-scheme\]\),\n:root\[data-scheme='light'\] \{/);
  assert.match(css, /:root\[data-scheme='dark'\] \{/);
  assert.match(css, /--density-control-md: 32px;/);
  // The corners are Sigrid's (the theme's shape): written once in each block, with one value.
  for (const name of ['sm', 'md', 'lg', 'xl']) {
    const values = [...css.matchAll(new RegExp(`^ {2}--radius-${name}: ([^;]+);`, 'gm'))].map((match) => match[1]);
    assert.equal(values.length, 2, `--radius-${name} is written in both blocks`);
    assert.equal(values[0], values[1], `--radius-${name} is the same in light and in dark`);
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
