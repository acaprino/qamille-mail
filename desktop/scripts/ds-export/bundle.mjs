// components/bundle.css: the app's compiled stylesheet, minus what tokens.json declares (so that a token edited
// on the design system page wins), plus the frame of a preview card and the forced states of the controls.
import fs from 'node:fs';
import path from 'node:path';
import { ds, here } from './config.mjs';
import { readText } from './module-source.mjs';

const byName = (a, b) => (a.startsWith('main') === b.startsWith('main') ? (a < b ? -1 : a > b ? 1 : 0) : a.startsWith('main') ? -1 : 1);

// The top-level rules of a stylesheet as [prelude, body] pairs; at-rules are kept whole.
function topLevel(text) {
  const out = [];
  let depth = 0;
  let begin = 0;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '{') depth++;
    else if (text[i] === '}') {
      depth--;
      if (depth === 0) {
        const block = text.slice(begin, i + 1);
        const open = block.indexOf('{');
        out.push([block.slice(0, open).trim(), block.slice(open + 1, -1), block]);
        begin = i + 1;
      }
    }
  }
  return out;
}

// The reminder chip lives in its own window: keep only its scoped rules, not its body and font rules.
const scopedOnly = (text) => topLevel(text).filter(([prelude]) => prelude.includes('svelte-')).map(([, , block]) => block).join('');

// `:hover` and `:focus-visible` cannot be forced in a static preview, so each such rule of the controls gets a
// twin keyed to an attribute: data-state="hover" or "focus" on the control stands for the real state.
const CONTROLS = /^(\.primary|\.secondary|\.icon-button|\.text-button|\.popover button|button|input|textarea|select)\b/;
function forcedStates(css) {
  const twins = [];
  for (const [prelude, body] of topLevel(css)) {
    if (prelude.startsWith('@')) continue;
    const kept = prelude.split(',').map((s) => s.trim()).filter((s) => /:(hover|focus-visible)/.test(s) && CONTROLS.test(s));
    if (!kept.length) continue;
    const twin = kept.map((s) => s.replace(/:hover/g, '[data-state="hover"]').replace(/:focus-visible/g, '[data-state="focus"]'));
    twins.push(`${twin.join(',')}{${body}}`);
  }
  return twins;
}

/** `declared`: names of the variables tokens.json carries. `buildDir`: the output of `vite build`. */
export function buildBundle({ buildDir, declared }) {
  const assets = path.join(buildDir, 'assets');
  const files = fs.readdirSync(assets).filter((f) => f.endsWith('.css')).sort(byName);
  const main = files.filter((f) => !f.startsWith('reminder'));
  const reminder = files.filter((f) => f.startsWith('reminder'));
  let css = main.map((f) => readText(path.join(assets, f))).join('\n');
  css = css.replace(/@font-face\{[^}]*\}/g, '');
  css = css.replace(/(:root(?:\[data-theme=["']?dark["']?\])?)\{([^}]*)\}/g, (all, sel, body) => {
    const keep = body.split(';').filter((d) => { const m = d.trim().match(/^--([\w-]+):/); return !m || !declared.has(m[1]); }).filter((d) => d.trim());
    return keep.length ? `${sel}{${keep.join(';')}}` : '';
  });
  // The 1px window frame belongs to the app window, not to a preview card.
  css = css.replace(/@media screen\{body:after\{[^}]*\}\}/, '');
  if (/body:after/.test(css)) throw new Error('bundle: the window frame rule is still in the stylesheet');
  css += reminder.map((f) => scopedOnly(readText(path.join(assets, f)))).join('');
  const leftover = [...css.matchAll(/url\(([^)]*)\)/g)].map((m) => m[1]).filter((u) => !u.startsWith('data:') && !u.startsWith('"data:'));
  if (leftover.length) throw new Error(`bundle: urls a preview cannot load: ${[...new Set(leftover)].join(', ')}`);
  // The Ark parts are styled by ark.css and the files it imports: each scope they select on has to be in the build,
  // or the previews of the Ark primitives would be drawn without their rules.
  const arkFiles = [ds.ark, ...[...readText(ds.ark).matchAll(/@import url('([^']+)')/g)].map((m) => path.join(path.dirname(ds.ark), m[1]))];
  const scopes = new Set(arkFiles.flatMap((f) => [...readText(f).matchAll(/data-scope=['"]?([\w-]+)/g)].map((m) => m[1])));
  const missing = [...scopes].filter((scope) => !new RegExp(`data-scope=["']?${scope}["']?`).test(css));
  if (!scopes.size || missing.length) throw new Error(`bundle: no rule for the Ark scope(s) ${missing.join(', ') || '(none found in ark.css)'}: ark.css and its imports are not all in the build`);
  const twins = forcedStates(css);
  if (!twins.length) throw new Error('bundle: no :hover or :focus-visible rule of the controls was found');
  const frame = readText(path.join(here, 'harness/preview.css'));
  return `${css}\n${frame}\n/* Forced states of the previews: data-state="hover" and "focus" stand for :hover and :focus-visible. */\n${twins.join('\n')}\n`;
}
