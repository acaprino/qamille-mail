// tokens.json from theme.css: every variable, its exact value, in source order, with a usage note.
// Every token has to land somewhere: a variable of a kind this file does not know is an error, so that a
// new kind of token cannot be left out of the export unnoticed.
import { USAGE, OTHER_USAGE, SIZE_ROLES } from './notes-it.mjs';
import { measure, warnings } from './contrast.mjs';

const isColor = (v) => /^#[0-9a-fA-F]{3,8}$/.test(v);
const need = (table, name) => {
  if (!table[name]) throw new Error(`tokens: no usage note for --${name} (add it to scripts/ds-export/notes-it.mjs)`);
  return table[name];
};

/** `theme` comes from readTheme(); `fonts` from readFonts(); `digest` from moduleDigest(). */
export function buildTokens({ theme, fonts, digest }) {
  const { light, dark } = theme;
  const colors = [];
  const radius = [];
  const shadow = [];
  const layout = [];
  const zIndex = [];
  const motion = [];
  const families = {};
  const sizes = new Map();
  for (const [name, value] of light) {
    if (name.startsWith('text-') && /px$/.test(value)) { sizes.set(name, value); continue; }
    if (name.startsWith('font-')) { families[name.slice('font-'.length)] = value; continue; }
    if (name.startsWith('radius-')) { radius.push({ name, value, usage: need(OTHER_USAGE, name) }); continue; }
    if (name.startsWith('shadow')) { shadow.push({ name, value: { light: value, dark: dark.get(name) ?? value }, usage: need(OTHER_USAGE, name) }); continue; }
    if (/^(panel|modal|recipients|control)-/.test(name)) { layout.push({ name, value, usage: need(OTHER_USAGE, name) }); continue; }
    if (name.startsWith('z-')) { zIndex.push({ name, value, usage: need(OTHER_USAGE, name) }); continue; }
    if (name.startsWith('motion-') || name.startsWith('ease-')) { motion.push({ name, value, usage: need(OTHER_USAGE, name) }); continue; }
    if (isColor(value)) {
      const token = { name, value: { light: value.toLowerCase() }, usage: need(USAGE, name) };
      if (dark.has(name)) token.value.dark = dark.get(name).toLowerCase();
      colors.push(token);
      continue;
    }
    throw new Error(`tokens: --${name}: ${value} has no place in tokens.json (extend scripts/ds-export/tokens.mjs)`);
  }
  const lightNames = new Set(light.map(([n]) => n));
  for (const n of dark.keys()) if (!lightNames.has(n)) throw new Error(`tokens: --${n} is declared for the dark theme only`);

  // Under-the-floor contrast goes into the note of the foreground token.
  const warn = warnings(measure(Object.fromEntries(colors.map((t) => [t.name, t.value]))));
  for (const t of colors) if (warn[t.name]) t.usage = `${t.usage} ${warn[t.name]}`;

  const styles = [...sizes].map(([name, fontSize]) => {
    const role = SIZE_ROLES[name];
    if (!role) throw new Error(`tokens: no type role for --${name} (add it to scripts/ds-export/notes-it.mjs)`);
    return { name, fontSize, lineHeight: role[2], fontWeight: role[1], usage: role[0], family: role[3] };
  });
  const group = (name, family) => ({
    name,
    family,
    styles: styles.filter((s) => s.family === family).map((s) => ({ name: s.name, fontSize: s.fontSize, lineHeight: s.lineHeight, fontWeight: s.fontWeight, usage: s.usage })),
  });
  for (const key of Object.keys(families)) if (!styles.some((s) => s.family === key)) throw new Error(`tokens: font family --font-${key} is used by no size`);

  const tokens = {
    name: 'Qamille',
    version: 1,
    meta: {
      source: 'local-checkout',
      ref: `desktop/src/ds@${digest}`,
      package: 'desktop',
      paths: {
        tokens: ['desktop/src/ds/tokens/theme.css'],
        fonts: fonts.map((f) => `desktop/public/${f.rel}`),
        styles: ['desktop/src/ds/styles/system.css', 'desktop/src/ds/styles/ark.css'],
      },
    },
    color: { themes: [{ id: 'light', name: 'Chiaro' }, { id: 'dark', name: 'Scuro' }], tokens: colors },
    type: {
      // IBM Plex Sans is two subsets of one family (latin-ext, then latin: the last one wins where they overlap).
      // tokens.json has no field for a unicode-range, so the order is the module's own and Latin text, Italian
      // included, comes from the second file.
      fonts: fonts.map((f) => ({ family: f.family, file: `fonts/${f.file}`, weight: f.weight, style: f.style })),
      families,
      groups: [group('Testo', 'text'), group('Titoli', 'title')],
    },
    radius: { tokens: radius },
    shadow: { tokens: shadow },
    layout: { note: 'Fixed container widths, and the height of the controls.', tokens: layout },
    zIndex: { note: 'Stacking order of the app. The parts Ark moves out of the flow stay on this ladder too.', tokens: zIndex },
    motion: { note: 'Duration and curve of the entrances the app animates: side panel, dialog, popover, lists, notices, tooltip, switch.', tokens: motion },
  };
  return {
    tokens,
    summary: `colors ${colors.length}, type ${styles.length}, radius ${radius.length}, shadow ${shadow.length}, layout ${layout.length}, z ${zIndex.length}, motion ${motion.length}, families ${Object.keys(families).length}`,
    count: light.length,
  };
}
