// The visual system has one home (ADR-0011). Outside desktop/src/ds, markup goes through the
// primitives and never writes their classes or a <dialog> by hand (INV-002) nor a native <select>,
// checkbox or hand-made tablist (INV-003), and never imports @ark-ui (INV-007: only the module does);
// a command menu is a Menu, never a Popover whose children are all buttons; inside the module,
// nothing reaches into the app (INV-001); only src/ui and src/theme import Sigrid (INV-009). The .tsx files are read with TypeScript's own parser, so a
// comment or a string outside a className that mentions a class is not a violation.
//   node tests/ds-usage.mjs
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const desktop = path.resolve(import.meta.dirname, '..');
const require = createRequire(path.join(desktop, 'package.json'));
const ts = require('typescript');

const root = process.env.DS_USAGE_ROOT ? path.resolve(process.env.DS_USAGE_ROOT) : path.join(desktop, 'src');
const DS = path.join(root, 'ds');
const INSTEAD = {
  primary: 'Button variant="primary"',
  secondary: 'Button variant="secondary"',
  'icon-button': 'Button variant="icon"',
  'text-button': 'Button variant="text"',
  field: 'Field',
  popover: 'Popover',
  'side-panel': 'SidePanel',
  'side-panel-layer': 'SidePanel',
  modal: 'Modal',
};
// INV-003 exceptions: a native control that stays outside the primitives, with the reason.
// The Calendar rail rows are <button role="checkbox"> (not a native checkbox, not flagged).
const ALLOWED = [
  { file: 'components/MailList.tsx', control: 'checkbox', class: 'mail-checkbox', reason: 'rows stay plain markup (spec 7.6)' },
];
const used = new Set();
const words = (dir, keep) =>
  fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'vendor' ? [] : words(full, keep);
    return keep.test(entry.name) ? [full] : [];
  });
const problems = [];

/** Every string literal inside an expression (className={cn('a', on && 'b')} gives a, b). */
function strings(node, out = []) {
  if (!node) return out;
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) out.push(node.text);
  else if (ts.isTemplateExpression(node)) {
    out.push(node.head.text);
    for (const span of node.templateSpans) {
      strings(span.expression, out);
      out.push(span.literal.text);
    }
  } else ts.forEachChild(node, (child) => void strings(child, out));
  return out;
}

/**
 * What a JSX child list is made of, one word per child: 'button' for a <button> or a Button, 'divider' for a
 * span.popover-divider, the tag name for any other element, 'other' for text or an expression it cannot
 * follow. Whitespace and {' '} count for nothing; `on && <x/>`, `on ? <x/> : <y/>`, a fragment and
 * `list.map((i) => <x/>)` count for the elements they give.
 */
function childKinds(tree, nodes, out = []) {
  for (const node of nodes) kindsOf(tree, node, out);
  return out;
}
function kindsOf(tree, node, out) {
  if (!node) return out;
  if (ts.isParenthesizedExpression(node)) return kindsOf(tree, node.expression, out);
  if (ts.isJsxText(node)) {
    if (node.text.trim()) out.push('other');
    return out;
  }
  if (ts.isJsxExpression(node)) {
    const inner = node.expression;
    if (!inner || ((ts.isStringLiteral(inner) || ts.isNoSubstitutionTemplateLiteral(inner)) && !inner.text.trim())) return out;
    return kindsOf(tree, inner, out);
  }
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) return kindsOf(tree, node.right, out);
  if (ts.isConditionalExpression(node)) {
    kindsOf(tree, node.whenTrue, out);
    return kindsOf(tree, node.whenFalse, out);
  }
  if (ts.isJsxFragment(node)) return childKinds(tree, node.children, out);
  if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'map') {
    const callback = node.arguments[0];
    if (callback && (ts.isArrowFunction(callback) || ts.isFunctionExpression(callback)) && !ts.isBlock(callback.body)) return kindsOf(tree, callback.body, out);
    out.push('other');
    return out;
  }
  const element = ts.isJsxElement(node) ? node.openingElement : ts.isJsxSelfClosingElement(node) ? node : null;
  if (!element) {
    out.push('other');
    return out;
  }
  const name = element.tagName.getText(tree);
  if (name === 'button' || name === 'Button') out.push('button');
  else if (name === 'span' && element.attributes.properties.some((a) => ts.isJsxAttribute(a) && a.name.getText(tree) === 'className' && a.initializer && strings(a.initializer).join(' ').split(/\s+/).includes('popover-divider')))
    out.push('divider');
  else out.push(name);
  return out;
}

// INV-002 and INV-003: markup outside the module.
for (const file of words(root, /\.tsx$/)) {
  if (file.startsWith(`${DS}${path.sep}`)) continue;
  // src/ui is where the primitives live from now on (the Sigrid plan): its adapters and its own components write the markup.
  if (file.startsWith(`${path.join(root, 'ui')}${path.sep}`)) continue;
  const rel = path.relative(root, file).replaceAll('\\', '/');
  const source = fs.readFileSync(file, 'utf8');
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const visit = (node) => {
    // A command menu is a Menu (role menu, arrows, Home and End, typeahead), not a Popover of buttons:
    // Popover stays for what is not a menu (a filter, a picker, a small form).
    if (ts.isJsxElement(node) && node.openingElement.tagName.getText(tree) === 'Popover') {
      const kinds = childKinds(tree, node.children).filter((kind) => kind !== 'divider');
      if (kinds.length && kinds.every((kind) => kind === 'button')) {
        const line = tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1;
        problems.push(`${rel}:${line}  <Popover> whose children are all buttons is a command menu: use Menu`);
      }
    }
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const name = node.tagName.getText(tree);
      if (/^[a-z]/.test(name)) {
        const line = tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1;
        const attribute = (key) => node.attributes.properties.find((a) => ts.isJsxAttribute(a) && a.name.getText(tree) === key);
        const text = (key) => {
          const a = attribute(key);
          if (!a?.initializer) return '';
          return strings(a.initializer).join(' ');
        };
        if (name === 'dialog') problems.push(`${rel}:${line}  <dialog>: use Modal or SidePanel`);
        const control =
          name === 'select' ? 'select' : name === 'input' && text('type') === 'checkbox' ? 'checkbox' : text('role') === 'tablist' ? 'tablist' : '';
        if (control) {
          const allowed = ALLOWED.find((entry) => entry.file === rel && entry.control === control && text('className').split(/\s+/).includes(entry.class));
          if (allowed) used.add(allowed);
          else {
            const instead = { select: 'Select', checkbox: 'Checkbox or Switch', tablist: 'Tabs' }[control];
            problems.push(`${rel}:${line}  native ${control} on <${name}>: use ${instead} (INV-003)`);
          }
        }
        const classes = text('className').split(/\s+/).filter(Boolean);
        for (const [cls, instead] of Object.entries(INSTEAD)) {
          if (classes.includes(cls)) problems.push(`${rel}:${line}  class "${cls}" on <${name}>: use ${instead}`);
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
}

for (const entry of ALLOWED) {
  if (!used.has(entry)) problems.push(`${entry.file}  INV-003 allow-list entry "${entry.class}" matches nothing: remove it`);
}

// INV-007: only the module talks to Ark UI.
for (const file of words(root, /\.(tsx?|js)$/)) {
  if (file.startsWith(`${DS}${path.sep}`)) continue;
  const rel = path.relative(root, file).replaceAll('\\', '/');
  fs.readFileSync(file, 'utf8').split('\n').forEach((text, index) => {
    if (/(?:from\s+|import\s*\(?\s*)['"]@ark-ui\//.test(text)) problems.push(`${rel}:${index + 1}  imports @ark-ui: go through a primitive in src/ds (INV-007)`);
  });
}

// INV-009 (draft for ADR-0011 v4, plan docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md): only src/ui and
// src/theme import Sigrid, by its name or by a path into vendor/sigrid; the rest of the app goes through them. The
// imports are read with TypeScript's parser: a comment or a string that names Sigrid is not an import.
const SIGRID = '@acaprino/sigrid';
const SIGRID_HOMES = ['ui/', 'theme/'];

/** Every module specifier of a script, with its line: imports, re-exports, type imports, import() and require(). */
function moduleSpecifiers(rel, source) {
  const kind = rel.endsWith('x') ? ts.ScriptKind.TSX : /\.[cm]?js$/.test(rel) ? ts.ScriptKind.JS : ts.ScriptKind.TS;
  const tree = ts.createSourceFile(rel, source, ts.ScriptTarget.Latest, true, kind);
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

/** The INV-009 problems of `files` ({ rel, source }, rel relative to the src folder `base`). */
function sigridOutsideItsHomes(files, base) {
  const vendor = path.join(base, '..', 'vendor', 'sigrid');
  const inVendor = (specifier, rel) => {
    if (!specifier.startsWith('.')) return false;
    const target = path.resolve(base, path.dirname(rel), specifier);
    return target === vendor || target.startsWith(`${vendor}${path.sep}`);
  };
  return files.flatMap(({ rel, source }) =>
    SIGRID_HOMES.some((home) => rel.startsWith(home))
      ? []
      : moduleSpecifiers(rel, source)
          .filter(({ specifier }) => specifier === SIGRID || specifier.startsWith(`${SIGRID}/`) || inVendor(specifier, rel))
          .map(({ specifier, line }) => `${rel}:${line}  imports ${specifier}: only src/ui and src/theme import Sigrid (INV-009)`),
  );
}

// The check proves itself on files made up here, before it is trusted with the app's: if it stops catching one, the
// script fails rather than passing on nothing.
{
  const base = path.join(desktop, 'inv-009-fixture', 'src');
  const fixture = [
    { rel: 'components/Bad.tsx', source: "import { Button } from '@acaprino/sigrid';\nexport default Button;\n" },
    { rel: 'components/Lazy.tsx', source: "export const load = () => import('@acaprino/sigrid');\n" },
    { rel: 'lib/sneaky.ts', source: "\nexport * from '../../vendor/sigrid/dist/index.js';\n" },
    { rel: 'ds/primitives/Kind.ts', source: "export type Kind = import('@acaprino/sigrid').ButtonProps;\n" },
    { rel: 'components/Note.tsx', source: "// '@acaprino/sigrid' is reached through src/ui.\nexport const name = '@acaprino/sigrid';\n" },
    { rel: 'ui/Good.tsx', source: "export { WordsProvider } from '@acaprino/sigrid';\n" },
    { rel: 'theme/good.ts', source: "import type { ThemeDef } from '@acaprino/sigrid';\nexport type T = ThemeDef;\n" },
  ];
  const expected = ['components/Bad.tsx:1', 'components/Lazy.tsx:1', 'lib/sneaky.ts:2', 'ds/primitives/Kind.ts:1'];
  const caught = sigridOutsideItsHomes(fixture, base).map((problem) => problem.split('  ')[0]);
  if (JSON.stringify(caught) !== JSON.stringify(expected)) problems.push(`INV-009 check is broken: on its fixture it reports ${JSON.stringify(caught)}, not ${JSON.stringify(expected)}`);
}

problems.push(
  ...sigridOutsideItsHomes(
    words(root, /\.(?:[cm]?[jt]sx?)$/).map((file) => ({ rel: path.relative(root, file).replaceAll('\\', '/'), source: fs.readFileSync(file, 'utf8') })),
    root,
  ),
);

// INV-001: the module depends on nothing of the app.
for (const file of words(DS, /\.(tsx?|js|css)$/)) {
  const rel = path.relative(root, file).replaceAll('\\', '/');
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  lines.forEach((text, index) => {
    const target = /(?:from\s+|import\s+|@import\s+url\()['"]([^'"]+)['"]/.exec(text)?.[1];
    if (!target) return;
    if (target.startsWith('@tauri-apps')) problems.push(`${rel}:${index + 1}  imports ${target}: the module stays free of the shell`);
    if (target.startsWith('.')) {
      const resolved = path.resolve(path.dirname(file), target);
      // The one bridge of the move to Sigrid (the Sigrid plan, Tasks 2 to 20): the Tailwind entry takes Sigrid's theme and
      // fonts and Qamille's parameters for them. It goes with src/ds at the end.
      const bridge =
        rel === 'ds/styles/tailwind.css' &&
        [path.join(root, 'theme'), path.join(root, '..', 'vendor', 'sigrid')].some((home) => resolved.startsWith(`${home}${path.sep}`));
      // The primitives drawn by Sigrid are re-exported from src/ui until src/ds goes (Task 20).
      const reexport = rel.startsWith('ds/primitives/') && resolved.startsWith(`${path.join(root, 'ui')}${path.sep}`);
      if (!bridge && !reexport && resolved !== DS && !resolved.startsWith(`${DS}${path.sep}`)) problems.push(`${rel}:${index + 1}  imports ${target}: the module does not reach into the app`);
    }
  });
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} uses outside the visual system module (ADR-0011)`);
  process.exit(1);
}
console.log('PASS markup goes through the primitives, the module stands alone, and only src/ui and src/theme import Sigrid');
