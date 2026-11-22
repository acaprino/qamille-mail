// The «Compatto» look's tokens, as the browser resolves them (tests/support/theme-tokens.mjs): the tokens only Qamille has
// stay what the approved table says, and the ones Sigrid's engine now computes are the values listed here, the
// before/after of the move to Sigrid (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md, «Che cosa cambierà a vista»).
// A token that changes is a decision: it goes in CHANGED with the value it had. Pure file check.
//   node tests/tokens-compatto.mjs
import { readThemes } from './support/theme-tokens.mjs';

const { light, dark } = readThemes();

/** Light tokens that did not move: Qamille's own, and the ones the engine keeps exactly. [token, value]. */
const UNCHANGED = [
  ['--text', '#111827'],
  ['--border', '#e3e6ea'],
  ['--border-soft', '#eef0f3'],
  ['--surface', '#ffffff'],
  ['--ground', '#f6f7f9'],
  ['--accent', '#4f46e5'],
  ['--accent-text', '#4338ca'],
  ['--accent-hover', '#4338ca'],
  ['--on-accent', '#ffffff'],
  ['--hover', '#f4f5f8'],
  ['--thread-open', '#f7f8fe'],
  ['--sidebar-hover', '#22262e'],
  ['--text-lg', '15px'],
  ['--text-xl', '18px'],
  ['--text-2xl', '20px'],
  ['--text-3xl', '26px'],
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
  ['--text-secondary', '#4b5563', '#3a4254'],
  ['--body-text', '#1f2937', '#111827'],
  ['--muted', '#4b5563', '#5a6376'],
  ['--nav-text', '#3d4656', '#3a4254'],
  ['--border-strong', '#dde1e7', '#87898d'],
  ['--selection', '#ededfc', '#dbe1ff'],
  ['--sidebar', '#16181d', '#15171b'],
  ['--sidebar-active', '#2a2e38', '#282b31'],
  ['--sidebar-text', '#d3d7de', '#d1d4da'],
  ['--sidebar-muted', '#a3aab7', '#9ea2a9'],
  ['--accent-on-sidebar', '#a7a3f2', '#adb8ff'],
  ['--ring', '#4f46e5', '#4338ca'],
  ['--danger', '#b03f49', '#ad2a1e'],
  ['--success', '#3f7560', '#1c6b3f'],
  ['--highlight', '#f1d56d65', '#ffe38a'],
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

// The type scale below the titles is Sigrid's (12, 13 and 14 px): Qamille's own steps for them are gone, and --text-md
// follows --text-sm.
for (const name of ['--text-2xs', '--text-xs', '--text-sm']) {
  if (light.has(name)) problems.push(`${name} is still written by Qamille: the scale is Sigrid's`);
}
if (light.get('--text-md') !== 'var(--text-sm)') problems.push(`--text-md: expected var(--text-sm), found ${light.get('--text-md')}`);

// Every token that moved to Sigrid is written in both schemes.
const hex = /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/;
for (const name of ['--surface', '--ground', '--text', '--muted', '--accent', '--accent-text', '--on-accent', '--border', '--sidebar', '--selection', '--danger', '--success']) {
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
