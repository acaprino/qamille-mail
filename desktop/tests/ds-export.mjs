// The design system export is deterministic and complete (ADR-0011 v2: EX-007, INV-005).
//
//   node tests/ds-export.mjs                      runs scripts/ds-export.mjs twice into two temp folders
//   node tests/ds-export.mjs --a <dir> --b <dir>  checks two folders that already exist, without running it
//   node tests/ds-export.mjs --selftest           proves the comparison fails on a difference (no export runs)
//
// It asserts that: every file of the two runs is byte-identical; every token of theme.css is in the exported
// tokens with its value; each primitive of src/ds/primitives has a preview that shows what it
// is for; the guide is exported as written and
// states the type scale of theme.css.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const argv = process.argv.slice(2);
const at = (flag) => { const i = argv.indexOf(flag); return i >= 0 ? argv[i + 1] : undefined; };
// This test keeps its own list, and checks it against the folder: a primitive added without a card fails here.
const PRIMITIVES = [
  'Button', 'Field', 'Icon', 'Modal', 'SidePanel', 'Popover', 'Tooltip', 'Select', 'Checkbox', 'Switch', 'Tabs', 'Slider', 'DateField', 'RecipientsInput', 'Toaster',
];

const problems = [];
const fail = (message) => problems.push(message);

// Every file under `dir`, as relative path -> bytes.
function readTree(dir) {
  const out = new Map();
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) walk(full);
      else out.set(path.relative(dir, full).replaceAll('\\', '/'), fs.readFileSync(full));
    }
  };
  walk(dir);
  return out;
}

/** The files that differ between two trees, with what differs. Empty when they are identical. */
export function compare(a, b) {
  const diffs = [];
  for (const file of [...new Set([...a.keys(), ...b.keys()])].sort()) {
    if (!a.has(file)) { diffs.push(`${file}: only in the second run`); continue; }
    if (!b.has(file)) { diffs.push(`${file}: only in the first run`); continue; }
    const x = a.get(file);
    const y = b.get(file);
    if (x.equals(y)) continue;
    let i = 0;
    while (i < x.length && i < y.length && x[i] === y[i]) i++;
    const around = (buf) => JSON.stringify(buf.toString('utf8', Math.max(0, i - 30), i + 40));
    diffs.push(`${file}: differs at byte ${i} (${x.length} against ${y.length} bytes): ${around(x)} against ${around(y)}`);
  }
  return diffs;
}

if (argv.includes('--selftest')) {
  const same = new Map([['project/a.txt', Buffer.from('one')], ['project/b.txt', Buffer.from('two')]]);
  const copy = new Map([...same].map(([k, v]) => [k, Buffer.from(v)]));
  if (compare(same, copy).length) throw new Error('selftest: identical trees reported as different');
  copy.set('project/b.txt', Buffer.from('twp'));
  if (compare(same, copy).length !== 1) throw new Error('selftest: a changed byte was not reported');
  copy.set('project/b.txt', Buffer.from('two'));
  copy.set('project/c.txt', Buffer.from('extra'));
  if (compare(same, copy).length !== 1) throw new Error('selftest: an extra file was not reported');
  copy.delete('project/c.txt');
  copy.delete('project/a.txt');
  if (compare(same, copy).length !== 1) throw new Error('selftest: a missing file was not reported');
  console.log('ds-export selftest: the comparison reports a changed byte, an extra file and a missing one');
  process.exit(0);
}

// SKIPPED after the React port (ADR-0011 v3, "Consequences"): scripts/ds-export/ is still built on
// Svelte (Harness.svelte, `svelte-*` classes) and this test reads src/ds/primitives/*.svelte, which no
// longer exist. The exporter has to be rewritten for React before a new export; then remove this block.
// The --selftest above checks only the comparison and still runs.
console.log('SKIP ds-export: npm run ds:export is still Svelte-based and is not ported to React yet (docs/adr/ADR-0011-v3-modulo-sistema-visivo.md, Consequences)');
process.exit(0);

function runExport(dir) {
  const started = Date.now();
  const r = spawnSync(process.execPath, ['scripts/ds-export.mjs', '--out', dir], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, timeout: 20 * 60 * 1000 });
  const seconds = Math.round((Date.now() - started) / 1000);
  if (r.status !== 0) throw new Error(`ds-export failed (${r.status}) after ${seconds}s:\n${(r.stdout + r.stderr).split('\n').slice(-25).join('\n')}`);
  console.log(`  export into ${dir}: ${seconds}s`);
}

// --- Inputs read by this test on its own: it does not use the exporter's parsers.
const themeText = fs.readFileSync(path.join(root, 'src/ds/tokens/theme.css'), 'utf8').replace(/\r\n/g, '\n');
function themeTokens() {
  const block = (re) => [...(themeText.match(re)?.[1] ?? '').matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]);
  return { light: block(/:root \{([\s\S]*?)\n\}/), dark: new Map(block(/:root\[data-scheme='dark'\] \{([\s\S]*?)\n\}/)) };
}

function checkTokens(tree, label) {
  const raw = tree.get('project/tokens.json');
  if (!raw) return fail(`${label}: project/tokens.json is missing`);
  const t = JSON.parse(raw.toString('utf8'));
  // Every exported token by variable name, with the value(s) it carries.
  const exported = new Map();
  const add = (name, light, dark) => exported.set(name, { light, dark });
  for (const group of [t.radius, t.layout, t.zIndex, t.motion]) for (const x of group?.tokens ?? []) add(x.name, x.value);
  for (const x of t.shadow?.tokens ?? []) add(x.name, x.value.light, x.value.dark);
  for (const x of t.color?.tokens ?? []) add(x.name, x.value.light, x.value.dark);
  for (const g of t.type?.groups ?? []) for (const s of g.styles) add(s.name, s.fontSize);
  for (const [family, value] of Object.entries(t.type?.families ?? {})) add(`font-${family}`, value);

  const { light, dark } = themeTokens();
  if (!light.length) return fail('theme.css: no token found, the test cannot parse it');
  const names = new Set(light.map(([n]) => n));
  for (const [name, value] of light) {
    const x = exported.get(name);
    if (!x) { fail(`${label}: --${name} of theme.css is not in tokens.json`); continue; }
    if (String(x.light).toLowerCase() !== value.toLowerCase()) fail(`${label}: --${name} is ${x.light} in tokens.json and ${value} in theme.css`);
    if (dark.has(name) && x.dark !== undefined && String(x.dark).toLowerCase() !== dark.get(name).toLowerCase()) fail(`${label}: --${name} is ${x.dark} in the dark theme of tokens.json and ${dark.get(name)} in theme.css`);
    if (dark.has(name) && x.dark === undefined) fail(`${label}: --${name} has a dark value in theme.css and none in tokens.json`);
  }
  for (const name of dark.keys()) if (!names.has(name)) fail(`theme.css: --${name} is declared for the dark theme only`);
  for (const name of exported.keys()) if (!names.has(name)) fail(`${label}: tokens.json carries --${name}, which theme.css does not declare`);
  console.log(`  ${label}: ${light.length} tokens of theme.css, all in tokens.json`);
}

function checkPrimitives(tree, label) {
  const iconTable = fs.readFileSync(path.join(root, 'src/ds/primitives/Icon.svelte'), 'utf8');
  const icons = [...iconTable.matchAll(/^    (?:'[\w-]+'|[\w-]+):/gm)].length;
  // What each preview has to show for the primitive to be on the page at all.
  const needs = {
    Button: ['class="primary"', 'class="secondary"', 'class="icon-button"', 'class="text-button"', 'data-state="hover"', 'data-state="focus"', 'disabled'],
    Field: ['class="field"', '<input', '<textarea', 'field-help'],
    Icon: ['<svg', 'stroke-width="1.7"'],
    // One shot per size; a dialog is taken from <body> by its Ark attributes.
    Modal: ['data-scope="dialog"', 'data-variant="modal"', 'data-size="sm"', 'data-size="md"', 'data-size="lg"', 'modal-actions'],
    SidePanel: ['data-variant="side"', 'data-part="backdrop"', 'data-narrow', 'side-panel-bar', 'side-panel-body'],
    // The parts that exist only while open are in the card, opened by the capture.
    Popover: ['data-scope="popover"', 'data-part="content"', 'aria-expanded="true"', 'popover-divider'],
    Tooltip: ['data-scope="tooltip"', 'role="tooltip"', '<kbd>'],
    Select: ['data-scope="select"', 'role="combobox"', 'data-part="item"', 'data-placeholder-shown', 'data-disabled'],
    Checkbox: ['data-scope="checkbox"', 'type="checkbox"', 'data-state="checked"', 'data-state="indeterminate"', 'data-disabled'],
    Switch: ['data-scope="switch"', 'role="switch"', 'data-state="checked"', 'data-state="unchecked"', 'data-disabled'],
    Tabs: ['data-scope="tabs"', 'role="tablist"', 'role="tab"', 'data-variant="underline"', 'data-variant="segment"', 'class="count"'],
    Slider: ['data-scope="slider"', 'data-part="thumb"', 'role="slider"'],
    DateField: ['data-scope="date-picker"', 'data-part="input"', 'data-part="table-cell-trigger"', 'data-today'],
    RecipientsInput: ['data-scope="tags-input"', 'data-scope="combobox"', 'data-part="item"', 'data-invalid', 'data-part="content"'],
    Toaster: ['data-scope="toast"', 'role="status"', 'role="alert"', 'data-type="error"', 'data-part="close-trigger"'],
  };
  // Every file of primitives/ has a card, and every card is a primitive of the folder.
  const files = fs.readdirSync(path.join(root, 'src/ds/primitives')).filter((f) => f.endsWith('.svelte')).map((f) => f.replace('.svelte', '')).sort();
  if (files.join() !== [...PRIMITIVES].sort().join()) fail(`${label}: src/ds/primitives has [${files.join(', ')}], this test expects [${[...PRIMITIVES].sort().join(', ')}]`);
  // The ids come from Ark's counters: the export renumbers them, so a stray counter id would make two runs differ.
  const strayId = /\sid="(?!ds-)[^"]*"/;
  for (const name of PRIMITIVES) {
    const html = tree.get(`project/components/${name}/preview.html`)?.toString('utf8');
    if (!html) { fail(`${label}: ${name} has no preview`); continue; }
    if (!html.startsWith('<!-- @dsCard')) fail(`${label}: the preview of ${name} has no @dsCard marker`);
    if (!/subtitle="Anteprima montata dal componente del modulo"/.test(html)) fail(`${label}: the preview of ${name} is not marked as mounted from the module`);
    for (const needle of needs[name] ?? []) if (!html.includes(needle)) fail(`${label}: the preview of ${name} lacks ${needle}`);
    if (!needs[name]) fail(`${label}: this test says nothing about the preview of ${name}`);
    if (strayId.test(html)) fail(`${label}: the preview of ${name} carries an id that was not renumbered`);
    if (!tree.get(`project/components/${name}/README.md`)?.length) fail(`${label}: ${name} has no README.md`);
  }
  const grid = tree.get('project/components/Icon/preview.html')?.toString('utf8') ?? '';
  const drawn = (grid.match(/<svg\b/g) ?? []).length;
  if (icons < 10 || drawn !== icons) fail(`${label}: the Icon preview draws ${drawn} icons, Icon.svelte has ${icons}`);
  console.log(`  ${label}: ${PRIMITIVES.length} primitives, each with a preview (${icons} icons drawn)`);
}

function checkGuide(tree, label) {
  const source = fs.readFileSync(path.join(root, 'src/ds/README.md'), 'utf8').replace(/\r\n/g, '\n');
  const exported = tree.get('project/README.md')?.toString('utf8');
  if (exported !== source) fail(`${label}: project/README.md is not desktop/src/ds/README.md`);
  // The guide names the type scale: it has to be the one of theme.css.
  const sizes = themeTokens().light.filter(([n, v]) => n.startsWith('text-') && /px$/.test(v)).map(([, v]) => String(parseFloat(v)).replace('.', ','));
  const scale = `${sizes.slice(0, -1).join(', ')} e ${sizes.at(-1)} px`;
  if (!source.includes(scale)) fail(`the guide does not state the type scale of theme.css: "${scale}"`);
  for (const name of PRIMITIVES) if (!source.includes(`### ${name}`)) fail(`the guide has no section for ${name}`);
  console.log(`  ${label}: the guide is exported as written and states the scale ${scale}`);
}

function checkClean(tree, label, dirs) {
  const hits = [];
  for (const [file, data] of tree) {
    if (/\.(png|ttf|woff2)$/.test(file)) continue;
    const text = data.toString('utf8');
    const lower = text.toLowerCase();
    for (const d of dirs) for (const form of [d, d.replaceAll('\\', '/')]) if (lower.includes(form.toLowerCase())) hits.push(`${file}: ${form}`);
    for (const needle of ['127.0.0.1', 'localhost']) if (lower.includes(needle)) hits.push(`${file}: ${needle}`);
    if (/\b[A-Za-z]:[\\/]/.test(text)) hits.push(`${file}: a drive path`);
  }
  if (hits.length) fail(`${label}: files carry the machine they were made on:\n    ${hits.slice(0, 10).join('\n    ')}`);
  else console.log(`  ${label}: no absolute path and no server address in any file`);
}

// --- Run.
const tmp = [];
let dirA = at('--a');
let dirB = at('--b');
try {
  if (!dirA || !dirB) {
    dirA = fs.mkdtempSync(path.join(os.tmpdir(), 'qamille-ds-a-'));
    dirB = fs.mkdtempSync(path.join(os.tmpdir(), 'qamille-ds-b-'));
    tmp.push(dirA, dirB);
    console.log('running the export twice');
    runExport(dirA);
    runExport(dirB);
  }
  const a = readTree(path.resolve(dirA));
  const b = readTree(path.resolve(dirB));
  const diffs = compare(a, b);
  if (diffs.length) fail(`the two exports differ in ${diffs.length} file(s):\n    ${diffs.slice(0, 20).join('\n    ')}`);
  else console.log(`  ${a.size} files, byte-identical in the two runs`);
  for (const [label, tree] of [['run 1', a], ['run 2', b]]) {
    checkTokens(tree, label);
    checkPrimitives(tree, label);
    checkGuide(tree, label);
    checkClean(tree, label, [path.resolve(dirA), path.resolve(dirB), os.tmpdir(), root]);
  }
} finally {
  for (const d of tmp) fs.rmSync(d, { recursive: true, force: true });
}

if (problems.length) {
  console.error(`\nds-export: ${problems.length} problem(s)\n  ${problems.join('\n  ')}`);
  process.exit(1);
}
console.log('ds-export: ok');
