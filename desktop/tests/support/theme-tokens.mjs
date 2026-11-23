// Reads the custom properties of the app's look, by scheme, as the browser resolves them: the tokens only Qamille has
// (desktop/src/theme/qamille-tokens.css, the :root block and the :root[data-scheme='dark'] block) and the ones Sigrid's
// engine computes (desktop/src/theme/compatto.css, generated), with every var() followed to its value. Pure file reads,
// no browser. `light` and `dark` are complete maps: what a scheme does not redeclare falls back on :root.
import fs from 'node:fs';
import path from 'node:path';

export const THEME_CSS = path.resolve(import.meta.dirname, '../../src/theme/qamille-tokens.css');
export const COMPATTO_CSS = path.resolve(import.meta.dirname, '../../src/theme/compatto.css');

function declarations(block) {
  const tokens = new Map();
  for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) tokens.set(match[1], match[2].trim().toLowerCase());
  return tokens;
}

const strip = (css) => css.replace(/\/\*[\s\S]*?\*\//g, '');

/** `value` with every `var(--name)` replaced by the value of that name in `tokens` (a name nothing writes stays as it is). */
function resolve(value, tokens, seen = new Set()) {
  return value.replace(/var\((--[\w-]+)\)/g, (whole, name) => {
    if (seen.has(name) || !tokens.has(name)) return whole;
    return resolve(tokens.get(name), tokens, new Set([...seen, name]));
  });
}

function resolved(tokens) {
  return new Map([...tokens].map(([name, value]) => [name, resolve(value, tokens, new Set([name]))]));
}

/** { light: Map(name -> value), dark: Map(name -> value) }, values lower-cased and resolved. */
export function readThemes(file = THEME_CSS, generated = COMPATTO_CSS) {
  const own = strip(fs.readFileSync(file, 'utf8'));
  const ownRoot = own.match(/(?:^|\n):root\s*\{([^}]*)\}/);
  const ownDark = own.match(/:root\[data-scheme=['"]dark['"]\]\s*\{([^}]*)\}/);
  if (!ownRoot) throw new Error(`${file}: no :root block`);
  if (!ownDark) throw new Error(`${file}: no :root[data-scheme='dark'] block`);
  const css = strip(fs.readFileSync(generated, 'utf8'));
  const root = css.match(/(?:^|\n):root\s*\{([^}]*)\}/);
  const light = css.match(/:root\[data-scheme=['"]light['"]\]\s*\{([^}]*)\}/);
  const dark = css.match(/:root\[data-scheme=['"]dark['"]\]\s*\{([^}]*)\}/);
  if (!root || !light || !dark) throw new Error(`${generated}: the :root, light and dark blocks are not all there`);
  const base = new Map([...declarations(ownRoot[1]), ...declarations(root[1])]);
  return {
    light: resolved(new Map([...base, ...declarations(light[1])])),
    dark: resolved(new Map([...base, ...declarations(ownDark[1]), ...declarations(dark[1])])),
  };
}
