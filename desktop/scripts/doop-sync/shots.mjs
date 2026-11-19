#!/usr/bin/env node
// Captures the screenshot frames of the Doop canvas (the states extract.mjs does not export as HTML),
// with hotspots measured on the live DOM. Same browser setup as extract.mjs.
// Usage: node desktop/scripts/doop-sync/shots.mjs <out dir> [--port 1433]
// Writes <out>/<state>.png and <out>/<state>.frame.html. Upload each PNG with upload_asset(local_file=true),
// replace IMAGE_URL in the frame file with the returned URL and push it with set_frame_html.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const desktop = path.resolve(here, '../..');
const { chromium } = createRequire(path.join(desktop, 'package.json'))('@playwright/test');
const frames = JSON.parse(fs.readFileSync(path.join(here, 'frames.json'), 'utf8')).frames;
const id = (k) => frames[k].id;
const out = path.resolve(process.argv[2] && !process.argv[2].startsWith('--') ? process.argv[2] : path.join(here, 'out'));
fs.mkdirSync(out, { recursive: true });
const portArg = process.argv.indexOf('--port');
const port = portArg > 0 ? Number(process.argv[portArg + 1]) : 1433;
const base = `http://127.0.0.1:${port}`;

const go = (k) => `navigate:${id(k)}`;
const nav = (text, k) => ({ find: { sel: 'nav button, button', text }, on: [['click', go(k)]], dissolve: true });
const compose = { find: { sel: 'button.compose-main' }, on: [['click', go('composer-new')], ['key:n', go('composer-new')]] };
const profile = { find: { sel: 'button.profile' }, on: [['click', go('settings')]], dissolve: true };
const sidebar = (skip) =>
  [
    ['compose', compose],
    ['inbox', nav('Posta in arrivo', 'inbox-maria')],
    ['calendar', nav('Calendario', 'calendar')],
    ['meetings', nav('Meeting', 'meetings')],
    ['assistant', nav('Assistente', 'assistant')],
    ['settings', profile],
  ]
    .filter(([k]) => !skip.includes(k))
    .map(([, v]) => v);
const row = (nth, k) => ({ find: { sel: 'button.mail-open', nth }, on: [['click', go(k)]] });
const reply = { find: { sel: 'button.primary', text: 'Rispondi' }, on: [['click', go('composer-reply')], ['key:r', go('composer-reply')]] };
const inboxRows = (self) =>
  [[0, 'inbox-maria'], [1, 'inbox-luca'], [2, 'inbox-sofia'], [3, 'inbox-github']].filter(([, k]) => k !== self).map(([n, k]) => row(n, k));
const draftHot = [
  { find: { sel: 'button', aria: 'Salva e chiudi la bozza' }, on: [['click', 'back'], ['key:Escape', 'back']] },
  { find: { sel: 'button', aria: 'Riduci la bozza' }, on: [['click', 'back']] },
  // Exact text: the sidebar "Inviati" also starts with "Invia".
  { find: { sel: 'button', textEq: 'Invia' }, on: [['click', 'back']] },
  { find: { sel: 'button', aria: 'Elimina bozza' }, on: [['click', 'back']] },
  { find: { sel: 'button', ariaStart: 'Salva e chiudi ', ariaNot: 'Salva e chiudi la bozza' }, on: [['click', 'back']] },
];

const STATES = {
  'inbox-luca': { steps: [{ sel: 'button.mail-open', nth: 1 }], hot: [...sidebar(['inbox']), ...inboxRows('inbox-luca'), reply], alt: 'posta in arrivo con il messaggio di Luca Bianchi aperto' },
  'inbox-sofia': { steps: [{ sel: 'button.mail-open', nth: 2 }], hot: [...sidebar(['inbox']), ...inboxRows('inbox-sofia'), reply], alt: 'posta in arrivo con il messaggio di Sofia aperto' },
  'inbox-github': { steps: [{ sel: 'button.mail-open', nth: 3 }], hot: [...sidebar(['inbox']), ...inboxRows('inbox-github'), reply], alt: 'posta in arrivo con la notifica GitHub aperta' },
  'composer-reply': { steps: [{ sel: 'button.primary', text: 'Rispondi' }], hot: [...draftHot, ...sidebar(['compose'])], alt: 'bozza di risposta al messaggio Q4 proposal' },
  'calendar-new-event': {
    steps: [{ sel: 'button', text: 'Calendario' }, { sel: 'button', textEq: 'Nuovo evento' }],
    hot: [
      { find: { sel: '.event-sheet button', aria: 'Chiudi' }, on: [['click', go('calendar')], ['key:Escape', go('calendar')]] },
      { find: { sel: '.event-sheet button', textEq: 'Annulla' }, on: [['click', go('calendar')]] },
      { find: { sel: '.event-sheet button', textEq: 'Salva' }, on: [['click', go('calendar')]] },
      ...sidebar(['calendar']),
    ],
    alt: 'pannello laterale Nuovo evento aperto sul calendario',
  },
  'meetings-recorder': {
    steps: [{ sel: 'button', text: 'Meeting' }, { sel: 'button', textEq: 'Registra ora' }],
    hot: [{ find: { sel: 'button', textEq: 'Annulla' }, on: [['click', go('meetings')]] }, ...sidebar(['meetings'])],
    alt: 'registratore meeting aperto con microfono e audio del computer',
  },
  'settings-agent': {
    steps: [{ sel: 'button.profile' }, { sel: 'button', textEq: 'Accesso agenti' }],
    hot: [{ find: { sel: 'button', textEq: 'Generali' }, on: [['click', go('settings')]] }, ...sidebar(['settings'])],
    alt: 'impostazioni, sezione Accesso agenti',
  },
};

function pick(f) {
  const all = [...document.querySelectorAll(f.sel)].filter((e) => {
    const text = e.innerText?.trim() ?? '';
    const aria = e.getAttribute('aria-label') ?? '';
    if (f.text && !text.startsWith(f.text)) return false;
    if (f.textEq && text !== f.textEq) return false;
    if (f.aria && aria !== f.aria) return false;
    if (f.ariaStart && !aria.startsWith(f.ariaStart)) return false;
    if (f.ariaNot && aria === f.ariaNot) return false;
    const r = e.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  });
  return all[f.nth ?? 0] ?? null;
}

const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: desktop, stdio: 'ignore', windowsHide: true });
for (let i = 0; i < 100; i++) {
  try { if ((await fetch(base)).ok) break; } catch { /* starting */ }
  await new Promise((r) => setTimeout(r, 200));
}
let browser;
try {
  try { browser = await chromium.launch({ channel: 'msedge' }); } catch { browser = await chromium.launch(); }
  const context = await browser.newContext({ locale: 'it-IT', timezoneId: 'Europe/Rome', viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await page.clock.setFixedTime(new Date('2026-11-03T15:00:00+01:00'));
  for (const [state, def] of Object.entries(STATES)) {
    await page.goto(base);
    await page.evaluate(() => { localStorage.clear(); sessionStorage.clear(); });
    await page.goto(base);
    await page.waitForSelector('button.mail-open');
    for (const s of def.steps) {
      const ok = await page.evaluate(`(() => { const e = (${pick})(${JSON.stringify(s)}); if (!e) return 'missing'; e.click(); return 'ok'; })()`);
      if (ok === 'missing') throw new Error(`${state}: step target missing ${JSON.stringify(s)}`);
      await page.waitForTimeout(500);
    }
    await page.mouse.move(1439, 899);
    await page.waitForTimeout(700);
    await page.screenshot({ path: path.join(out, `${state}.png`) });
    const spots = [];
    for (const h of def.hot) {
      const r = await page.evaluate(`(() => { const e = (${pick})(${JSON.stringify(h.find)}); if (!e) return null; const b = e.getBoundingClientRect(); return [Math.round(b.left), Math.round(b.top), Math.round(b.width), Math.round(b.height)]; })()`);
      if (!r) { console.warn(`${state}: hotspot missing ${JSON.stringify(h.find)}`); continue; }
      const style = `left:${r[0]}px;top:${r[1]}px;width:${r[2]}px;height:${r[3]}px`;
      if (h.on.length > 1) {
        const json = JSON.stringify(h.on.map(([on, action]) => ({ on, action })));
        spots.push(`<i class="h" style="${style}" data-doop-interactions='${json}'></i>`);
      } else {
        const [on, action] = h.on[0];
        spots.push(`<i class="h" style="${style}" data-doop-on="${on}" data-doop-action="${action}"${h.dissolve ? ' data-doop-transition="dissolve" data-doop-duration="120"' : ''}></i>`);
      }
    }
    const head =
      '<!doctype html>\n<html lang="it"><head><meta charset="utf-8"><style>*{margin:0}html,body{width:1440px;height:900px;overflow:hidden;background:#fff}img{display:block;width:1440px;height:900px}.h{position:absolute;display:block;cursor:pointer}</style></head><body>';
    fs.writeFileSync(path.join(out, `${state}.frame.html`), `${head}<img src="IMAGE_URL" alt="Qamille: ${def.alt}">${spots.join('\n')}</body></html>`);
    console.log(`${state}: ${spots.length}/${def.hot.length} hotspots`);
  }
} finally {
  await browser?.close();
  server.kill();
}
