// The light theme of the "Compatto" look (spec 2026-11-08 ark-compatto, section 6): every value of the
// token table, read from the :root block of src/ds/tokens/theme.css. The dark block is not looked at
// here (it follows in its own task). Pure file check.
//   node tests/tokens-compatto.mjs
import { readThemes } from './support/theme-tokens.mjs';

const { light } = readThemes();

/** [token, expected value]. */
const TABLE = [
  // Type scale
  ['--text-2xs', '11.5px'],
  ['--text-xs', '12.5px'],
  ['--text-sm', '13.5px'],
  ['--text-md', '14px'],
  ['--text-lg', '15px'],
  ['--text-xl', '18px'],
  ['--text-2xl', '20px'],
  ['--text-3xl', '26px'],
  // Text
  ['--text', '#111827'],
  ['--body-text', '#1f2937'],
  ['--text-secondary', '#4b5563'],
  ['--muted', '#4b5563'],
  // Borders
  ['--border', '#e3e6ea'],
  ['--border-soft', '#eef0f3'],
  ['--border-strong', '#dde1e7'],
  // Surfaces
  ['--surface', '#ffffff'],
  ['--ground', '#f6f7f9'],
  // Accent, selection, hover, open conversation
  ['--accent', '#4f46e5'],
  ['--selection', '#ededfc'],
  ['--hover', '#f4f5f8'],
  ['--thread-open', '#f7f8fe'],
  // Dark sidebar
  ['--sidebar', '#16181d'],
  ['--sidebar-active', '#2a2e38'],
  ['--sidebar-hover', '#22262e'],
  ['--sidebar-text', '#d3d7de'],
  ['--sidebar-text-active', '#ffffff'],
  ['--sidebar-muted', '#a3aab7'],
  ['--accent-on-sidebar', '#a7a3f2'],
  // Radii and controls
  ['--radius-sm', '4px'],
  ['--radius-md', '6px'],
  ['--control-height', '32px'],
];

// Values the table says are unchanged.
const UNCHANGED = [
  ['--radius-lg', '8px'],
  ['--radius-xl', '12px'],
  ['--radius-full', '999px'],
  ['--radius-sheet', '14px'],
  ['--danger', '#b03f49'],
  ['--success', '#3f7560'],
  ['--priority-mark', '#c46a0a'],
  ['--scrim', '#0f183055'],
  ['--shadow-sm', 'none'],
];

const problems = [];
for (const [name, expected] of [...TABLE, ...UNCHANGED]) {
  const actual = light.get(name);
  if (actual !== expected) problems.push(`${name}: expected ${expected}, found ${actual ?? 'nothing'}`);
}

// Chosen by the builder, not fixed by the table: a color, and a darker indigo than --accent.
const hex = /^#[0-9a-f]{6}$/;
for (const name of ['--accent-hover', '--accent-text']) {
  if (!hex.test(light.get(name) ?? '')) problems.push(`${name}: expected a #rrggbb color, found ${light.get(name) ?? 'nothing'}`);
}
const luma = (value) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(value.slice(i, i + 2), 16));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
if (hex.test(light.get('--accent-hover') ?? '') && luma(light.get('--accent-hover')) >= luma(light.get('--accent'))) {
  problems.push(`--accent-hover (${light.get('--accent-hover')}) is not darker than --accent (${light.get('--accent')})`);
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} light tokens differ from the Compatto table`);
  process.exit(1);
}
console.log(`PASS ${TABLE.length + UNCHANGED.length + 2} light tokens match the Compatto table`);
