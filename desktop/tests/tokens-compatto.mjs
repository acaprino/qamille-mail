// The tokens of the app's look, as the browser resolves them (tests/support/theme-tokens.mjs): the colours, the corners and
// the elevations are Sigrid's (compatto.css, written by its engine from the accent: tests/theme-css.mjs keeps the file equal
// to the engine), so no hex of them is pinned here. What this checks is what is Qamille's own (qamille-tokens.css), that the
// type scale is Sigrid's, that the tokens CSS reads are written in both schemes, and that the names Qamille retired stay gone.
// Pure file check.
//   node tests/tokens-compatto.mjs
import { readThemes } from './support/theme-tokens.mjs';

const { light, dark } = readThemes();

/** Tokens only Qamille has, in the light block: [token, value]. */
const OWN = [
  ['--radius-full', '999px'],
  ['--radius-sheet', '14px'],
  ['--priority-mark', '#c46a0a'],
  ['--scrim', '#0f183055'],
  ['--shadow-sm', 'none'],
];

/** Tokens that were Qamille's and went to Sigrid's names or were deleted: nothing writes them any more. */
const RETIRED = ['--sidebar-hover', '--on-accent-soft', '--on-accent-border'];

const problems = [];
for (const [name, expected] of OWN) {
  const actual = light.get(name);
  if (actual !== expected) problems.push(`${name}: expected ${expected}, found ${actual ?? 'nothing'}`);
}
for (const name of RETIRED) {
  for (const [scheme, tokens] of [['light', light], ['dark', dark]]) {
    if (tokens.has(name)) problems.push(`${scheme} ${name} is written again: use Sigrid's tokens`);
  }
}

// The whole type scale is Sigrid's: 12, 13, 14 and 16 px, and the titles at 18, 20 and 24 px (`lg`, `xl`, `2xl`, since Sigrid
// 3.1). Qamille writes none of the steps, and no longer has its own `md` or `3xl`.
for (const name of ['--text-2xs', '--text-xs', '--text-sm', '--text-base', '--text-md', '--text-lg', '--text-xl', '--text-2xl', '--text-3xl']) {
  if (light.has(name)) problems.push(`${name} is still written by Qamille: the scale is Sigrid's`);
}

// Every token CSS reads from Sigrid is written in both schemes, as a colour.
const hex = /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/;
for (const name of ['--color-raised', '--color-paper', '--color-paper-2', '--color-ink', '--color-ink-mute', '--color-primary', '--color-accent', '--color-primary-foreground', '--color-line', '--color-sidebar-paper', '--color-selected', '--color-destructive', '--color-success']) {
  for (const [scheme, tokens] of [['light', light], ['dark', dark]]) {
    if (!hex.test(tokens.get(name) ?? '')) problems.push(`${scheme} ${name}: expected a #rrggbb colour, found ${tokens.get(name) ?? 'nothing'}`);
  }
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} tokens differ from the look`);
  process.exit(1);
}
console.log(`PASS ${OWN.length} own tokens, ${RETIRED.length} retired tokens gone, the scale and ${13 * 2} Sigrid colours in place`);
