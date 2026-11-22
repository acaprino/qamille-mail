// The owner's rule of 2026-11-22: Qamille relies only on Sigrid (plan docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md,
// Global Constraints). No file of desktop/src imports a package that Sigrid already carries (FORBIDDEN): what Sigrid
// lacks is asked of Sigrid, never written in Qamille. During the move the files of PENDING still do; the list only
// shrinks, and is empty at the end (Task 20, which also checks package.json and the lockfile).
// The imports are read with TypeScript's own parser (as in ui-usage.mjs), so a comment or a string that names a
// package is not an import; the .css files are read for their @import and @plugin rules.
//   node --test tests/only-sigrid.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import test from 'node:test';

const desktop = path.resolve(import.meta.dirname, '..');
const require = createRequire(path.join(desktop, 'package.json'));
const ts = require('typescript');

/** What Sigrid carries: a name ending in «/» is a scope (every package in it), any other one the package and its subpaths. */
export const FORBIDDEN = [
  '@ark-ui/',
  '@base-ui/',
  'motion',
  'lucide-react',
  '@fluentui/',
  'clsx',
  'class-variance-authority',
  'tailwind-merge',
  '@internationalized/date',
  'tw-animate-css',
  // What those carry inside: importing it directly would bring the same library back by another name.
  '@zag-js/',
  'framer-motion',
  'motion-dom',
  'motion-utils',
];

/**
 * The files that still import one, relative to desktop/ (`/**` is a whole folder). At each merge the controller
 * removes the files the task freed; an entry that no longer imports anything forbidden fails the test below.
 */
export const PENDING = [];

// PENDING as the Sigrid plan's Task 0 wrote it. PENDING may lose entries of this list and never gain one: a file
// that starts importing a forbidden package is not added here, it goes through Sigrid (src/ui) instead.
const FIRST_PENDING = [
  'src/ds/**',
  'src/App.tsx',
  'src/main.tsx',
  'src/components/ActivityCenter.tsx',
  'src/components/Composer.tsx',
  'src/components/ComposerDock.tsx',
  'src/components/Contacts.tsx',
  'src/components/MailList.tsx',
  'src/components/UndoStack.tsx',
  'src/components/meetings/MeetingList.tsx',
  'src/components/meetings/MeetingNotice.tsx',
  'src/components/meetings/MeetingProgress.tsx',
  'src/components/meetings/MeetingRecordingIndicator.tsx',
  'src/components/meetings/MeetingRowItem.tsx',
  'src/reminder/Reminder.tsx',
];

/** The FORBIDDEN entry a module specifier falls under, or null. */
export function forbiddenPackage(specifier) {
  return FORBIDDEN.find((name) => (name.endsWith('/') ? specifier.startsWith(name) : specifier === name || specifier.startsWith(`${name}/`))) ?? null;
}

const CODE = /\.(?:[cm]?[jt]sx?)$/;

/** Every module specifier of a script, with its line: imports, re-exports, type imports, import() and require(). */
function scriptImports(file, source) {
  const kind = file.endsWith('x') ? ts.ScriptKind.TSX : /\.[cm]?js$/.test(file) ? ts.ScriptKind.JS : ts.ScriptKind.TS;
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, kind);
  const found = [];
  const add = (node) => {
    if (node && ts.isStringLiteralLike(node)) found.push({ specifier: node.text, line: tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1 });
  };
  const visit = (node) => {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) add(node.moduleSpecifier);
    else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) add(node.moduleReference.expression);
    else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) add(node.argument.literal);
    else if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(node.expression) && node.expression.text === 'require'))) add(node.arguments[0]);
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return found;
}

/** Every @import, @plugin and @reference of a stylesheet, with its line; comments are left out. */
function styleImports(source) {
  // Comments become spaces of the same length (newlines kept), so the lines stay where they were.
  const text = source.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));
  const found = [];
  for (const match of text.matchAll(/@(?:import|plugin|reference)\s+(?:url\(\s*)?(?:'([^']+)'|"([^"]+)"|([^'")\s;]+))/g)) {
    found.push({ specifier: match[1] ?? match[2] ?? match[3], line: text.slice(0, match.index).split('\n').length });
  }
  return found;
}

/** The files of `root`/src that Qamille writes. src/vendor holds ArtCraft's generated engines, not Qamille's code. */
function sources(root) {
  const walk = (dir) =>
    fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) return entry.name === 'vendor' || entry.name === 'node_modules' ? [] : walk(full);
      return CODE.test(entry.name) || entry.name.endsWith('.css') ? [full] : [];
    });
  return walk(path.join(root, 'src'));
}

const relative = (root, file) => path.relative(root, file).replaceAll('\\', '/');
const matches = (entry, rel) => (entry.endsWith('/**') ? rel.startsWith(entry.slice(0, -2)) : rel === entry);

/** Every forbidden import under `root`/src: { file (relative to root), line, specifier, package }. */
export function forbiddenImports(root = desktop) {
  return sources(root).flatMap((file) => {
    const source = fs.readFileSync(file, 'utf8');
    const found = file.endsWith('.css') ? styleImports(source) : scriptImports(file, source);
    return found.flatMap(({ specifier, line }) => {
      const name = forbiddenPackage(specifier);
      return name ? [{ file: relative(root, file), line, specifier, package: name }] : [];
    });
  });
}

/** The forbidden imports of the files outside `pending`. */
export function violations(root = desktop, pending = PENDING) {
  return forbiddenImports(root).filter((hit) => !pending.some((entry) => matches(entry, hit.file)));
}

/** The entries of `pending` that match no file importing a forbidden package any more. */
export function staleEntries(root = desktop, pending = PENDING) {
  const hits = forbiddenImports(root);
  return pending.filter((entry) => !hits.some((hit) => matches(entry, hit.file)));
}

const describe = (hits) => hits.map((hit) => `${hit.file}:${hit.line}  imports ${hit.specifier}`).join('\n');

test('no file outside PENDING imports a package Sigrid already carries', () => {
  const found = violations();
  assert.deepEqual(found, [], `Qamille relies only on Sigrid (go through src/ui, and ask Sigrid for what it lacks):\n${describe(found)}`);
});

test('every PENDING entry still imports something Sigrid carries', () => {
  const stale = staleEntries();
  assert.deepEqual(stale, [], `remove from PENDING what no longer imports a forbidden package: ${stale.join(', ')}`);
});

test('PENDING only shrinks', () => {
  assert.equal(new Set(PENDING).size, PENDING.length, 'PENDING has an entry twice');
  const added = PENDING.filter((entry) => !FIRST_PENDING.includes(entry));
  assert.deepEqual(added, [], `PENDING never grows: ${added.join(', ')}`);
});

// The checks on a fake desktop/src, under desktop/.cache (ignored by git), not in the system's temp folder.
const cache = path.join(desktop, '.cache');
fs.mkdirSync(cache, { recursive: true });
const scratch = fs.mkdtempSync(path.join(cache, 'test-only-sigrid-'));
test.after(() => fs.rmSync(scratch, { recursive: true, force: true }));
let counter = 0;
function fakeDesktop(files) {
  const root = path.join(scratch, `desktop-${++counter}`);
  for (const [name, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), text);
  }
  return root;
}

test('a file outside PENDING that imports motion fails, a file of PENDING does not', () => {
  const root = fakeDesktop({
    'src/App.tsx': "import { motion } from 'motion/react';\nexport const a = motion;\n",
    'src/components/Fake.tsx': "import { useState } from 'react';\nimport { motion } from 'motion/react';\nexport const b = [useState, motion];\n",
  });
  assert.deepEqual(violations(root, ['src/App.tsx']), [{ file: 'src/components/Fake.tsx', line: 2, specifier: 'motion/react', package: 'motion' }]);
  assert.deepEqual(violations(root, ['src/App.tsx', 'src/components/Fake.tsx']), []);
});

test('every way of importing is seen, and nothing that only names a package', () => {
  const root = fakeDesktop({
    'src/a.ts': [
      "import type { Variants } from 'motion';",
      "export { Dialog } from '@base-ui/react/dialog';",
      "const later = () => import('lucide-react');",
      "const old = require('clsx');",
      "import merge = require('tailwind-merge');",
      "type Day = import('@internationalized/date').CalendarDate;",
      "// import { motion } from 'motion/react';",
      "const words = 'motion clsx @ark-ui/react';",
      "import { near } from 'motion-plus';",
      "import { also } from 'clsx-like';",
      "export const all = [later, old, merge, words, near, also];",
    ].join('\n'),
    'src/b.tsx': "import { Dialog } from '@ark-ui/react/dialog';\nimport { Icon } from '@fluentui/react-icons';\nexport const c = <Dialog.Root>{Icon}</Dialog.Root>;\n",
    'src/c.css': "/* @import 'tw-animate-css'; */\n@import 'tailwindcss/theme.css' layer(theme);\n@import \"tw-animate-css\";\n@plugin 'class-variance-authority';\n",
    'src/vendor/artcraft/engine.js': "import x from 'motion';\n",
  });
  const seen = violations(root, []).map((hit) => `${hit.file}:${hit.line} ${hit.specifier}`);
  assert.deepEqual(seen, [
    'src/a.ts:1 motion',
    'src/a.ts:2 @base-ui/react/dialog',
    'src/a.ts:3 lucide-react',
    'src/a.ts:4 clsx',
    'src/a.ts:5 tailwind-merge',
    'src/a.ts:6 @internationalized/date',
    'src/b.tsx:1 @ark-ui/react/dialog',
    'src/b.tsx:2 @fluentui/react-icons',
    'src/c.css:3 tw-animate-css',
    'src/c.css:4 class-variance-authority',
  ]);
});

test('an entry of PENDING whose files import nothing forbidden is stale', () => {
  const root = fakeDesktop({
    'src/ds/primitives/Switch.tsx': "import { Switch } from '@ark-ui/react/switch';\nexport default Switch;\n",
    'src/ds/primitives/Icon.tsx': 'export default function Icon() { return null; }\n',
    'src/App.tsx': "import { useState } from 'react';\nexport const a = useState;\n",
  });
  assert.deepEqual(staleEntries(root, ['src/ds/**', 'src/App.tsx', 'src/main.tsx']), ['src/App.tsx', 'src/main.tsx']);
});

test('package.json and the lockfile carry none of what Sigrid carries', () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(desktop, 'package.json'), 'utf8'));
  const declared = Object.keys({ ...manifest.dependencies, ...manifest.devDependencies, ...manifest.optionalDependencies });
  const locked = Object.keys(JSON.parse(fs.readFileSync(path.join(desktop, 'package-lock.json'), 'utf8')).packages ?? {})
    .filter((name) => name.startsWith('node_modules/'))
    .map((name) => name.slice('node_modules/'.length).replace(/^.*node_modules\//, ''));
  const found = [...declared, ...locked].filter((name) => forbiddenPackage(name));
  assert.deepEqual([...new Set(found)], [], 'a package that Sigrid already carries is in package.json or the lockfile');
});
