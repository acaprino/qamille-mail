// Reads the module's own files: tokens, fonts, icons, and a digest that names its state without a commit.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { desktop, ds, src } from './config.mjs';

// Text is read with LF line ends: a checkout with autocrlf must not change a byte of the output.
export const readText = (file) => fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');

const walk = (dir) =>
  fs.readdirSync(dir, { withFileTypes: true })
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .flatMap((e) => (e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)]));

/**
 * First 12 hex digits of a hash over what defines the look: every file of desktop/src/ds but the guide, by
 * relative path. The guide is exported on its own; a rewritten sentence must not change tokens.json.
 */
export function moduleDigest() {
  const hash = crypto.createHash('sha256');
  for (const file of walk(ds.dir).filter((f) => f !== ds.readme)) {
    hash.update(path.relative(ds.dir, file).replaceAll('\\', '/'));
    hash.update('\0');
    hash.update(readText(file));
    hash.update('\0');
  }
  return hash.digest('hex').slice(0, 12);
}

/** Every custom property of theme.css: `light` in source order, `dark` as the values that differ. */
export function readTheme() {
  const css = readText(ds.theme);
  const block = (re) => {
    const m = css.match(re);
    if (!m) throw new Error(`theme.css: block not found: ${re}`);
    return [...m[1].matchAll(/--([\w-]+):\s*([^;]+);/g)].map((x) => [x[1], x[2].trim()]);
  };
  return {
    light: block(/:root \{([\s\S]*?)\n\}/),
    dark: new Map(block(/:root\[data-theme='dark'\] \{([\s\S]*?)\n\}/)),
  };
}

/** The name and the path of each icon, from the `paths` table of Icon.svelte. */
export function readIcons() {
  const source = readText(path.join(ds.primitives, 'Icon.svelte'));
  const block = source.match(/paths: Record<string, string> = \{([\s\S]*?)\n  \};/);
  if (!block) throw new Error('Icon.svelte: the paths table was not found');
  const icons = [...block[1].matchAll(/(?:'([\w-]+)'|([\w-]+)):\s*(?:\n\s*)?'([^']+)'/g)].map((m) => [m[1] || m[2], m[3]]);
  if (!icons.length) throw new Error('Icon.svelte: no icons parsed');
  return icons;
}

/** The @font-face blocks of system.css: family, weight, style and the file under public/. */
export function readFonts() {
  const css = readText(ds.system);
  return [...css.matchAll(/@font-face\s*\{([^}]*)\}/g)].map((m) => {
    const body = m[1];
    const get = (prop) => body.match(new RegExp(`${prop}:\\s*([^;]+);`))?.[1].trim();
    const url = body.match(/url\('([^']+)'\)/)?.[1];
    if (!url) throw new Error('system.css: @font-face without a url');
    return {
      family: get('font-family').replace(/['"]/g, ''),
      weight: get('font-weight'),
      style: get('font-style'),
      file: path.basename(url),
      // The path under public/, as the stylesheet writes it, without the leading slash.
      rel: url.replace(/^\//, ''),
      source: path.join(desktop, 'public', url),
    };
  });
}

/** The bolt of the logo, from Sidebar.svelte. */
export function readBolt() {
  const sidebar = readText(path.join(src, 'components/Sidebar.svelte'));
  const d = sidebar.match(/d="(M12 1 2 14[^"]*)"/)?.[1];
  if (!d) throw new Error('Sidebar.svelte: the bolt path was not found');
  return d;
}
