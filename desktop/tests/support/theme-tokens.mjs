// Reads the custom properties of desktop/src/ds/tokens/theme.css by block: `light` is the :root
// block, `dark` the :root[data-scheme='dark'] block. Pure file reads, no browser.
import fs from 'node:fs';
import path from 'node:path';

export const THEME_CSS = path.resolve(import.meta.dirname, '../../src/ds/tokens/theme.css');

function declarations(block) {
  const tokens = new Map();
  for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) tokens.set(match[1], match[2].trim().toLowerCase());
  return tokens;
}

/** { light: Map(name -> value), dark: Map(name -> value) }, values lower-cased. */
export function readThemes(file = THEME_CSS) {
  const css = fs.readFileSync(file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  const light = css.match(/(?:^|\n):root\s*\{([^}]*)\}/);
  const dark = css.match(/:root\[data-scheme=['"]dark['"]\]\s*\{([^}]*)\}/);
  if (!light) throw new Error(`${file}: no :root block`);
  if (!dark) throw new Error(`${file}: no :root[data-scheme='dark'] block`);
  return { light: declarations(light[1]), dark: declarations(dark[1]) };
}
