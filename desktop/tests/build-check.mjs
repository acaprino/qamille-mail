// The production build, checked for what the Sigrid plan promises (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md):
//   - one copy of React: react and react-dom come from one node_modules folder each, and Sigrid bundles neither;
//   - the copies of Zag (the machines under Ark UI) and of Motion, counted and printed: Qamille's own, from its
//     node_modules, and Sigrid's, bundled inside vendor/sigrid/dist. With --strict there must be one of each (Zag from
//     Task 16 and Motion from Task 18, when Qamille no longer has its own);
//   - no `cursor: pointer`, `not-allowed` or `progress` in the CSS (docs/aspetto-nativo.md, rule 3);
//   - Sigrid's licence, third-party notices and provenance travel with the build (sigrid-licenses/).
// The build is Vite's own, with vite.config.ts, written to a scratch folder under desktop/.cache (ignored by git) and
// removed at the end. It does not run prebuild: it builds on the copy of Sigrid already in vendor/sigrid.
//   node tests/build-check.mjs [--strict]
import fs from 'node:fs';
import path from 'node:path';

const desktop = path.resolve(import.meta.dirname, '..');
const strict = process.argv.includes('--strict');
const vendor = path.join(desktop, 'vendor', 'sigrid').replaceAll('\\', '/');
const slash = (file) => file.replaceAll('\\', '/');
const MOTION = new Set(['motion', 'motion-dom', 'motion-utils', 'framer-motion']);

// A family of packages, and how to tell that Sigrid's bundle holds a copy of it. Sigrid's dist is not minified, and
// every library it bundles starts with a `// node_modules/<package>/` comment, which the build drops; the rendered code
// keeps the names, so a copy is there when the name of one of its central declarations is (rolldown may add $1).
const FAMILIES = {
  React: { owns: (name) => name === 'react' || name === 'react-dom', bundled: /^\/\/ node_modules\/react(?:-dom)?\//m, marker: null },
  Zag: { owns: (name) => name.startsWith('@zag-js/'), bundled: /^\/\/ node_modules\/@zag-js\//m, marker: /\bfunction useMachine(?:\$\d+)?\s*\(/ },
  Motion: { owns: (name) => MOTION.has(name), bundled: /^\/\/ node_modules\/(?:motion|motion-dom|framer-motion)\//m, marker: /\bclass MotionValue(?:\$\d+)?\b|\bMotionValue(?:\$\d+)?\s*=\s*class\b/ },
};

/** The node_modules folder and package of a module id, or null for the app's own code. The last node_modules wins. */
function packageOf(id) {
  const match = /^(.*\/node_modules\/)((?:@[^/]+\/)?[^/]+)\//.exec(slash(id).replace(/\?.*$/, ''));
  return match && { folder: match[1], name: match[2] };
}

const problems = [];
fs.mkdirSync(path.join(desktop, '.cache'), { recursive: true });
const outDir = fs.mkdtempSync(path.join(desktop, '.cache', 'build-check-'));
try {
  const { build } = await import('vite');
  const result = await build({
    root: desktop,
    configFile: path.join(desktop, 'vite.config.ts'),
    logLevel: 'error',
    build: { outDir, emptyOutDir: true, reportCompressedSize: false },
  });
  const chunks = (Array.isArray(result) ? result : [result]).flatMap((output) => output.output).filter((item) => item.type === 'chunk');

  // Every module that left code in the build, once, with its rendered code.
  const rendered = new Map();
  for (const chunk of chunks) {
    for (const [id, module] of Object.entries(chunk.modules)) {
      if (id.startsWith('\0') || !module.renderedLength) continue;
      rendered.set(slash(id).replace(/\?.*$/, ''), { chunk: chunk.fileName, length: module.renderedLength, code: module.code ?? '' });
    }
  }
  const sigridModules = [...rendered].filter(([id]) => id.toLowerCase().startsWith(`${vendor.toLowerCase()}/dist/`));

  const copies = {};
  for (const [family, { owns, bundled, marker }] of Object.entries(FAMILIES)) {
    const own = new Map();
    for (const [id] of rendered) {
      const pkg = packageOf(id);
      if (!pkg || !owns(pkg.name)) continue;
      const roots = own.get(pkg.name) ?? new Set();
      roots.add(pkg.folder);
      own.set(pkg.name, roots);
    }
    const folders = new Set([...own.values()].flatMap((roots) => [...roots]));
    const found = [...folders].map((folder) => path.relative(desktop, folder).replaceAll('\\', '/') || folder);
    for (const [id, { code }] of sigridModules) {
      const source = fs.readFileSync(id, 'utf8');
      if (!bundled.test(source)) continue;
      const file = path.relative(desktop, id).replaceAll('\\', '/');
      if (!marker) {
        problems.push(`${file} bundles its own ${family}: Sigrid must take it from Qamille (a peer), or there are two`);
        continue;
      }
      // The marker must name the library in Sigrid's own file, or the count below would miss it without a word.
      if (!marker.test(source)) {
        problems.push(`${file} bundles ${family}, but not the declaration build-check looks for (${marker}): update FAMILIES`);
        continue;
      }
      if (marker.test(code)) found.push(`Sigrid's bundle (${file})`);
    }
    copies[family] = found;
    for (const [name, roots] of own) {
      if (family === 'React' && roots.size > 1) problems.push(`${name} comes from ${roots.size} folders: ${[...roots].join(', ')}`);
    }
    console.log(`${family}: ${found.length} ${found.length === 1 ? 'copy' : 'copies'}${found.length ? ` (${found.join('; ')})` : ''}`);
  }
  if (copies.React.length !== 1) problems.push(`React: ${copies.React.length} copies, not one`);
  if (strict) {
    for (const family of ['Zag', 'Motion']) if (copies[family].length !== 1) problems.push(`${family}: ${copies[family].length} copies, not one (--strict)`);
  }
  const sigridLength = sigridModules.reduce((sum, [, { length }]) => sum + length, 0);
  const sigridChunks = [...new Set(sigridModules.map(([, { chunk }]) => chunk))];
  const kB = (bytes) => `${Math.round(bytes / 1024)} kB`;
  console.log(`Sigrid: ${kB(sigridLength)} of code before minification, in ${sigridChunks.map((name) => `${name} (${kB(fs.statSync(path.join(outDir, name)).size)})`).join(', ') || 'no chunk'}`);

  // The cursors of a web page, which a Windows app does not show on its controls.
  const cssFiles = fs.readdirSync(path.join(outDir, 'assets')).filter((name) => name.endsWith('.css'));
  for (const name of cssFiles) {
    const css = fs.readFileSync(path.join(outDir, 'assets', name), 'utf8');
    for (const match of css.matchAll(/cursor\s*:\s*(pointer|not-allowed|progress)\b/g)) {
      problems.push(`assets/${name}: cursor: ${match[1]} near «${css.slice(Math.max(0, match.index - 60), match.index + 30)}»`);
    }
  }
  console.log(`CSS: ${cssFiles.length} files read for cursors`);

  // The notices of Sigrid, as sigrid:sync left them in public/sigrid-licenses.
  for (const name of ['LICENSE', 'THIRD_PARTY.md', 'provenance.json']) {
    const built = path.join(outDir, 'sigrid-licenses', name);
    if (!fs.existsSync(built)) problems.push(`sigrid-licenses/${name} is not in the build: npm run sigrid:sync`);
    else if (!fs.readFileSync(built).equals(fs.readFileSync(path.join(vendor, name)))) problems.push(`sigrid-licenses/${name} differs from vendor/sigrid/${name}: npm run sigrid:sync`);
  }
} finally {
  fs.rmSync(outDir, { recursive: true, force: true });
}

if (problems.length) {
  console.error(problems.join('\n'));
  console.error(`\nFAIL ${problems.length} ${problems.length === 1 ? 'problem' : 'problems'} in the build`);
  process.exit(1);
}
console.log(`PASS one copy of React${strict ? ', of Zag and of Motion' : ''}, no web cursor in the CSS, Sigrid's notices in the build`);
