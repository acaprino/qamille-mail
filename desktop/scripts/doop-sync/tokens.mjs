#!/usr/bin/env node
// Three-way sync of design tokens between desktop/src/ds/tokens/theme.css and a Doop style guide.
//
//   export [--out file]                      theme.css -> guideline markdown (push to Doop with set_guidelines)
//   plan <guideline.md>                      show what changed in the app, in Doop, or in both
//   pull <guideline.md> [--write] [--add] [--prefer=doop|app]
//                                            apply Doop-only changes to theme.css (dry run without --write)
//   mark-synced <guideline.md> [--force]     record the current state as the common base
//
// "Base" is desktop/scripts/doop-sync/tokens.lock.json: the token values both sides had at the last sync.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const THEME = process.env.DOOP_SYNC_THEME || path.resolve(here, '../../src/ds/tokens/theme.css');
const LOCK = process.env.DOOP_SYNC_LOCK || path.join(here, 'tokens.lock.json');
const NOTES = path.join(here, 'tokens.notes.md');
const THEMES = ['light', 'dark'];
const SELECTOR = { light: ':root', dark: ":root[data-theme='dark']" };

const norm = (v) => v.replace(/\s+/g, ' ').trim().toLowerCase();
const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

function blocks(text) {
  const found = [];
  const re = /(:root(?:\[data-theme=['"]dark['"]\])?)\s*\{([^}]*)\}/g;
  for (let m; (m = re.exec(text)); ) {
    const open = m.index + m[0].indexOf('{') + 1;
    found.push({ theme: m[1].includes('dark') ? 'dark' : 'light', start: open, end: m.index + m[0].length - 1 });
  }
  return found;
}

function readTokens(text) {
  const out = { light: {}, dark: {} };
  for (const b of blocks(text)) {
    for (const t of text.slice(b.start, b.end).matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[b.theme][t[1]] = t[2].trim();
  }
  return out;
}

function setToken(text, theme, name, value, add) {
  const re = new RegExp(`(${escapeRe(name)}\\s*:\\s*)([^;]+)(;)`);
  for (const b of blocks(text).filter((x) => x.theme === theme)) {
    const inner = text.slice(b.start, b.end);
    if (re.test(inner)) return text.slice(0, b.start) + inner.replace(re, `$1${value}$3`) + text.slice(b.end);
  }
  if (!add) return text;
  const last = blocks(text).filter((x) => x.theme === theme).pop();
  return text.slice(0, last.end).replace(/\s*$/, '\n') + `  ${name}: ${value};\n` + text.slice(last.end);
}

function renderGuideline(tokens) {
  const block = (theme) =>
    `${SELECTOR[theme]} {\n${Object.entries(tokens[theme]).map(([k, v]) => `  ${k}: ${v};`).join('\n')}\n}`;
  const notes = fs.existsSync(NOTES) ? fs.readFileSync(NOTES, 'utf8').trim() : '';
  return [
    '# Qamille design tokens',
    '',
    'Source of truth in code: `desktop/src/ds/tokens/theme.css`. Edit the values inside the two CSS blocks. The app picks the changes up on the next pull (`desktop/scripts/doop-sync/tokens.mjs`). Do not rename or delete tokens here.',
    '',
    '## Light',
    '',
    '```css',
    block('light'),
    '```',
    '',
    '## Dark',
    '',
    '```css',
    block('dark'),
    '```',
    '',
    notes,
    '',
  ].join('\n');
}

function threeWay(base, app, doop) {
  const r = { pull: [], push: [], conflicts: [], newInDoop: [], missingInDoop: [] };
  for (const theme of THEMES) {
    const names = new Set([...Object.keys(app[theme]), ...Object.keys(doop[theme])]);
    for (const name of names) {
      const a = app[theme][name];
      const d = doop[theme][name];
      const b = base?.[theme]?.[name];
      if (a === undefined) { r.newInDoop.push({ theme, name, value: d }); continue; }
      if (d === undefined) { r.missingInDoop.push({ theme, name }); continue; }
      if (norm(a) === norm(d)) continue;
      const appChanged = b === undefined || norm(a) !== norm(b);
      const doopChanged = b === undefined || norm(d) !== norm(b);
      if (doopChanged && !appChanged) r.pull.push({ theme, name, app: a, doop: d });
      else if (appChanged && !doopChanged) r.push.push({ theme, name, app: a, doop: d });
      else r.conflicts.push({ theme, name, base: b, app: a, doop: d });
    }
  }
  return r;
}

const read = (file) => fs.readFileSync(path.resolve(file), 'utf8');
const readLock = () => (fs.existsSync(LOCK) ? JSON.parse(fs.readFileSync(LOCK, 'utf8')).tokens : null);
const line = (t, extra) => `  [${t.theme}] ${t.name}: ${extra}`;

function printPlan(r) {
  const section = (title, items, fmt) => {
    if (!items.length) return;
    console.log(`${title} (${items.length})`);
    items.forEach((t) => console.log(fmt(t)));
  };
  section('Doop -> app (will be applied by pull)', r.pull, (t) => line(t, `${t.app} -> ${t.doop}`));
  section('App -> Doop (export, then set_guidelines)', r.push, (t) => line(t, `Doop has ${t.doop}, app has ${t.app}`));
  section('Conflicts (changed on both sides)', r.conflicts, (t) => line(t, `base ${t.base ?? '?'} | app ${t.app} | Doop ${t.doop}`));
  section('New in Doop (pull --add to create)', r.newInDoop, (t) => line(t, t.value));
  section('Missing in Doop (ignored)', r.missingInDoop, (t) => line(t, 'not in the guide'));
  if (!r.pull.length && !r.push.length && !r.conflicts.length && !r.newInDoop.length) console.log('Tokens are in sync.');
}

const [cmd, ...rest] = process.argv.slice(2);
const flags = new Set(rest.filter((a) => a.startsWith('--')));
const prefer = rest.find((a) => a.startsWith('--prefer='))?.split('=')[1];
const outIdx = rest.indexOf('--out');
const positional = rest.filter((a, i) => !a.startsWith('--') && rest[i - 1] !== '--out');
const appText = fs.readFileSync(THEME, 'utf8');
const app = readTokens(appText);

if (cmd === 'export') {
  const md = renderGuideline(app);
  if (outIdx >= 0) { fs.writeFileSync(path.resolve(rest[outIdx + 1]), md); console.log(`Wrote ${rest[outIdx + 1]}`); }
  else process.stdout.write(md);
} else if (cmd === 'plan' || cmd === 'pull') {
  if (!positional[0]) throw new Error('Pass the guideline markdown file.');
  const doop = readTokens(read(positional[0]));
  const plan = threeWay(readLock(), app, doop);
  if (cmd === 'plan') { printPlan(plan); process.exitCode = plan.conflicts.length ? 2 : 0; }
  else {
    const todo = [...plan.pull];
    for (const c of plan.conflicts) if (prefer === 'doop') todo.push({ ...c, doop: c.doop });
    if (plan.conflicts.length && !prefer) { printPlan(plan); console.log('\nResolve the conflicts first: --prefer=doop or --prefer=app.'); process.exit(2); }
    let text = appText;
    for (const t of todo) text = setToken(text, t.theme, t.name, t.doop, false);
    if (flags.has('--add')) for (const t of plan.newInDoop) text = setToken(text, t.theme, t.name, t.value, true);
    printPlan(plan);
    if (flags.has('--write')) { fs.writeFileSync(THEME, text); console.log('\ntheme.css updated. Review, run the checks, then export, set_guidelines and mark-synced.'); }
    else console.log('\nDry run. Add --write to apply.');
  }
} else if (cmd === 'mark-synced') {
  if (!positional[0]) throw new Error('Pass the guideline markdown file.');
  const plan = threeWay(null, app, readTokens(read(positional[0])));
  const diff = plan.pull.length + plan.push.length + plan.conflicts.length;
  if (diff && !flags.has('--force')) { printPlan(plan); console.log('\nThe two sides differ. Sync them first, or pass --force.'); process.exit(2); }
  fs.writeFileSync(LOCK, JSON.stringify({ syncedAt: new Date().toISOString(), tokens: app }, null, 2) + '\n');
  console.log(`Base recorded in ${path.relative(process.cwd(), LOCK)}.`);
} else {
  console.log('Usage: tokens.mjs export [--out file] | plan <file> | pull <file> [--write] [--add] [--prefer=doop|app] | mark-synced <file> [--force]');
  process.exitCode = 1;
}
