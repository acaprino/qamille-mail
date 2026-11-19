#!/usr/bin/env node
// Generates the files of the Qamille design system artifact from the module desktop/src/ds (ADR-0011).
//
//   npm run ds:export                      all the files, into desktop/ds-export/
//   npm run ds:export -- --out <dir>       into another folder (the test uses two temp folders)
//   npm run ds:export -- --only Button,Sidebar
//                                          partial: only the cards of those components, nothing else rewritten
//   npm run ds:export -- --blobs <file> --at <iso date>
//                                          at publishing time: the upload ids that the asset uploads returned
//                                          ({ "Icons": { "threads.svg": "<id>" }, "Logos": {...} }) and the date go
//                                          into project/design-system.json; an ordinary export has neither
//
// What it does: reads theme.css, the system and Ark stylesheets (system.css, ark.css and what it imports), the icons and the props of the module; builds the
// app to take its compiled CSS; starts its own Vite server (port 28180 with --strictPort, or a free one,
// never 1420); mounts the primitives in a harness page (opening the Ark overlays the way a person does) and captures them; captures the components of
// the app from the browser preview with sample data; and writes the files.
//
// INV-005: the same module and the same sample data give the same bytes. The clock, the locale (it-IT) and the
// time zone (Europe/Rome) are fixed in scripts/ds-export/config.mjs, no file carries a date or an absolute
// path, and a capture that does not settle is an error. tests/ds-export.mjs runs this twice and compares.
//
// To look at the result: node scripts/ds-export/verify.mjs [--dir <export folder>] renders every card in both themes.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { parseArgs, PRIMITIVES } from './ds-export/config.mjs';
import { moduleDigest, readBolt, readFonts, readIcons, readTheme } from './ds-export/module-source.mjs';
import { buildTokens } from './ds-export/tokens.mjs';
import { allComponents } from './ds-export/props.mjs';
import { buildBundle } from './ds-export/bundle.mjs';
import { build, pickPort, startDev } from './ds-export/server.mjs';
import { launch } from './ds-export/browser.mjs';
import { capturePrimitives } from './ds-export/capture-primitives.mjs';
import { capturePages } from './ds-export/capture-pages.mjs';
import { assemble } from './ds-export/assemble.mjs';

const args = parseArgs(process.argv.slice(2));
const log = (line) => console.log(line);
const partial = Boolean(args.only);
const want = (name) => !args.only || args.only.includes(name);

const theme = readTheme();
const fonts = readFonts();
const icons = readIcons();
const digest = moduleDigest();
const { tokens, summary, count } = buildTokens({ theme, fonts, digest });
log(`module desktop/src/ds@${digest}: ${summary}, icons ${icons.length}`);

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'qamille-ds-'));
const failures = [];
let bundleCss = '';
let server;
let browser;
try {
  if (!partial) {
    log('building the app for its compiled CSS');
    await build(path.join(work, 'app-build'));
    const declared = new Set([
      ...tokens.color.tokens, ...tokens.radius.tokens, ...tokens.shadow.tokens, ...tokens.layout.tokens, ...tokens.zIndex.tokens,
      ...Object.keys(tokens.type.families).map((k) => ({ name: `font-${k}` })),
    ].map((t) => t.name));
    bundleCss = buildBundle({ buildDir: path.join(work, 'app-build'), declared });
  }

  const port = await pickPort(args.port);
  log(`Vite server on ${port}`);
  server = await startDev(port);
  browser = await launch();
  // The first load compiles on demand and may optimize dependencies: a warm-up keeps that out of the captures.
  const warm = await browser.newPage();
  for (const url of ['/', '/reminder.html', '/scripts/ds-export/harness/index.html', '/tests/attachment-panel.html']) {
    await warm.goto(server.base + url, { waitUntil: 'networkidle' }).catch(() => {});
  }
  await warm.close();

  log('primitives, mounted from the module');
  const primitives = await capturePrimitives({ browser, base: server.base, want, icons, log });
  log('components of the app, from the preview');
  const pages = await capturePages({ browser, base: server.base, want, log });
  failures.push(...primitives.failures, ...pages.failures);
  const frags = [...primitives.frags, ...pages.frags];

  if (!partial) {
    for (const name of PRIMITIVES) if (!frags.some((f) => f.comp === name)) failures.push(`${name}: no preview`);
    // A scoped class in a preview that bundle.css does not define means the preview was built from other CSS than the bundle's.
    const used = new Set(frags.flatMap((f) => f.html.match(/svelte-[a-z0-9]+/g) ?? []));
    const unknown = [...used].filter((c) => !bundleCss.includes(`.${c}`));
    if (unknown.length) failures.push(`previews use scoped classes that bundle.css lacks: ${unknown.slice(0, 8).join(', ')}${unknown.length > 8 ? ` (+${unknown.length - 8})` : ''}`);
  }
  if (failures.length) throw new Error(`${failures.length} problem(s):\n  ${failures.join('\n  ')}`);

  const components = allComponents();
  const files = assemble({ tokens, frags, bundleCss, icons, bolt: readBolt(), fonts, digest, tokenCount: count, components, blobs: args.blobs, at: args.at, partial });

  // No file may carry the machine it was made on.
  const leaks = [];
  const forms = (p) => [p, p.replaceAll('\\', '/')].map((s) => s.toLowerCase());
  const machine = [...forms(os.tmpdir()), ...forms(process.cwd()), ...forms(args.out), '127.0.0.1', 'localhost'];
  const drive = /\b[A-Za-z]:[\\/]/;
  for (const [file, data] of files) {
    if (typeof data !== 'string') continue;
    const lower = data.toLowerCase();
    const hit = machine.find((s) => lower.includes(s)) ?? data.match(drive)?.[0];
    if (hit) {
      const i = lower.indexOf(hit.toLowerCase());
      leaks.push(`${file}: ${hit} in ...${JSON.stringify(data.slice(Math.max(0, i - 70), i + 50))}...`);
    }
  }
  if (leaks.length) throw new Error(`the files carry the machine they were made on:\n  ${leaks.join('\n  ')}`);

  if (!partial) for (const dir of ['project', 'upload']) fs.rmSync(path.join(args.out, dir), { recursive: true, force: true });
  if (!partial) fs.rmSync(path.join(args.out, 'files.json'), { force: true });
  for (const [file, data] of [...files].sort(([a], [b]) => (a < b ? -1 : 1))) {
    const target = path.join(args.out, file);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, data);
  }
  log(`${files.size} files written to ${path.relative(process.cwd(), args.out) || '.'}${partial ? ' (partial)' : ''}`);
} finally {
  await browser?.close().catch(() => {});
  server?.stop();
  fs.rmSync(work, { recursive: true, force: true });
}
