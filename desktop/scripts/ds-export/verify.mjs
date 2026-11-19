#!/usr/bin/env node
// Renders each preview of an export the way the design system frame does (tokens.css compiled from tokens.json,
// then bundle.css, data-theme on html) and writes one screenshot per card and theme. Reports the cards that
// overflow, scroll or raise a page error.
//
//   node scripts/ds-export/verify.mjs [--dir desktop/ds-export] [--out <folder for the screenshots>]
//
// Not part of the export and not run by the test: a look at the result, with the same fixed locale and time zone.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { desktop, FROZEN } from './config.mjs';
import { launch, newContext } from './browser.mjs';
import { pickPort } from './server.mjs';

const arg = (flag) => { const i = process.argv.indexOf(flag); return i >= 0 ? process.argv[i + 1] : undefined; };
const dir = path.resolve(arg('--dir') ?? path.join(desktop, 'ds-export'));
const out = path.resolve(arg('--out') ?? path.join(dir, '_shots'));
const proj = path.join(dir, 'project');
const tokens = JSON.parse(fs.readFileSync(path.join(proj, 'tokens.json'), 'utf8'));

// The same layout as the format's "tokens.css as compiled".
function compileTokens() {
  const first = tokens.color.themes[0].id;
  const pick = (t, theme) => (typeof t.value === 'string' ? t.value : t.value[theme] ?? t.value[first]);
  const themed = [...tokens.color.tokens, ...tokens.shadow.tokens];
  let css = `:root,[data-theme="${first}"]{${themed.map((t) => `--${t.name}:${pick(t, first)}`).join(';')}}\n`;
  for (const th of tokens.color.themes.slice(1)) css += `[data-theme="${th.id}"]{${themed.filter((t) => typeof t.value !== 'string' && t.value[th.id]).map((t) => `--${t.name}:${t.value[th.id]}`).join(';')}}\n`;
  const plain = [...tokens.radius.tokens, ...tokens.layout.tokens, ...tokens.zIndex.tokens, ...(tokens.motion?.tokens ?? [])];
  css += `:root{${plain.map((t) => `--${t.name}:${t.value}`).join(';')};${Object.entries(tokens.type.families).map(([k, v]) => `--font-${k}:${v}`).join(';')}}\n`;
  for (const f of tokens.type.fonts) css += `@font-face{font-family:'${f.family}';src:url('/project/${f.file}');font-weight:${f.weight};font-style:${f.style}}\n`;
  return css;
}
const tokensCss = compileTokens();

const server = http.createServer((req, res) => {
  const url = decodeURI(req.url.split('?')[0]);
  if (url === '/tokens.css') { res.writeHead(200, { 'content-type': 'text/css' }); res.end(tokensCss); return; }
  if (url.startsWith('/frame/')) {
    const [, , comp, theme] = url.split('/');
    let doc = fs.readFileSync(path.join(proj, 'components', comp, 'preview.html'), 'utf8');
    doc = doc.replace('<head>', '<head><link rel="stylesheet" href="/tokens.css"><link rel="stylesheet" href="/project/components/bundle.css">').replace(/<html[^>]*>/, `<html lang="it" data-theme="${theme}">`);
    res.writeHead(200, { 'content-type': 'text/html' });
    res.end(doc);
    return;
  }
  if (url.startsWith('/project/')) {
    const file = path.join(proj, url.slice('/project/'.length));
    if (fs.existsSync(file)) { res.writeHead(200, { 'content-type': file.endsWith('.css') ? 'text/css' : 'font/ttf' }); fs.createReadStream(file).pipe(res); return; }
  }
  res.writeHead(404);
  res.end();
});
const port = await pickPort(28183);
await new Promise((r) => server.listen(port, '127.0.0.1', r));
const browser = await launch();
const context = await newContext(browser, { viewport: { width: 1440, height: 1000 } });
const comps = fs.readdirSync(path.join(proj, 'components'), { withFileTypes: true }).filter((e) => e.isDirectory() && fs.existsSync(path.join(proj, 'components', e.name, 'preview.html'))).map((e) => e.name).sort();
const results = {};
try {
  for (const theme of ['light', 'dark']) {
    const shots = path.join(out, theme);
    fs.mkdirSync(shots, { recursive: true });
    for (const c of comps) {
      const marker = fs.readFileSync(path.join(proj, 'components', c, 'preview.html'), 'utf8').split('\n')[0];
      const w = Number((marker.match(/width=(\d+)/) || [])[1] || 960);
      const h = Number((marker.match(/height=(\d+)/) || [])[1] || 120);
      const page = await context.newPage();
      await page.setViewportSize({ width: w, height: Math.max(h, 40) });
      const errs = [];
      page.on('pageerror', (e) => errs.push(e.message.split('\n')[0]));
      await page.goto(`http://127.0.0.1:${port}/frame/${c}/${theme}`, { waitUntil: 'networkidle' });
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(120);
      await page.screenshot({ path: path.join(shots, `${c}.png`) });
      const stats = await page.evaluate(() => ({
        overflowed: document.documentElement.scrollWidth > innerWidth + 1,
        tall: document.documentElement.scrollHeight > innerHeight + 1,
      }));
      results[`${c}/${theme}`] = { ...stats, errs };
      await page.close();
    }
  }
} finally {
  await browser.close();
  server.close();
}
const bad = Object.entries(results).filter(([, r]) => r.errs.length || r.overflowed || r.tall);
console.log(`previews ${comps.length} x 2 themes (${FROZEN.locale}), problems ${bad.length}, screenshots in ${path.relative(process.cwd(), out)}`);
for (const [k, r] of bad) console.log(k, JSON.stringify(r).slice(0, 180));
