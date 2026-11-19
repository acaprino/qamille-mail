// WCAG contrast of the text and mark pairs the usage notes name, measured on the module's own tokens in both
// themes. A pair below its floor adds a sentence to the foreground token's usage note.
const parse = (hex) => {
  let h = hex.slice(1);
  if (h.length <= 4) h = [...h].map((c) => c + c).join('');
  const n = (i) => parseInt(h.slice(i, i + 2), 16);
  return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? n(6) / 255 : 1 };
};
const lum = ({ r, g, b }) => {
  const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
};
const over = (fg, bg) => ({ r: fg.r * fg.a + bg.r * (1 - fg.a), g: fg.g * fg.a + bg.g * (1 - fg.a), b: fg.b * fg.a + bg.b * (1 - fg.a), a: 1 });

// [foreground, ground, floor]: 4.5 for text, 3 for marks and borders that carry meaning.
export const PAIRS = [
  ['text', 'surface', 4.5], ['text', 'ground', 4.5], ['text', 'hover', 4.5], ['text', 'selection', 4.5],
  ['text-secondary', 'surface', 4.5], ['body-text', 'surface', 4.5],
  ['muted', 'surface', 4.5], ['muted', 'ground', 4.5], ['muted', 'hover', 4.5],
  ['nav-text', 'ground', 4.5], ['accent-text', 'surface', 4.5], ['accent-text', 'selection', 4.5], ['accent-text', 'badge', 4.5],
  ['on-accent', 'accent', 4.5], ['on-accent-soft', 'accent', 4.5], ['on-accent', 'accent-hover', 4.5],
  ['priority-text', 'priority-bg', 4.5], ['priority-text', 'priority-wash', 4.5], ['label-text', 'label-bg', 4.5],
  ['success', 'success-bg', 4.5], ['danger', 'surface', 4.5], ['danger', 'danger-bg', 4.5],
  ['avatar-text', 'sender-1', 4.5], ['avatar-text', 'sender-2', 4.5], ['avatar-text', 'sender-3', 4.5], ['avatar-text', 'sender-4', 4.5],
  ['accent-text', 'avatar', 4.5], ['paper-text', 'paper', 4.5], ['paper-muted', 'paper', 4.5], ['on-window-close', 'window-close', 4.5],
  ['priority-mark', 'surface', 3], ['priority-mark', 'priority-wash', 3],
  ['label-1', 'ground', 3], ['label-2', 'ground', 3], ['label-3', 'ground', 3], ['label-4', 'ground', 3], ['label-5', 'ground', 3],
  ['border-strong', 'surface', 3], ['accent', 'surface', 3], ['border', 'surface', 3],
];

/** `colors`: name -> { light, dark? }. Every pair is measured; a name that is not a color token is an error. */
export function measure(colors) {
  const val = (name, theme) => {
    const v = colors[name];
    if (!v) throw new Error(`contrast: --${name} is not a color token`);
    return v[theme] ?? v.light;
  };
  const out = [];
  for (const [fg, bg, floor] of PAIRS) {
    const ratios = {};
    for (const theme of ['light', 'dark']) {
      const ground = parse(val(bg, theme));
      const ink = over(parse(val(fg, theme)), ground);
      const [a, b] = [lum(ink), lum(ground)].sort((x, y) => y - x);
      ratios[theme] = (a + 0.05) / (b + 0.05);
    }
    out.push({ fg, bg, floor, ratios });
  }
  return out;
}

const it = (n) => n.toFixed(2).replace('.', ',');
const floorIt = (n) => String(n).replace('.', ',');

/** token name -> the sentence to add to its usage note (only for pairs under their floor). */
export function warnings(measured) {
  const by = {};
  for (const { fg, bg, floor, ratios } of measured) {
    const low = ['light', 'dark'].filter((t) => ratios[t] < floor);
    if (!low.length) continue;
    const names = { light: 'tema chiaro', dark: 'tema scuro' };
    const parts = low.map((t) => `${it(ratios[t])}:1 nel ${names[t]}`).join(' e ');
    (by[fg] ??= []).push(`Su \`${bg}\` ${parts}, sotto ${floorIt(floor)}:1.`);
  }
  return Object.fromEntries(Object.entries(by).map(([k, v]) => [k, v.join(' ')]));
}
