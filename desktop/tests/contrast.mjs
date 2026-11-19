// WCAG 2.x contrast of the pairs of the theme, measured on src/ds/tokens/theme.css. Every text pair has to
// reach 4.5:1; the non-text pairs of the dark theme (field border, chosen tab, focus ring) 3:1. The themes to check are the arguments (one or more of `light`, `dark`); the dark block falls
// back on the :root value for a token it does not redeclare. Pure file check.
//   node tests/contrast.mjs light
//   node tests/contrast.mjs light dark
import { readThemes } from './support/theme-tokens.mjs';

const themes = process.argv.slice(2);
const KNOWN = ['light', 'dark'];
if (!themes.length || themes.some((t) => !KNOWN.includes(t))) {
  console.error(`usage: node tests/contrast.mjs <theme>...   (themes: ${KNOWN.join(', ')})`);
  process.exit(2);
}

const FLOOR = 4.5;
const FLOOR_UI = 3;
/** [foreground token, background token]. */
const PAIRS = [
  // Content on the white sheet
  ['text', 'surface'],
  ['text-secondary', 'surface'],
  ['muted', 'surface'],
  ['accent-text', 'surface'],
  ['accent-hover', 'surface'],
  ['accent-text', 'selection'],
  ['accent-text', 'badge'], // the count chip; dark --badge is picked to keep this at 4.5:1
  ['accent-text', 'avatar'],
  // Content on the filled accent
  ['on-accent', 'accent'],
  ['on-accent', 'accent-hover'],
  ['on-accent-soft', 'accent'],
  // Dark sidebar
  ['sidebar-text', 'sidebar'],
  ['sidebar-muted', 'sidebar'],
  ['accent-on-sidebar', 'sidebar'],
  ['sidebar-text', 'sidebar-hover'],
  ['sidebar-text-active', 'sidebar-active'],
  ['sidebar-muted', 'sidebar-active'],
  ['accent-on-sidebar', 'sidebar-active'],
];
/** Non-text pairs, measured on the dark theme only (the light values are the ones the user approved in the
 *  mockup, where --border-strong is #dde1e7 on white). The resting border of a field is --border-strong
 *  (ruling R16: input, textarea, select, the Select and DateField controls); the chosen tab and the focus
 *  ring are drawn in --accent. */
const UI_PAIRS = [
  ['border-strong', 'surface'],
  ['accent', 'surface'],
  ['accent', 'ground'],
];

const parse = (value, name) => {
  let h = value.startsWith('#') ? value.slice(1) : '';
  if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join('');
  if (!/^[0-9a-f]{6}(?:[0-9a-f]{2})?$/.test(h)) throw new Error(`--${name}: "${value}" is not a hex color`);
  const n = (i) => parseInt(h.slice(i, i + 2), 16);
  return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? n(6) / 255 : 1 };
};
const luminance = ({ r, g, b }) => {
  const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
};
const over = (fg, bg) => ({ r: fg.r * fg.a + bg.r * (1 - fg.a), g: fg.g * fg.a + bg.g * (1 - fg.a), b: fg.b * fg.a + bg.b * (1 - fg.a), a: 1 });

const all = readThemes();
const problems = [];
let measured = 0;
let total = 0;
for (const theme of themes) {
  const value = (name) => {
    const v = all[theme].get(`--${name}`) ?? all.light.get(`--${name}`);
    if (!v) throw new Error(`--${name} is not defined`);
    return v;
  };
  const pairs = [...PAIRS.map((p) => [...p, FLOOR]), ...(theme === 'dark' ? UI_PAIRS.map((p) => [...p, FLOOR_UI]) : [])];
  for (const [fg, bg, floor] of pairs) {
    total++;
    let ratio;
    try {
      const ground = parse(value(bg), bg);
      const ink = over(parse(value(fg), fg), ground);
      const [hi, lo] = [luminance(ink), luminance(ground)].sort((x, y) => y - x);
      ratio = (hi + 0.05) / (lo + 0.05);
    } catch (error) {
      problems.push(`[${theme}] --${fg} on --${bg}: ${error.message}`);
      continue;
    }
    measured++;
    const line = `[${theme}] --${fg} on --${bg}: ${ratio.toFixed(2)}:1`;
    if (ratio < floor) problems.push(`${line} (floor ${floor}:1)`);
    else console.log(`ok   ${line}`);
  }
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} of ${total} pairs are below their floor or cannot be measured`);
  process.exit(1);
}
console.log(`PASS ${measured} pairs reach their floor in ${themes.join(', ')}`);
