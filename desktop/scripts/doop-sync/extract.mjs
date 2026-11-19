#!/usr/bin/env node
// Captures real, editable HTML for the Doop prototype frames from the browser preview of the app.
//
//   node desktop/scripts/doop-sync/extract.mjs --out <dir> [--only id,id] [--port 1425]
//
// For each state it writes <dir>/<id>.html: a self-contained document (pruned CSS, shared SVG
// sprite, Google Fonts link) whose real elements carry the data-doop-* prototype interactions.
// The agent then pushes each file to its Doop frame with set_frame_html (frame ids in frames.json).
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const desktop = path.resolve(here, '../..');
const { chromium } = createRequire(path.join(desktop, 'package.json'))('@playwright/test');
const frames = JSON.parse(fs.readFileSync(path.join(here, 'frames.json'), 'utf8'));

const arg = (name, fallback) => { const i = process.argv.indexOf(`--${name}`); return i > 0 ? process.argv[i + 1] : fallback; };
const outDir = path.resolve(arg('out', path.join(here, 'out')));
const port = Number(arg('port', 1425));
const only = arg('only', '')?.split(',').filter(Boolean);
const base = `http://127.0.0.1:${port}`;
const scrollbars = arg('scrollbars', 'keep');
const restoreScroll = '<script>document.querySelectorAll("[data-st]").forEach(function(e){var s=e.getAttribute("data-st").split(",");e.scrollTop=+s[0];e.scrollLeft=+s[1]});var f=document.querySelector("[data-focus]");if(f)f.focus()</script>';
const verifyDir = arg('verify', '') ? path.resolve(arg('verify')) : '';
if (verifyDir) fs.mkdirSync(verifyDir, { recursive: true });

const nav = (text, target) => ({ find: { sel: 'button', text }, go: target });
const SIDEBAR = (self) => [
  { find: { sel: 'button.compose-main' }, go: 'composer-new', key: 'n', skip: self === 'composer-new' },
  nav('Posta in arrivo', 'inbox-maria'),
  nav('Calendario', 'calendar'),
  nav('Meeting', 'meetings'),
  nav('Assistente', 'assistant'),
  { find: { sel: 'button.profile' }, go: 'settings' },
].filter((h) => h.go !== self && !h.skip);

const STATES = {
  'inbox-maria': {
    steps: [],
    hot: [
      ...SIDEBAR('inbox-maria'),
      { find: { sel: 'button.mail-open', nth: 1 }, go: 'inbox-luca' },
      { find: { sel: 'button.mail-open', nth: 2 }, go: 'inbox-sofia' },
      { find: { sel: 'button.mail-open', nth: 3 }, go: 'inbox-github' },
      { find: { sel: 'button.primary', text: 'Rispondi' }, go: 'composer-reply', key: 'r' },
    ],
  },
  'composer-new': {
    steps: [{ sel: 'button.compose-main' }],
    hot: [
      ...SIDEBAR('composer-new'),
      { find: { sel: 'button', aria: 'Salva e chiudi la bozza' }, go: 'back', key: 'Escape' },
      { find: { sel: 'button', aria: 'Riduci la bozza' }, go: 'back' },
      { find: { sel: 'button', textEq: 'Invia' }, go: 'back' },
      { find: { sel: 'button', aria: 'Elimina bozza' }, go: 'back' },
      { find: { sel: 'button', ariaStart: 'Salva e chiudi ', ariaNot: 'Salva e chiudi la bozza' }, go: 'back' },
    ],
  },
  calendar: {
    steps: [{ sel: 'button', text: 'Calendario' }],
    hot: [...SIDEBAR('calendar'), { find: { sel: 'button', textEq: 'Nuovo evento' }, go: 'calendar-new-event' }],
  },
  meetings: {
    steps: [{ sel: 'button', text: 'Meeting' }],
    hot: [...SIDEBAR('meetings'), { find: { sel: 'button', textEq: 'Registra ora' }, go: 'meetings-recorder' }],
  },
  assistant: { steps: [{ sel: 'button', text: 'Assistente' }], hot: SIDEBAR('assistant') },
  settings: {
    steps: [{ sel: 'button.profile' }],
    hot: [...SIDEBAR('settings'), { find: { sel: 'button', textEq: 'Accesso agenti' }, go: 'settings-agent' }],
  },
};

// Runs inside the page: wires interactions, then returns { html, css, lang, theme }.
function pageExtract({ hot, ids }) {
  const find = (f) => {
    let els = [...document.querySelectorAll(f.sel || 'button')];
    const label = (e) => e.getAttribute('aria-label') || '';
    if (f.text) els = els.filter((e) => e.innerText.trim().startsWith(f.text));
    if (f.textEq) els = els.filter((e) => e.innerText.trim() === f.textEq);
    if (f.aria) els = els.filter((e) => label(e) === f.aria);
    if (f.ariaStart) els = els.filter((e) => label(e).startsWith(f.ariaStart));
    if (f.ariaNot) els = els.filter((e) => label(e) !== f.ariaNot);
    // Hidden copies (a minimized draft, a closed panel) would take the wiring and then be pruned.
    els = els.filter((e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; });
    return els[f.nth || 0];
  };
  const missing = [];
  for (const h of hot) {
    const el = find(h.find);
    if (!el) { missing.push(JSON.stringify(h.find)); continue; }
    const action = h.go === 'back' ? 'back' : `navigate:${ids[h.go]}`;
    const list = [{ on: 'click', action }];
    if (h.key) list.push({ on: `key:${h.key}`, action });
    el.setAttribute('data-doop-interactions', JSON.stringify(list));
  }
  document.querySelectorAll('input,textarea,select,option').forEach((e) => {
    if (e.tagName === 'INPUT') { e.setAttribute('value', e.value); if (e.checked) e.setAttribute('checked', ''); }
    if (e.tagName === 'TEXTAREA') e.textContent = e.value;
    if (e.tagName === 'OPTION' && e.selected) e.setAttribute('selected', '');
  });

  // Frames do not scroll: drop everything that lies fully outside the 1440x900 viewport.
  const W = window.innerWidth;
  const H = window.innerHeight;
  // Scroll offsets cannot live in static HTML: record them, a tiny script restores them in the frame.
  const focused = document.activeElement && document.activeElement !== document.body ? document.activeElement : null;
  focused?.setAttribute('data-focus', '');
  const scrolled = [...document.body.querySelectorAll('*')].filter((e) => e.scrollTop > 0 || e.scrollLeft > 0);
  scrolled.forEach((e) => e.setAttribute('data-st', `${Math.round(e.scrollTop)},${Math.round(e.scrollLeft)}`));
  document.body.querySelectorAll('*').forEach((e) => {
    const r = e.getBoundingClientRect();
    if (scrolled.some((s) => s.contains(e) && s !== e)) return;
    if (r.width && r.height && (r.top >= H || r.left >= W || r.bottom <= 0 || r.right <= 0)) e.setAttribute('data-prune', '');
  });
  // Pruning shortens scroll containers, which would change their scrollbars. Measure the loss and put it back as a spacer.
  const scrollers = [...document.body.querySelectorAll('*')].filter((e) => e.scrollHeight > e.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(e).overflowY));
  scrollers.forEach((e, i) => e.setAttribute('data-sc', String(i)));
  const before = scrollers.map((e) => e.scrollHeight);
  const pruned = [...document.body.querySelectorAll('[data-prune]')];
  const shown = pruned.map((e) => e.style.display);
  pruned.forEach((e) => { e.style.display = 'none'; });
  const spacers = scrollers.map((e, i) => Math.max(0, before[i] - e.scrollHeight));
  pruned.forEach((e, i) => { e.style.display = shown[i]; });
  const body = document.body.cloneNode(true);
  body.querySelectorAll('script,noscript,[data-doop-sync-ignore],[data-prune]').forEach((n) => n.remove());
  body.querySelectorAll('[data-sc]').forEach((e) => {
    const extra = spacers[Number(e.getAttribute('data-sc'))];
    e.removeAttribute('data-sc');
    if (extra > 0) e.insertAdjacentHTML('beforeend', `<div aria-hidden="true" style="flex:none;height:${extra}px"></div>`);
  });
  // A select is as wide as its widest option; keep its width now that the other options are dropped.
  document.body.querySelectorAll('select').forEach((s, i) => s.setAttribute('data-sw', `${s.getBoundingClientRect().width}`));
  body.querySelectorAll('select').forEach((s, i) => { s.style.width = `${[...document.body.querySelectorAll('select')][i].getBoundingClientRect().width}px`; });
  body.querySelectorAll('*').forEach((e) => e.removeAttribute('data-prune'));
  // A frame shows only the selected option; long lists (time zones, languages) would dominate the size.
  body.querySelectorAll('select option:not([selected])').forEach((o) => o.remove());

  const baseSel = (s) => s.replace(/::?[a-zA-Z-]+(\([^)]*\))?/g, '').replace(/[>+~\s]+$/, '').trim() || '*';
  const used = (sel) => { try { return body.querySelector(sel) !== null; } catch { return true; } };
  const interactive = /:(hover|active|focus|focus-visible|focus-within)\b|::selection/;
  // Hover/focus rules only matter for what is hovered or focused right now (the frame restores focus with a script).
  const activeNow = (s) => { try { return document.querySelector(s) !== null; } catch { return false; } };
  const walk = (rules, out) => {
    for (const r of rules) {
      if (r.type === 1) {
        const keep = r.selectorText.split(',').filter((s) => (!interactive.test(s) || activeNow(s.trim())) && (/^(:root|html|body|\*)/.test(s.trim()) || used(baseSel(s.trim()))));
        if (keep.length) out.push(`${keep.join(',')}{${r.style.cssText.replace(/;\s+/g, ';').replace(/:\s+/g, ':')}}`);
      } else if (r.type === 4) {
        if (/\bprint\b/.test(r.conditionText)) continue;
        const inner = []; walk(r.cssRules, inner);
        if (inner.length) out.push(`@media ${r.conditionText}{${inner.join('')}}`);
      } else if (r.type === 7) out.push(r.cssText);
    }
  };
  const css = [];
  for (const sheet of document.styleSheets) { try { walk(sheet.cssRules, css); } catch { /* cross-origin */ } }
  const symbols = new Map();
  body.querySelectorAll('svg').forEach((svg) => {
    const inner = svg.innerHTML;
    if (!inner || inner.startsWith('<use')) return;
    if (!symbols.has(inner)) symbols.set(inner, `i${symbols.size}`);
    svg.innerHTML = `<use href="#${symbols.get(inner)}"/>`;
  });
  body.querySelectorAll('*').forEach((e) => { for (const a of [...e.attributes]) if (/^data-svelte|^data-sveltekit/.test(a.name)) e.removeAttribute(a.name); });
  const sprite = symbols.size
    ? `<svg width="0" height="0" style="position:absolute" aria-hidden="true"><defs>${[...symbols].map(([inner, id]) => `<symbol id="${id}">${inner}</symbol>`).join('')}</defs></svg>`
    : '';
  const html = (sprite + body.innerHTML).replace(/<!--[\s\S]*?-->/g, '');
  const longhands = /font-style:inherit;font-variant:inherit;font-weight:inherit;font-stretch:inherit;line-height:inherit;font-family:inherit;font-optical-sizing:inherit;font-size-adjust:inherit;font-kerning:inherit;font-feature-settings:inherit;font-variation-settings:inherit;font-language-override:inherit;/g;
  const cssText = css.filter((c) => !c.startsWith('@keyframes svelte-')).join('\n').replace(longhands, 'font:inherit;');
  return { html, css: cssText, lang: document.documentElement.lang, theme: document.documentElement.dataset.theme || '', missing, scrolls: scrolled.length + (focused ? 1 : 0) };
}

async function startServer() {
  const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: desktop, stdio: 'ignore', windowsHide: true });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(base)).ok) return server; } catch { /* starting */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  server.kill();
  throw new Error('Preview server did not start.');
}

fs.mkdirSync(outDir, { recursive: true });
const server = await startServer();
let browser;
try {
  try { browser = await chromium.launch({ channel: 'msedge' }); } catch { browser = await chromium.launch(); }
  const context = await browser.newContext({ locale: 'it-IT', timezoneId: 'Europe/Rome', viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await page.clock.setFixedTime(new Date('2026-11-03T15:00:00+01:00'));
  const ids = Object.fromEntries(Object.entries(frames.frames).map(([k, v]) => [k, v.id]));
  const summary = [];
  for (const [id, state] of Object.entries(STATES)) {
    if (only.length && !only.includes(id)) continue;
    await page.goto(base);
    await page.evaluate(() => { localStorage.clear(); sessionStorage.clear(); });
    await page.goto(base);
    await page.waitForSelector('button.mail-open');
    for (const s of state.steps) {
      await page.evaluate((f) => {
        const el = [...document.querySelectorAll(f.sel)].find((e) => !f.text || e.innerText.trim().startsWith(f.text));
        if (!el) throw new Error(`step target missing: ${JSON.stringify(f)}`);
        el.click();
      }, s);
      await page.waitForTimeout(500);
    }
    await page.waitForTimeout(700);
    if (verifyDir) await page.screenshot({ path: path.join(verifyDir, `${id}.real.png`) });
    const r = await page.evaluate(pageExtract, { hot: state.hot, ids });
    if (r.missing.length) console.warn(`[${id}] hotspot targets not found: ${r.missing.join(' | ')}`);
    const doc = `<!doctype html><html lang="${r.lang}"${r.theme ? ` data-theme="${r.theme}"` : ''}><head><meta charset="utf-8"><link rel="preconnect" href="https://fonts.googleapis.com"><link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Lexend:wght@100..900&family=Source+Sans+3:wght@200..900&display=swap"><style>${r.css}${scrollbars === 'hide' ? '\n*{scrollbar-gutter:auto!important;scrollbar-width:none!important}' : ''}</style></head><body>${r.html}${r.scrolls ? restoreScroll : ''}</body></html>`;
    const file = path.join(outDir, `${id}.html`);
    fs.writeFileSync(file, doc);
    if (verifyDir) {
      // Same engine, same viewport: render the generated document on its own and screenshot it.
      const check = await context.newPage();
      await check.setViewportSize({ width: 1440, height: 900 });
      await check.setContent(doc, { waitUntil: 'networkidle' });
      await check.evaluate(() => document.fonts.ready);
      await check.waitForTimeout(500);
      await check.screenshot({ path: path.join(verifyDir, `${id}.html.png`) });
      await check.close();
    }
    const sha = crypto.createHash('sha256').update(doc).digest('hex').slice(0, 12);
    summary.push(`${id}: ${doc.length} bytes, sha ${sha}${r.missing.length ? `, ${r.missing.length} hotspot(s) missing` : ''}`);
  }
  console.log(summary.join('\n'));
} finally {
  await browser?.close();
  server.kill();
}
