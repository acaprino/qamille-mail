// Reads each Svelte component's real props: the destructuring (for defaults) and its TypeScript type.
import fs from 'node:fs';
import path from 'node:path';
import { desktop, src } from './config.mjs';
import { readText } from './module-source.mjs';

const walk = (d) =>
  fs.readdirSync(d, { withFileTypes: true })
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .flatMap((e) => (e.isDirectory() ? (e.name === 'vendor' ? [] : walk(path.join(d, e.name))) : e.name.endsWith('.svelte') ? [path.join(d, e.name)] : []));
// The instance script: a component with a `<script module>` (Toaster) has two, and the props are in the other.
export const scriptOf = (file) => {
  const all = [...readText(file).matchAll(/<script([^>]*)>([\s\S]*?)<\/script>/g)];
  return (all.find((m) => !/\bmodule\b/.test(m[1])) ?? all[0] ?? [])[2] || '';
};

// Finds the closing partner of the bracket at `start`, skipping strings, comments and `=>`.
function closing(s, start) {
  const pairs = { '{': '}', '(': ')', '[': ']' };
  const stack = [];
  for (let i = start; i < s.length; i++) {
    const c = s[i];
    if (c === "'" || c === '"' || c === '`') { i++; while (i < s.length && s[i] !== c) { if (s[i] === '\\') i++; i++; } continue; }
    if (c === '/' && s[i + 1] === '/') { while (i < s.length && s[i] !== '\n') i++; continue; }
    if (c === '=' && s[i + 1] === '>') { i++; continue; }
    if (c === '{' || c === '(' || c === '[') stack.push(pairs[c]);
    else if (c === '}' || c === ')' || c === ']') { stack.pop(); if (!stack.length) return i; }
  }
  return -1;
}

export function readProps(file) {
  const script = scriptOf(file);
  const at = script.indexOf('$props()');
  if (at < 0) return { kind: 'none', members: [] };
  const open = script.lastIndexOf('let {', at);
  if (open < 0) return { kind: 'none', members: [] };
  const brace = script.indexOf('{', open);
  const end = closing(script, brace);
  const pattern = script.slice(brace + 1, end);
  const after = script.slice(end + 1, at);
  const type = after.replace(/^\s*:\s*/, '').replace(/=\s*$/, '').trim();
  const defaults = {};
  let depth = 0;
  let cur = '';
  const parts = [];
  for (const ch of pattern) {
    if ('{[('.includes(ch)) depth++;
    if ('}])'.includes(ch)) depth--;
    if (ch === ',' && depth === 0) { parts.push(cur); cur = ''; } else cur += ch;
  }
  if (cur.trim()) parts.push(cur);
  for (const p of parts) {
    // A prop is written `name`, `name = default`, `name: local` or, when it is not an identifier, `'aria-label': ariaLabel`.
    const m = p.trim().match(/^(?:'([\w-]+)'|([\w$]+))(?:\s*:\s*[\w$]+)?(?:\s*=\s*([\s\S]+))?$/);
    if (m) defaults[m[1] ?? m[2]] = m[3]?.trim();
  }
  return { kind: 'typed', type, defaults, names: Object.keys(defaults) };
}

/**
 * The declaration of a component's props type, ready for index.d.ts:
 * `{ kind: 'interface', body }` for a type written in the same file, `{ kind: 'alias', text }` for an
 * expression such as `HTMLButtonAttributes & { ... }`, or null when the type lives elsewhere.
 */
export function resolveType(file, type) {
  if (type.startsWith('{')) return { kind: 'interface', body: type };
  const script = scriptOf(file);
  // Only a bare name is looked up: a type with `|` or `&` in it would be a pattern of its own.
  const m = /^\w+$/.test(type) ? script.match(new RegExp(`(?:type|interface)\\s+${type}\\s*=?\\s*\\{`)) : null;
  if (m) {
    const start = m.index + m[0].length - 1;
    return { kind: 'interface', body: script.slice(start, closing(script, start) + 1) };
  }
  if (/^[\w.]+(<[^>]*>)?\s*&/.test(type) || /^[\w.]+Attributes\b/.test(type)) return { kind: 'alias', text: type.replace(/\n {2}/g, '\n') };
  return null;
}

/**
 * The `type X = ...` declarations of the component's script that its props type uses: the exported ones, and a
 * local one (`type Item = { ... }`) when the props mention it. An object type is read to its closing brace.
 */
export function declaredTypes(file, used = '') {
  const script = scriptOf(file);
  const out = [];
  for (const m of script.matchAll(/^[ \t]*(export )?type (\w+)\s*=\s*/gm)) {
    if (!m[1] && !new RegExp(`\\b${m[2]}\\b`).test(used)) continue;
    const at = m.index + m[0].length;
    const text = script[at] === '{' ? script.slice(at, closing(script, at) + 1) : script.slice(at, script.indexOf(';', at));
    out.push({ name: m[2], text: text.replace(/\n {2}/g, '\n').trim() });
  }
  return out;
}

export function allComponents() {
  const out = {};
  for (const f of walk(src)) {
    const name = path.basename(f, '.svelte');
    if (name === 'App') continue;
    out[name] = { file: path.relative(path.dirname(desktop), f).replaceAll('\\', '/'), abs: f };
  }
  return out;
}
