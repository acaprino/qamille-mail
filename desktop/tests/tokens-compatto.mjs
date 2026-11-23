// The «Compatto» look's tokens, as the browser resolves them (tests/support/theme-tokens.mjs): the tokens only Qamille has
// stay what the approved table says, and the ones Sigrid's engine now computes are the values listed here, the
// before/after of the move to Sigrid (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md, «Che cosa cambierà a vista»).
// A token that changes is a decision: it goes in CHANGED with the value it had. Pure file check.
//   node tests/tokens-compatto.mjs
import { readThemes } from './support/theme-tokens.mjs';

const { light, dark } = readThemes();

/** Light tokens that did not move: Qamille's own, and the ones the engine keeps exactly. [token, value]. */
const UNCHANGED = [
  ['--color-ink', '#111827'],
  ['--color-line', '#e3e6ea'],
  ['--color-paper-2', '#eef0f3'],
  ['--color-raised', '#ffffff'],
  ['--color-paper', '#f6f7f9'],
  ['--color-primary', '#4f46e5'],
  ['--color-accent', '#4338ca'],
  ['--color-accent', '#4338ca'],
  ['--color-primary-foreground', '#ffffff'],
  ['--hover', '#f4f5f8'],
  ['--thread-open', '#f7f8fe'],
  ['--sidebar-hover', '#22262e'],
  ['--radius-sm', '4px'],
  ['--radius-md', '6px'],
  ['--radius-lg', '8px'],
  ['--radius-xl', '12px'],
  ['--radius-full', '999px'],
  ['--radius-sheet', '14px'],
  ['--priority-mark', '#c46a0a'],
  ['--scrim', '#0f183055'],
  ['--shadow-sm', 'none'],
];

/** Light tokens the engine now computes differently: [token, what it was, what it is]. */
const CHANGED = [
  ['--color-ink-soft', '#4b5563', '#3a4254'],
  ['--color-ink', '#1f2937', '#111827'],
  ['--color-ink-mute', '#4b5563', '#5a6376'],
  ['--color-ink-soft', '#3d4656', '#3a4254'],
  ['--color-input', '#dde1e7', '#87898d'],
  ['--color-selected', '#ededfc', '#dbe1ff'],
  ['--color-sidebar-paper', '#16181d', '#15171b'],
  ['--color-sidebar-selected', '#2a2e38', '#282b31'],
  ['--color-sidebar-ink', '#d3d7de', '#d1d4da'],
  ['--color-sidebar-mute', '#a3aab7', '#9ea2a9'],
  ['--color-sidebar-accent', '#a7a3f2', '#adb8ff'],
  ['--color-ring', '#4f46e5', '#4338ca'],
  ['--color-destructive', '#b03f49', '#ad2a1e'],
  ['--color-success', '#3f7560', '#1c6b3f'],
  ['--color-highlight', '#f1d56d65', '#ffe38a'],
];

const problems = [];
for (const [name, expected] of UNCHANGED) {
  const actual = light.get(name);
  if (actual !== expected) problems.push(`${name}: expected ${expected}, found ${actual ?? 'nothing'}`);
}
for (const [name, was, now] of CHANGED) {
  const actual = light.get(name);
  if (actual !== now) problems.push(`${name}: expected ${now} (was ${was}), found ${actual ?? 'nothing'}`);
}

// The whole type scale is Sigrid's: 12, 13, 14 and 16 px, and the titles at 18, 20 and 24 px (`lg`, `xl`, `2xl`, since Sigrid
// 3.1). Qamille writes none of the steps, and no longer has its own `md` or `3xl`. Its titles of 15, 18, 20 and 26 px
// moved to base, lg, xl and 2xl (16, 18, 20 and 24 px).
for (const name of ['--text-2xs', '--text-xs', '--text-sm', '--text-base', '--text-md', '--text-lg', '--text-xl', '--text-2xl', '--text-3xl']) {
  if (light.has(name)) problems.push(`${name} is still written by Qamille: the scale is Sigrid's`);
}

// Every token that moved to Sigrid is written in both schemes.
const hex = /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/;
for (const name of ['--color-raised', '--color-paper', '--color-ink', '--color-ink-mute', '--color-primary', '--color-accent', '--color-primary-foreground', '--color-line', '--color-sidebar-paper', '--color-selected', '--color-destructive', '--color-success']) {
  for (const [scheme, tokens] of [['light', light], ['dark', dark]]) {
    if (!hex.test(tokens.get(name) ?? '')) problems.push(`${scheme} ${name}: expected a #rrggbb colour, found ${tokens.get(name) ?? 'nothing'}`);
  }
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} tokens differ from the Compatto look`);
  process.exit(1);
}
console.log(`PASS ${UNCHANGED.length + CHANGED.length + 4 + 24} tokens match the Compatto look (${CHANGED.length} moved to Sigrid's values)`);
