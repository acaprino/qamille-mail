// What Qamille waits for from Sigrid (plan docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md, «Che cosa chiedere a
// Sigrid»), checked on a copy of Sigrid. The descriptions here and there say the same thing: whoever changes one
// changes the other.
//   P2  Base UI and Ark live together inside Sigrid: tests/sigrid-coexistence.mjs, in the browser.
//   P3  The radii as a parameter of the theme definition: with `radii: { sm: 4, md: 6, lg: 8, xl: 12 }`,
//       themeTokens(...).shape has --radius-lg 8px and --radius-xl 12px.
//   P4  Title steps above `base`: src/styles/theme.css defines a --text-* of at least 1.625rem, with its line height.
//   P5  The icon meanings of Qamille: every target name of ICON_MAP in src/ui/icon-names.ts (Task 19) is in
//       ICON_NAMES. Until that file exists, the check lists the icons Qamille draws today whose name Sigrid lacks.
// P1 is withdrawn (the owner's rule of 2026-11-22: Sigrid's bundle may carry Ark and Motion).
//
//   node --test tests/sigrid-preconditions.mjs            the checks on fake copies; the synced copy is printed only
//   node tests/sigrid-preconditions.mjs --require P2,P5   on desktop/vendor/sigrid: exit 1 unless each one named holds
// A task that waits for a prerequisite runs the second form at its Step 0, and does not start if it fails.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const desktop = path.resolve(import.meta.dirname, '..');
const requireHere = createRequire(import.meta.url);
const ts = createRequire(path.join(desktop, 'package.json'))('typescript');
const VENDOR = path.join(desktop, 'vendor', 'sigrid');
const KNOWN = ['P2', 'P3', 'P4', 'P5'];

/** Sigrid's built module of the copy in `dir` (an ES module: Node 22.12 and later load it with require). */
function loadSigrid(dir) {
  return requireHere(path.join(dir, 'dist', 'index.js'));
}

function radii(dir) {
  let sigrid;
  try {
    sigrid = loadSigrid(dir);
  } catch (error) {
    return { ok: false, why: `dist/index.js does not load: ${error.message.split('\n')[0]}` };
  }
  if (typeof sigrid.themeTokens !== 'function' || !sigrid.BASE_THEME) return { ok: false, why: 'dist/index.js has no themeTokens or BASE_THEME' };
  // The name proposed to Sigrid is `radii`: if Sigrid picks another one, only this line changes.
  const shape = sigrid.themeTokens({ ...sigrid.BASE_THEME, radii: { sm: 4, md: 6, lg: 8, xl: 12 } }, 'light')?.shape ?? {};
  const lg = shape['--radius-lg'];
  const xl = shape['--radius-xl'];
  return lg === '8px' && xl === '12px'
    ? { ok: true, why: 'ThemeDef.radii gives --radius-lg 8px and --radius-xl 12px' }
    : { ok: false, why: `with radii { sm: 4, md: 6, lg: 8, xl: 12 } themeTokens gives --radius-lg ${lg} and --radius-xl ${xl}, not 8px and 12px` };
}

/** The size of a step in px (1rem = 16px), or null when it is not a plain length. */
const pixels = (value) => {
  const match = /^([\d.]+)(rem|px)$/.exec(value.trim());
  return match ? Number(match[1]) * (match[2] === 'rem' ? 16 : 1) : null;
};

function titleSteps(dir) {
  const file = path.join(dir, 'src', 'styles', 'theme.css');
  if (!fs.existsSync(file)) return { ok: false, why: 'src/styles/theme.css is missing' };
  const css = fs.readFileSync(file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  const sizes = new Map([...css.matchAll(/--text-([a-z0-9]+(?:-[a-z0-9]+)*)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
  const leading = new Set([...css.matchAll(/--text-([a-z0-9-]+?)--line-height\s*:/g)].map((m) => m[1]));
  const steps = [...sizes].map(([name, value]) => ({ name, value, px: pixels(value) })).filter((step) => step.px !== null);
  const title = steps.find((step) => step.px >= 26 && leading.has(step.name));
  if (title) return { ok: true, why: `--text-${title.name} is ${title.value}, with its line height` };
  const largest = steps.sort((a, b) => b.px - a.px)[0];
  const large = steps.find((step) => step.px >= 26);
  return {
    ok: false,
    why: large
      ? `--text-${large.name} (${large.value}) has no --text-${large.name}--line-height`
      : `no --text-* reaches 1.625rem: the largest is ${largest ? `--text-${largest.name} (${largest.value})` : 'none'}`,
  };
}

/** The string values of the object literal given to `const <name>` in a TypeScript file, with their keys. */
function objectValues(file, name) {
  const tree = ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  let found = null;
  const unwrap = (node) => (node && (ts.isAsExpression(node) || ts.isSatisfiesExpression(node) || ts.isParenthesizedExpression(node) || ts.isTypeAssertionExpression(node)) ? unwrap(node.expression) : node);
  const visit = (node) => {
    if (!found && ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === name) {
      const value = unwrap(node.initializer);
      if (value && ts.isObjectLiteralExpression(value)) {
        found = value.properties.filter(ts.isPropertyAssignment).map((property) => {
          const key = ts.isIdentifier(property.name) || ts.isStringLiteralLike(property.name) ? property.name.text : property.name.getText(tree);
          const target = unwrap(property.initializer);
          return { key, value: target && ts.isStringLiteralLike(target) ? target.text : null };
        });
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return found;
}

function iconMeanings(dir, src) {
  let names;
  try {
    names = loadSigrid(dir).ICON_NAMES;
  } catch (error) {
    return { ok: false, why: `dist/index.js does not load: ${error.message.split('\n')[0]}` };
  }
  if (!Array.isArray(names)) return { ok: false, why: 'dist/index.js has no ICON_NAMES' };
  const map = path.join(src, 'ui', 'icon-names.ts');
  if (fs.existsSync(map)) {
    const entries = objectValues(map, 'ICON_MAP');
    if (!entries?.length) return { ok: false, why: 'src/ui/icon-names.ts has no ICON_MAP object' };
    const missing = [...new Set(entries.filter((entry) => entry.value === null || !names.includes(entry.value)).map((entry) => entry.value ?? `${entry.key} (no name)`))];
    return missing.length
      ? { ok: false, why: `ICON_NAMES lacks ${missing.length} of the names ICON_MAP needs: ${missing.join(', ')}` }
      : { ok: true, why: `every one of the ${entries.length} names of ICON_MAP is in ICON_NAMES` };
  }
  // Before Task 19: the names of the icons Qamille draws today (iconPaths of its Icon primitive).
  const today = path.join(src, 'ds', 'primitives', 'Icon.tsx');
  const drawn = fs.existsSync(today) ? (objectValues(today, 'iconPaths') ?? []).map((entry) => entry.key) : [];
  const missing = drawn.filter((icon) => !names.includes(icon));
  return {
    ok: false,
    why: `src/ui/icon-names.ts is not there yet (Task 19); of the ${drawn.length} icons Qamille draws today, ${missing.length} have a name that is not in ICON_NAMES: ${missing.join(', ')}`,
  };
}

/**
 * P3, P4 and P5 on the copy of Sigrid in `dir` (a folder like desktop/vendor/sigrid). `src` is Qamille's src folder,
 * where P5 reads the icons.
 */
export function checkPreconditions(dir, { src = path.join(desktop, 'src') } = {}) {
  return { P3: radii(dir), P4: titleSteps(dir), P5: iconMeanings(dir, src) };
}

/** The names after --require, checked: P2 to P5. */
export function requiredNames(argv) {
  const at = argv.indexOf('--require');
  if (at === -1) return null;
  const names = (argv[at + 1] ?? '').split(',').map((name) => name.trim().toUpperCase()).filter(Boolean);
  if (!names.length) throw new Error('--require needs a list, as in --require P2,P5');
  const unknown = names.filter((name) => !KNOWN.includes(name));
  if (unknown.length) throw new Error(`unknown prerequisite ${unknown.join(', ')}: they are ${KNOWN.join(', ')} (P1 is withdrawn)`);
  return names;
}

/** P2: the browser test, through test-browser.mjs (its own Vite) unless a preview is already running for it. */
function coexistence() {
  const args = process.env.QAMILLE_PREVIEW_URL ? ['tests/sigrid-coexistence.mjs'] : ['scripts/test-browser.mjs', 'sigrid-coexistence'];
  const run = spawnSync(process.execPath, args, { cwd: desktop, stdio: 'inherit', timeout: 600_000 });
  return run.status === 0
    ? { ok: true, why: 'tests/sigrid-coexistence.mjs passes' }
    : { ok: false, why: 'tests/sigrid-coexistence.mjs fails: the pairs that do not hold are listed above' };
}

function main(names) {
  if (!fs.existsSync(path.join(VENDOR, 'provenance.json'))) {
    console.error('Sigrid manca / Sigrid is missing: npm run sigrid:sync in desktop/');
    return 1;
  }
  const { version } = JSON.parse(fs.readFileSync(path.join(VENDOR, 'provenance.json'), 'utf8'));
  const results = { ...checkPreconditions(VENDOR), ...(names.includes('P2') ? { P2: coexistence() } : {}) };
  console.log(`Sigrid ${version} (desktop/vendor/sigrid):`);
  for (const name of KNOWN) {
    if (!results[name]) continue;
    console.log(`  ${name} ${results[name].ok ? 'ok  ' : 'FAIL'} ${names.includes(name) ? '(required) ' : ''}${results[name].why}`);
  }
  const missing = names.filter((name) => !results[name].ok);
  if (missing.length) console.log(`Not there yet: ${missing.join(', ')}. The task does not start: the request is in the plan, «Che cosa chiedere a Sigrid».`);
  return missing.length ? 1 : 0;
}

const entry = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
const names = entry ? (() => {
  try {
    return requiredNames(process.argv);
  } catch (error) {
    console.error(error.message);
    process.exit(2);
  }
})() : null;

if (names) process.exitCode = main(names);
else if (entry) {
  // The checks, on fake copies under desktop/.cache (ignored by git), not in the system's temp folder.
  const cache = path.join(desktop, '.cache');
  fs.mkdirSync(cache, { recursive: true });
  const scratch = fs.mkdtempSync(path.join(cache, 'test-sigrid-preconditions-'));
  test.after(() => fs.rmSync(scratch, { recursive: true, force: true }));
  let counter = 0;
  const write = (root, files) => {
    for (const [name, text] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
      fs.writeFileSync(path.join(root, name), text);
    }
    return root;
  };
  /** A fake copy of Sigrid: dist/index.js with what the checks read, and src/styles/theme.css. */
  const fakeSigrid = ({ radii: takesRadii = false, steps = "  --text-base: 1rem;\n  --text-base--line-height: 1.5;\n", icons = ['settings', 'mail'] } = {}) =>
    write(path.join(scratch, `sigrid-${++counter}`), {
      'package.json': '{ "type": "module" }\n',
      'dist/index.js': [
        "export const BASE_THEME = { id: 'base' };",
        `export const ICON_NAMES = ${JSON.stringify(icons)};`,
        'export function themeTokens(theme) {',
        takesRadii
          ? "  const r = theme.radii ?? { sm: 4, md: 6, lg: 10, xl: 10 };\n  return { shape: { '--radius-sm': `${r.sm}px`, '--radius-md': `${r.md}px`, '--radius-lg': `${r.lg}px`, '--radius-xl': `${r.xl}px` } };"
          : "  return { shape: { '--radius-sm': '4px', '--radius-md': '6px', '--radius-lg': '10px', '--radius-xl': '10px' } };",
        '}',
      ].join('\n'),
      'src/styles/theme.css': `@theme {\n  --text-sm: 0.875rem;\n  --text-sm--line-height: calc(1.25 / 0.875);\n${steps}}\n`,
    });
  /** A fake desktop/src: today's Icon primitive and, when given, src/ui/icon-names.ts. */
  const fakeSrc = ({ drawn = ['settings', 'inbox', 'trash'], map } = {}) =>
    write(path.join(scratch, `src-${++counter}`), {
      'ds/primitives/Icon.tsx': `export const iconPaths: Record<string, string> = {\n${drawn.map((name) => `  '${name}': 'M0 0',`).join('\n')}\n};\n`,
      ...(map ? { 'ui/icon-names.ts': map } : {}),
    });

  test('P3 holds when the theme definition takes the radii, and fails when it does not', () => {
    assert.equal(checkPreconditions(fakeSigrid({ radii: true }), { src: fakeSrc() }).P3.ok, true);
    const without = checkPreconditions(fakeSigrid(), { src: fakeSrc() }).P3;
    assert.equal(without.ok, false);
    assert.match(without.why, /--radius-lg 10px and --radius-xl 10px/);
  });

  test('P3 and P5 fail, saying why, on a copy whose module does not load', () => {
    const broken = fakeSigrid();
    fs.writeFileSync(path.join(broken, 'dist', 'index.js'), 'export const = ;\n');
    const result = checkPreconditions(broken, { src: fakeSrc() });
    assert.equal(result.P3.ok, false);
    assert.match(result.P3.why, /does not load/);
    assert.equal(result.P5.ok, false);
  });

  test('P4 holds with a step of at least 1.625rem and its line height', () => {
    const steps = '  --text-title: 1.625rem;\n  --text-title--line-height: 1.25;\n';
    assert.deepEqual(checkPreconditions(fakeSigrid({ steps }), { src: fakeSrc() }).P4, { ok: true, why: '--text-title is 1.625rem, with its line height' });
  });

  test('P4 fails when the scale stops at base, or a large step has no line height', () => {
    const base = checkPreconditions(fakeSigrid(), { src: fakeSrc() }).P4;
    assert.equal(base.ok, false);
    assert.match(base.why, /the largest is --text-base \(1rem\)/);
    const bare = checkPreconditions(fakeSigrid({ steps: '  --text-2xl: 28px;\n' }), { src: fakeSrc() }).P4;
    assert.equal(bare.ok, false);
    assert.match(bare.why, /--text-2xl \(28px\) has no --text-2xl--line-height/);
    const small = checkPreconditions(fakeSigrid({ steps: '  --text-lg: 1.25rem;\n  --text-lg--line-height: 1.4;\n' }), { src: fakeSrc() }).P4;
    assert.equal(small.ok, false);
  });

  test('P5, before Task 19: lists the icons drawn today whose name is not in ICON_NAMES', () => {
    const result = checkPreconditions(fakeSigrid({ icons: ['settings', 'inbox'] }), { src: fakeSrc({ drawn: ['settings', 'inbox', 'trash', 'replyAll'] }) }).P5;
    assert.equal(result.ok, false);
    assert.match(result.why, /not there yet \(Task 19\); of the 4 icons Qamille draws today, 2 have a name that is not in ICON_NAMES: trash, replyAll$/);
  });

  test('P5 holds when every target of ICON_MAP is in ICON_NAMES, and names the ones missing', () => {
    const sigrid = fakeSigrid({ icons: ['settings', 'mail', 'delete'] });
    const map = "import type { IconName } from '@acaprino/sigrid';\nexport const ICON_MAP = {\n  settings: 'settings',\n  trash: 'delete',\n  'list-ordered': 'mail',\n} as const satisfies Record<string, IconName>;\n";
    assert.deepEqual(checkPreconditions(sigrid, { src: fakeSrc({ map }) }).P5, { ok: true, why: 'every one of the 3 names of ICON_MAP is in ICON_NAMES' });
    const short = "export const ICON_MAP: Record<string, string | null> = { inbox: 'inbox', clip: 'attach', print: null };\n";
    const result = checkPreconditions(sigrid, { src: fakeSrc({ map: short }) }).P5;
    assert.equal(result.ok, false);
    assert.equal(result.why, 'ICON_NAMES lacks 3 of the names ICON_MAP needs: inbox, attach, print (no name)');
  });

  test('--require takes a list of P2 to P5', () => {
    assert.equal(requiredNames(['node', 'x.mjs']), null);
    assert.deepEqual(requiredNames(['node', 'x.mjs', '--require', 'P2,p5']), ['P2', 'P5']);
    assert.throws(() => requiredNames(['node', 'x.mjs', '--require']), /needs a list/);
    assert.throws(() => requiredNames(['node', 'x.mjs', '--require', 'P1']), /P1 is withdrawn/);
  });

  test('the synced copy, printed only', (t) => {
    if (!fs.existsSync(path.join(VENDOR, 'provenance.json'))) return t.skip('no synced Sigrid: npm run sigrid:sync');
    const { version } = JSON.parse(fs.readFileSync(path.join(VENDOR, 'provenance.json'), 'utf8'));
    for (const [name, { ok, why }] of Object.entries(checkPreconditions(VENDOR))) t.diagnostic(`Sigrid ${version} ${name} ${ok ? 'ok' : 'not yet'}: ${why}`);
  });
}
