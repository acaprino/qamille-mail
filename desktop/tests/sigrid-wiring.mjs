// How Qamille reaches Sigrid (plan docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md, Task 1). The copy is the
// one `npm run sigrid:sync` puts in desktop/vendor/sigrid, reached through a Vite alias and a TypeScript path with no
// npm entry: an entry would write Sigrid's version into the lockfile (a pin) and install what Sigrid bundles. The sync
// runs before dev and build and in CI, and the release build goes through `npm run build`. Without the copy the build
// stops with a message, so a build never goes ahead without Sigrid.
//   node --test tests/sigrid-wiring.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import test from 'node:test';

const desktop = path.resolve(import.meta.dirname, '..');
const repo = path.resolve(desktop, '..');
const require = createRequire(path.join(desktop, 'package.json'));
const ts = require('typescript');

const SIGRID = '@acaprino/sigrid';
const vendor = path.join(desktop, 'vendor', 'sigrid');
const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));
const read = (file) => fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
// Vite answers with forward slashes, TypeScript too; Windows paths compare without case.
const samePath = (a, b) => {
  const norm = (p) => path.resolve(p).replaceAll('\\', '/');
  return process.platform === 'win32' ? norm(a).toLowerCase() === norm(b).toLowerCase() : norm(a) === norm(b);
};

/** The files of Qamille's own code and configuration, where no Sigrid commit may be written. */
function ownFiles() {
  const skip = new Set(['node_modules', 'vendor', '.cache', 'dist', 'target', 'gen']);
  const walk = (dir) =>
    fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) return skip.has(entry.name) ? [] : walk(full);
      return /\.(?:[cm]?[jt]sx?|json|css|html|ya?ml)$/.test(entry.name) ? [full] : [];
    });
  return [
    ...['src', 'tests', 'scripts'].flatMap((dir) => walk(path.join(desktop, dir))),
    ...['package.json', 'package-lock.json', 'tsconfig.json', 'vite.config.ts'].map((name) => path.join(desktop, name)),
    ...walk(path.join(repo, '.github')),
  ];
}

test('no npm entry pins Sigrid', () => {
  const pkg = readJson(path.join(desktop, 'package.json'));
  for (const field of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies', 'overrides']) {
    assert.ok(!Object.hasOwn(pkg[field] ?? {}, SIGRID), `package.json ${field} names ${SIGRID}: Qamille reaches the synced copy through an alias, never through npm`);
  }
  for (const field of ['bundleDependencies', 'bundledDependencies']) assert.ok(!(pkg[field] ?? []).includes(SIGRID), `package.json ${field} names ${SIGRID}`);
  assert.ok(!read(path.join(desktop, 'package-lock.json')).includes(SIGRID), `package-lock.json names ${SIGRID}: its version would be a pin`);
  // Nor the commit of the synced copy: who needs it reads vendor/sigrid/provenance.json.
  const provenance = path.join(vendor, 'provenance.json');
  if (!fs.existsSync(provenance)) return;
  const { commit } = readJson(provenance);
  const written = new RegExp(`(?<![0-9a-f])(?:${commit}|${commit.slice(0, 7)})(?![0-9a-f])`, 'i');
  const found = ownFiles().filter((file) => written.test(fs.readFileSync(file, 'utf8')));
  assert.deepEqual(found.map((file) => path.relative(repo, file)), [], `Sigrid's commit ${commit.slice(0, 7)} is written in Qamille`);
});

test('Vite and TypeScript reach the synced copy', async () => {
  // The configuration as Vite resolves it (the build: tests/build-check.mjs builds through the same alias).
  const { resolveConfig } = await import('vite');
  const config = await resolveConfig({ root: desktop, configFile: path.join(desktop, 'vite.config.ts'), logLevel: 'silent' }, 'build', 'production', 'production');
  // The alias is on the exact name: the entry of dist, and nothing else of the package.
  const alias = config.resolve.alias.find((entry) => (entry.find instanceof RegExp ? entry.find.test(SIGRID) : entry.find === SIGRID));
  assert.ok(alias, `vite.config.ts has no resolve.alias for ${SIGRID}`);
  assert.ok(alias.find instanceof RegExp && !alias.find.test(`${SIGRID}/src/styles/theme.css`) && !alias.find.test(`x${SIGRID}`), 'the alias must match the exact name only');
  assert.ok(samePath(alias.replacement, path.join(vendor, 'dist', 'index.js')), `the alias leads to ${alias.replacement}, not vendor/sigrid/dist/index.js`);
  assert.ok(fs.existsSync(alias.replacement), `${alias.replacement} is missing: npm run sigrid:sync`);
  const importer = path.join(desktop, 'src', 'main.tsx');
  for (const name of ['react', 'react-dom']) assert.ok(config.resolve.dedupe?.includes(name), `resolve.dedupe lacks ${name}: Sigrid must run on Qamille's own copy`);
  const parsed = ts.getParsedCommandLineOfConfigFile(path.join(desktop, 'tsconfig.json'), {}, {
    ...ts.sys,
    onUnRecoverableConfigFileDiagnostic: (diagnostic) => {
      throw new Error(ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n'));
    },
  });
  const { resolvedModule } = ts.resolveModuleName(SIGRID, importer, parsed.options, ts.sys);
  assert.ok(resolvedModule, `TypeScript does not resolve ${SIGRID}`);
  assert.ok(samePath(resolvedModule.resolvedFileName, path.join(vendor, 'dist', 'index.d.ts')), `TypeScript resolves ${SIGRID} to ${resolvedModule.resolvedFileName}`);
});

test('predev and prebuild sync Sigrid after ArtCraft', () => {
  const { scripts } = readJson(path.join(desktop, 'package.json'));
  assert.equal(scripts['sigrid:sync'], 'node scripts/sync-sigrid.mjs');
  for (const hook of ['predev', 'prebuild']) assert.equal(scripts[hook], 'npm run artcraft:sync && npm run sigrid:sync', hook);
  // The type check, the visual-system checks and Vite all run after the hook, on the copy it left.
  assert.match(scripts.build, /^npm run check && /);
});

/** The steps of `job` in a workflow, each as its text with the step's own indentation taken off. */
function jobSteps(workflow, job) {
  const lines = workflow.split('\n');
  const start = lines.indexOf(`  ${job}:`);
  assert.ok(start >= 0, `no job ${job}`);
  const next = lines.findIndex((line, index) => index > start && /^ {2}\S/.test(line));
  const body = lines.slice(start + 1, next < 0 ? undefined : next);
  const steps = [];
  for (const line of body.slice(body.findIndex((l) => /^ {4}steps:\s*$/.test(l)) + 1)) {
    if (/^\s*#/.test(line)) continue;
    if (/^ {6}- /.test(line)) steps.push(line.slice(8));
    else if (steps.length && (/^ {8}/.test(line) || !line.trim())) steps[steps.length - 1] += `\n${line.slice(8)}`;
    else if (line.trim()) break;
  }
  return steps;
}

test('CI syncs Sigrid before the type check and checks the only-Sigrid rule', () => {
  const steps = jobSteps(read(path.join(repo, '.github', 'workflows', 'ci.yml')), 'desktop');
  const at = (command) => {
    const index = steps.findIndex((step) => new RegExp(`^run: ${command.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*$`, 'm').test(step));
    assert.ok(index >= 0, `the desktop job does not run ${command}`);
    return index;
  };
  const order = ['npm ci', 'npm run artcraft:sync', 'npm run sigrid:sync', 'npm run check', 'node --test tests/only-sigrid.mjs'].map(at);
  assert.deepEqual([...order].sort((a, b) => a - b), order, 'npm ci, artcraft:sync, sigrid:sync, check, only-sigrid: in this order');
  assert.ok(order[2] < at('npm run test:ds') && order[2] < at('npm run test:browser -- onboarding-professional'), 'Sigrid is synced before every check that builds or serves the app');
  // GitHub's API answers an anonymous runner with a low rate limit: the job's token lifts it.
  assert.match(steps[order[2]], /^env:\n {2}GH_TOKEN: \$\{\{ github\.token \}\}$/m, 'the sigrid:sync step needs GH_TOKEN');
});

test('the release build goes through npm run build, so it syncs Sigrid or stops', () => {
  // The command release.yml runs ("Prepare and build"), as release.mjs reads it from .github/release.json.
  const run = spawnSync(process.execPath, ['.github/scripts/release.mjs', 'config', 'build'], { cwd: repo, encoding: 'utf8' });
  assert.equal(run.status, 0, run.stderr);
  assert.match(run.stdout.trim(), /^build-release\.bat\b/, 'the release builds with build-release.bat');
  const bat = read(path.join(repo, 'build-release.bat'));
  assert.match(bat, /^set "NPM_DIR=desktop"$/m);
  const build = bat.slice(bat.indexOf('\n:build\n'));
  assert.match(build, /pushd "desktop" \|\| goto :sub_fail\ncall npm run build\nset "RC=%ERRORLEVEL%"\npopd\nif not "%RC%"=="0" goto :sub_fail\n/, 'build-release.bat stops when npm run build (and so its prebuild) fails');
  // The release job carries a token for GitHub's API, which the sync uses.
  assert.match(jobSteps(read(path.join(repo, '.github', 'workflows', 'release.yml')), 'release').join('\n'), /node \.github\/scripts\/release\.mjs config build/);
  assert.match(read(path.join(repo, '.github', 'workflows', 'release.yml')), /\n {2}release:\n(?: {4}.*\n)*? {4}env:\n {6}GH_TOKEN: /, 'the release job needs GH_TOKEN');
});

test('the build stops with a message when the copy is missing', () => {
  // Qamille's vite.config.ts in a desktop folder of its own, with no vendor/sigrid. It sits under desktop/.cache
  // (ignored by git), so its imports still resolve from desktop/node_modules.
  fs.mkdirSync(path.join(desktop, '.cache'), { recursive: true });
  const fake = fs.mkdtempSync(path.join(desktop, '.cache', 'test-sigrid-wiring-'));
  try {
    fs.copyFileSync(path.join(desktop, 'vite.config.ts'), path.join(fake, 'vite.config.ts'));
    const run = spawnSync(process.execPath, [path.join(desktop, 'node_modules', 'vite', 'bin', 'vite.js'), 'build', '--config', path.join(fake, 'vite.config.ts')], {
      cwd: fake,
      encoding: 'utf8',
      timeout: 120_000,
      windowsHide: true,
    });
    const output = `${run.stdout ?? ''}${run.stderr ?? ''}`;
    assert.notEqual(run.status, 0, output);
    assert.ok(output.includes('Sigrid manca: `npm run sigrid:sync` in `desktop/`'), output);
    assert.equal(fs.existsSync(path.join(fake, 'dist')), false, 'nothing is built');
  } finally {
    fs.rmSync(fake, { recursive: true, force: true });
  }
});
