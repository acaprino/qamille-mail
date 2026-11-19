// Captures the components of the app from the running preview: the browser build, sample data, Italian.
// The primitives of the module are not here: they are mounted by capture-primitives.mjs.
import fs from 'node:fs';
import path from 'node:path';
import { desktop } from './config.mjs';
import { capture, fresh, freezeClock, newContext, ready, settle, step } from './browser.mjs';
import { IDS, SUBJECT, withConversation } from '../../tests/support/conversation-fixture.mjs';

const side = (text) => ({ sel: '.sidebar button', text });
const more = { aria: 'Altre azioni sul messaggio' };
const settingsTab = (textEq) => ({ sel: '.settings-nav button', textEq });
// A Modal is an Ark Dialog: its content carries the caller's class and the data-scope of Ark.
const dialog = (cls) => `.${cls}[data-scope="dialog"][data-part="content"]`;
const profile = { sel: 'button.profile' };
const compose = { sel: 'button.compose-main' };
const contactsPage = (extra) => ({ sel: '.contacts-page button', ...extra });
const meetingsPage = (extra) => ({ sel: '.meetings-page button', ...extra });

// Component inventory: how to reach it from a fresh load and what to take.
export const FRAGS = [
  // Classes of the system that are not Svelte components: real instances.
  { comp: 'Avatar', group: 'Primitive', steps: [], sel: [{ sel: '.sidebar .account-avatar', nth: 0 }, { sel: '.sidebar .account-avatar', nth: 1 }, { sel: '.sidebar .account-avatar', nth: 2 }, { sel: '.sidebar .user-avatar' }] },
  { comp: 'SearchChip', group: 'Primitive', steps: [], sel: [{ sel: '.search-filter-button', nth: 0 }, { sel: '.search-filter-button', nth: 1 }, { sel: '.search-filter-button', nth: 2 }] },
  { comp: 'SettingRow', group: 'Primitive', steps: [profile], sel: [{ sel: '.settings-content .setting-row', nth: 0 }, { sel: '.settings-content .setting-row', nth: 1 }] },
  // Shell and mail.
  { comp: 'Sidebar', group: 'Struttura', steps: [], sel: ['aside.sidebar'] },
  { comp: 'SearchBar', group: 'Struttura', steps: [], sel: ['section.search-workbench'] },
  { comp: 'MailList', group: 'Posta', steps: [], sel: ['section.mail-list'] },
  { comp: 'Reader', group: 'Posta', steps: [], sel: ['section.reader'] },
  { comp: 'MessageHtml', group: 'Posta', steps: [{ sel: '.group-heading', textI: 'newsletter' }, { sel: 'button.mail-open', has: 'The Weekly Brief' }], sel: ['.message-body'] },
  { comp: 'UndoStack', group: 'Posta', steps: [{ ariaStart: 'Archivia' }], sel: ['section.undo-stack'] },
  { comp: 'AttachmentPanel', group: 'Posta', steps: [{ text: 'Q4-proposal.txt' }], sel: ['[data-scope="dialog"][data-part="positioner"][data-variant="side"]'] },
  { comp: 'AttachmentPreview', group: 'Posta', steps: [{ text: 'Q4-proposal.txt' }], sel: ['.attachment-preview'] },
  { comp: 'PrintPreview', group: 'Posta', steps: [more, { sel: '[data-scope="popover"][data-part="content"] button', text: 'Stampa email' }], sel: [dialog('mail-print-preview')] },
  { comp: 'AddFromMail', group: 'Posta', steps: [{ textEq: 'Aggiungi ai contatti' }], sel: [dialog('contact-dialog')] },
  // Composer.
  { comp: 'ComposerDock', group: 'Composizione', steps: [compose], sel: ['footer.composer-dock'] },
  { comp: 'Composer', group: 'Composizione', steps: [compose], sel: ['.composer-panel'] },
  { comp: 'RichEditor', group: 'Composizione', steps: [compose], sel: ['.rich-editor'] },
  { comp: 'RecipientInput', group: 'Composizione', steps: [compose], sel: ['[data-scope="combobox"][data-part="root"]'] },
  // Pages.
  { comp: 'Calendar', group: 'Calendario', steps: [side('Calendario')], sel: ['.calendar-page'] },
  { comp: 'Meetings', group: 'Meeting', steps: [side('Meeting')], sel: ['section.meetings-page'] },
  { comp: 'Assistant', group: 'Assistente', steps: [side('Assistente')], sel: ['section.assistant-page'] },
  { comp: 'Contacts', group: 'Rubrica', steps: [side('Rubrica')], sel: ['section.contacts-page'] },
  { comp: 'ContactForm', group: 'Rubrica', steps: [side('Rubrica'), { textEq: 'Nuovo contatto' }], sel: [dialog('contact-dialog')] },
  { comp: 'ContactFieldsEditor', group: 'Rubrica', steps: [side('Rubrica'), { textEq: 'Nuovo contatto' }], sel: ['.contact-fields'] },
  { comp: 'DuplicatesPanel', group: 'Rubrica', steps: [side('Rubrica'), contactsPage({ text: 'Duplicati' })], sel: ['section.duplicates-panel'] },
  { comp: 'MergeDialog', group: 'Rubrica', steps: [side('Rubrica'), contactsPage({ text: 'Duplicati' }), { sel: '.duplicates-panel button', text: 'Unisci' }], sel: [dialog('merge-dialog')] },
  { comp: 'Settings', group: 'Impostazioni', steps: [profile], sel: ['section.settings-page'] },
  { comp: 'UnsavedDialog', group: 'Impostazioni', steps: [profile, { fill: '.settings-content input:not([type=checkbox]):not([type=radio])', value: 'Alex B.' }, side('Posta in arrivo')], sel: [dialog('unsaved-dialog')] },
  { comp: 'MeetingRecordingIndicator', group: 'Meeting', steps: [side('Meeting'), meetingsPage({ textEq: 'Registra ora' }), { sel: '.record-form button', textEq: 'Registra', wait: 900 }, side('Posta in arrivo')], sel: ['button.recording-indicator'] },
  { comp: 'AccountStorage', group: 'Impostazioni', steps: [profile, settingsTab('Archivio locale')], sel: ['.settings-content'] },
  { comp: 'IndexingSettings', group: 'Impostazioni', steps: [profile, settingsTab('Ricerca')], sel: ['.settings-content'] },
  { comp: 'ContactsSettings', group: 'Impostazioni', steps: [profile, settingsTab('Rubrica')], sel: ['.settings-content'] },
  { comp: 'McpSettings', group: 'Impostazioni', steps: [profile, settingsTab('Accesso agenti')], sel: ['.mcp-settings'] },
  { comp: 'AssistantInstallers', group: 'Impostazioni', steps: [profile, settingsTab('Accesso agenti')], sel: ['section.assistant-installers'] },
  { comp: 'GoogleSetupGuide', group: 'Impostazioni', steps: [profile, settingsTab('Account'), { text: "Come attivare l'accesso Google" }], sel: [dialog('google-setup-guide')] },
  { comp: 'GoogleSignInDialog', group: 'Impostazioni', steps: [profile, settingsTab('Account'), { text: 'Accedi con Google' }], sel: [dialog('google-sign-in-dialog')] },
];

// The desktop bridge, faked: the app asks it for a workspace, the pages that live in their own window ask for more.
const bridge = ({ settings, account, start }) => {
  const callbacks = new Map();
  const listeners = new Map();
  let id = 0;
  const workspace = { accounts: [{ ...account, id: 1, name: 'Work', email: 'alex@studio-example.com', kind: 'Gmail', last_sync: 1793000000 }], counts: {}, labels: [], signatures: [], drafts: [], undo_actions: [], has_ai_key: false, has_google_secret: false, settings: { ...settings, locale: 'it-IT', dark: false } };
  window.isTauri = true;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback(cb) { const n = ++id; callbacks.set(n, cb); return n; },
    async invoke(command, args = {}) {
      if (command === 'plugin:event|listen') { listeners.set(args.event, [...(listeners.get(args.event) || []), args.handler]); return args.handler; }
      if (command === 'plugin:event|unlisten') return null;
      if (command === 'plugin:window|is_maximized') return false;
      if (command === 'workspace_snapshot') return structuredClone(workspace);
      if (command === 'mail_sync_status') return { revision: 7, accounts: [{ account_id: 1, phase: 'syncing', added: 0, errors: [] }] };
      if (command === 'list_mail') return { items: [], total: 0, next_offset: null };
      if (command === 'meeting_capture_status') return { state: 'idle' };
      if (command === 'meeting_list_page') return { items: [], total: 0, next_offset: null };
      if (command === 'reminder_view') return { chips: [{ key: 'k1', title: 'Design review', start, end: start + 3600, relative: { kind: 'in', minutes: 1 } }], recording: false, dark: false, locale: 'it-IT', time_zone: 'Europe/Rome' };
      return null;
    },
  };
};

// Pages that need the bridge. Not every one freezes the clock: the activity indicator only appears once
// real time has passed, and the date does not enter its markup.
const NATIVE = [
  { comp: 'ActivityIndicator', group: 'Struttura', file: '/', sel: ['.activity-indicator'], w: 1440, h: 900, clock: false },
  { comp: 'WindowControls', group: 'Struttura', file: '/', sel: ['.window-controls'], w: 1440, h: 900, clock: true },
  { comp: 'Reminder', group: 'Meeting', file: '/reminder.html', sel: ['section.chip'], w: 520, h: 68, clock: true },
];

/** Every page component, in a stable order. `want(name)` filters when the export runs partially. */
export async function capturePages({ browser, base, want, log }) {
  const frags = [];
  const failures = [];
  const done = (comp, group, r) => {
    frags.push({ comp, group, source: 'app', ...r });
    log(`  ${comp}: ${r.parts.map((p) => `${p.w}x${p.h}`).join(' ')}${r.missing.length ? `, MISSING ${r.missing.join(',')}` : ''}`);
  };
  const failed = (comp, e) => { failures.push(`${comp}: ${String(e.message).split('\n')[0]}`); log(`  ${comp}: FAILED ${String(e.message).split('\n')[0]}`); };

  const context = await newContext(browser);
  const page = await context.newPage();
  await freezeClock(page);
  try {
    for (const f of FRAGS) {
      if (!want(f.comp)) continue;
      try {
        await fresh(page, base);
        for (const st of f.steps) {
          if (st.fill) { await page.locator(st.fill).filter({ visible: true }).first().fill(st.value); await page.waitForTimeout(200); continue; }
          await step(page, st);
        }
        await ready(page, f.sel);
        await settle(page);
        done(f.comp, f.group, await capture(page, f.sel));
      } catch (e) {
        failed(f.comp, e);
      }
    }
  } finally {
    await context.close();
  }

  // A conversation in the list: the sample has none, so the fixture of the tests is served in its place.
  const wantThread = want('ThreadReplies') || want('RecipientLine');
  if (wantThread) {
    const sample = JSON.parse(fs.readFileSync(path.join(desktop, 'src/lib/sample.json'), 'utf8'));
    const fixture = withConversation(sample);
    const ctx = await newContext(browser);
    try {
      await ctx.addInitScript(() => localStorage.setItem('qamille:mail-display', 'threads'));
      const pg = await ctx.newPage();
      await freezeClock(pg);
      await pg.route(/\/src\/lib\/sample\.json/, (route) => route.fulfill({ contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};` }));
      await pg.goto(base);
      await pg.locator('.mail-row').first().waitFor({ timeout: 60000 });
      await pg.locator('.mail-row', { hasText: SUBJECT }).locator('.thread-toggle').click();
      await pg.locator('.reply-row').first().waitFor();
      if (want('ThreadReplies')) {
        try { await settle(pg); done('ThreadReplies', 'Posta', await capture(pg, ['.thread-replies'])); } catch (e) { failed('ThreadReplies', e); }
      }
      if (want('RecipientLine')) {
        try {
          await pg.locator(`.reply-row[data-reply-id="${IDS.many}"] .reply-open`).click();
          await pg.locator('.recipient-line[data-kind="to"]').first().waitFor();
          await settle(pg);
          done('RecipientLine', 'Posta', await capture(pg, ['.recipient-line[data-kind="to"]']));
        } catch (e) { failed('RecipientLine', e); }
      }
    } catch (e) {
      if (want('ThreadReplies')) failed('ThreadReplies', e);
      if (want('RecipientLine')) failed('RecipientLine', e);
    } finally {
      await ctx.close();
    }
  }

  const sample = JSON.parse(fs.readFileSync(path.join(desktop, 'src/lib/sample.json'), 'utf8'));
  const start = Date.parse('2026-11-03T15:01:00+01:00') / 1000;
  for (const f of NATIVE) {
    if (!want(f.comp)) continue;
    const ctx = await newContext(browser, { viewport: { width: f.w, height: f.h } });
    try {
      await ctx.addInitScript(bridge, { settings: sample.settings, account: sample.accounts[0], start });
      const pg = await ctx.newPage();
      if (f.clock) await freezeClock(pg);
      await pg.goto(base + f.file);
      await pg.waitForSelector(f.sel[0], { timeout: 30000 });
      await pg.waitForTimeout(500);
      await pg.evaluate(() => document.fonts.ready);
      done(f.comp, f.group, await capture(pg, f.sel));
    } catch (e) {
      failed(f.comp, e);
    } finally {
      await ctx.close();
    }
  }

  // AttachedMessageView only appears for .eml attachments: the fixture page of tests/attachment-panel.mjs.
  if (want('AttachedMessageView')) {
    const ctx = await newContext(browser);
    try {
      await ctx.addInitScript(() => {
        const pec = { subject: 'Avviso di accertamento', from_name: 'Ufficio Tributi', from_addr: 'protocollo@pec.comune.example', to: ['Mario Rossi <mario.rossi@pec.example>'], cc: [], date: 1793790000, body: 'Gentile contribuente, si trasmette in allegato l’avviso.', body_html: '', attachments: [{ index: 0, name: 'Avviso.pdf', size: 2048, mime: 'application/pdf', message: false }, { index: 1, name: 'inoltro.eml', size: 900, mime: 'message/rfc822', message: true }] };
        window.isTauri = true;
        window.panelCalls = []; window.panelNotices = []; window.panelClosed = 0;
        window.__TAURI_INTERNALS__ = { invoke(command) { if (command === 'read_attached_message') return Promise.resolve(pec); return Promise.resolve(null); } };
      });
      const pg = await ctx.newPage();
      await pg.goto(`${base}/tests/attachment-panel.html`);
      await pg.waitForSelector('.attached-message', { timeout: 30000 });
      await pg.waitForTimeout(600);
      await pg.evaluate(() => document.fonts.ready);
      done('AttachedMessageView', 'Posta', await capture(pg, ['.attached-message']));
    } catch (e) {
      failed('AttachedMessageView', e);
    } finally {
      await ctx.close();
    }
  }
  return { frags, failures };
}
