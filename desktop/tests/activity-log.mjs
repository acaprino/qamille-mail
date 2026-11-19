// The pure helpers behind the activity panel and page (centro attività): the tone, the words,
// the icon and the day groups of a log row. They take the interface language and the time zone
// as arguments, so plain Node can test them without the app's i18n store (`src/lib/i18n.ts`).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import {
  entryIcon,
  entrySubtitle,
  entryTitle,
  entryTone,
  formatEntryTime,
  coalesceEntries,
  groupByDay,
  liveLabel,
} from '../src/lib/activity-log.ts';

const italian = { tr: (it) => it, backendText: (message) => message };
const english = { tr: (_it, en) => en, backendText: (message) => `[en] ${message}` };
const zone = { locale: 'it-IT', timeZone: 'UTC' };
const at = (iso) => Date.parse(iso);
const entry = (overrides = {}) => ({
  seq: 1,
  at_ms: at('2026-11-08T14:05:00Z'),
  kind: 'action',
  code: 'archive',
  source: 'user',
  account_id: null,
  subject: null,
  detail: null,
  count: null,
  ref_kind: null,
  ref_id: null,
  undo_id: null,
  outcome: 'done',
  ...overrides,
});

test('tone follows the outcome before the kind', () => {
  assert.equal(entryTone(entry({ outcome: 'failed' })), 'error');
  assert.equal(entryTone(entry({ outcome: 'undone' })), 'undone');
  assert.equal(entryTone(entry()), 'ok');
  assert.equal(entryTone(entry({ kind: 'event', code: 'syncComplete' })), 'event');
  assert.equal(entryTone(entry({ kind: 'error', code: 'syncFailed' })), 'error');
  assert.equal(entryTone(entry({ kind: 'event', outcome: 'undone' })), 'undone');
});

test('titles reuse the undo labels in both languages', () => {
  assert.equal(entryTitle(entry({ code: 'markAsDone' }), italian), 'Segnato come fatto');
  assert.equal(entryTitle(entry({ code: 'markAsDone' }), english), 'Marked as done');
  assert.equal(entryTitle(entry({ code: 'archive' }), italian), 'Archiviato');
  assert.equal(entryTitle(entry({ code: 'snooze' }), italian), 'Posticipato');
  assert.equal(entryTitle(entry({ code: 'moveToTrash' }), english), 'Moved to trash');
});

test('titles of drafts, events and errors', () => {
  assert.equal(entryTitle(entry({ code: 'send' }), italian), 'Messaggio inviato');
  assert.equal(entryTitle(entry({ code: 'discardDraft' }), italian), 'Bozza eliminata');
  assert.equal(entryTitle(entry({ code: 'scheduleDraft' }), italian), 'Invio programmato');
  assert.equal(entryTitle(entry({ kind: 'event', code: 'syncComplete' }), italian), 'Nuovi messaggi');
  assert.equal(entryTitle(entry({ kind: 'event', code: 'indexComplete' }), italian), 'Indicizzazione completata');
  assert.equal(entryTitle(entry({ kind: 'event', code: 'modelInstalled' }), italian), 'Modello di ricerca installato');
  assert.equal(entryTitle(entry({ kind: 'event', code: 'contactsComplete' }), italian), 'Contatti aggiornati');
  assert.equal(entryTitle(entry({ kind: 'error', code: 'syncFailed' }), italian), 'Sincronizzazione non riuscita');
  assert.equal(entryTitle(entry({ kind: 'error', code: 'indexFailed' }), italian), 'Indicizzazione non riuscita');
  assert.equal(entryTitle(entry({ kind: 'error', code: 'modelFailed' }), italian), 'Installazione del modello non riuscita');
  assert.equal(entryTitle(entry({ kind: 'error', code: 'contactsFailed' }), italian), 'Sincronizzazione contatti non riuscita');
});

test('a failed action is named for what failed', () => {
  assert.equal(entryTitle(entry({ code: 'send', outcome: 'failed' }), italian), 'Invio non riuscito');
  assert.equal(entryTitle(entry({ code: 'archive', outcome: 'failed' }), italian), 'Azione non riuscita');
  assert.equal(entryTitle(entry({ code: 'archive', outcome: 'failed' }), english), 'Action failed');
});

test('unknown_code_gets_generic_title', () => {
  assert.equal(entryTitle(entry({ code: 'futureThing', kind: 'event' }), italian), 'Attività');
  assert.equal(entryTitle(entry({ code: 'futureThing', kind: 'event' }), english), 'Activity');
  assert.equal(entryIcon(entry({ code: 'futureThing', kind: 'event' })), 'info');
  assert.equal(entryTone(entry({ code: 'futureThing', kind: 'event' })), 'event');
});

test('sync subtitle pluralizes the count', () => {
  const sync = (count) => entry({ kind: 'event', code: 'syncComplete', count });
  assert.equal(entrySubtitle(sync(1), italian, () => null), '1 nuovo messaggio');
  assert.equal(entrySubtitle(sync(42), italian, () => null), '42 nuovi messaggi');
  assert.equal(entrySubtitle(sync(1), english, () => null), '1 new message');
  assert.equal(entrySubtitle(sync(42), english, () => null), '42 new messages');
});

test('counts of contacts and of assistant batches', () => {
  const contacts = (count) => entry({ kind: 'event', code: 'contactsComplete', count });
  assert.equal(entrySubtitle(contacts(1), italian, () => null), '1 contatto');
  assert.equal(entrySubtitle(contacts(3), italian, () => null), '3 contatti');
  const batch = entry({ source: 'assistant', code: 'archive', count: 12 });
  assert.equal(entrySubtitle(batch, italian, () => null), '12 messaggi');
  assert.equal(entrySubtitle(batch, english, () => null), '12 messages');
});

test('the subtitle is the subject, then the account', () => {
  const row = entry({ subject: 'Cyber Resilience Act', account_id: 2 });
  assert.equal(entrySubtitle(row, italian, (id) => (id === 2 ? 'tri-tech' : null)), 'Cyber Resilience Act · tri-tech');
  assert.equal(entrySubtitle(row, italian, () => null), 'Cyber Resilience Act', 'an unknown account is left out');
  assert.equal(entrySubtitle(entry({ account_id: 2 }), italian, () => 'tri-tech'), 'tri-tech');
  assert.equal(entrySubtitle(entry(), italian, () => null), '');
});

test('an error shows its reason through the backend text', () => {
  const row = entry({ kind: 'error', code: 'syncFailed', detail: 'timeout', subject: 'ignored', account_id: 1 });
  assert.equal(entrySubtitle(row, english, () => 'gmail'), '[en] timeout · gmail');
  const failed = entry({ code: 'send', outcome: 'failed', detail: 'Server unreachable', subject: 'Re: Disdetta' });
  assert.equal(entrySubtitle(failed, italian, () => null), 'Server unreachable');
  const quiet = entry({ code: 'send', outcome: 'failed', subject: 'Re: Disdetta' });
  assert.equal(entrySubtitle(quiet, italian, () => null), 'Re: Disdetta', 'no reason: the subject');
});

test('icons follow the code, an undone row shows the undo arrow', () => {
  assert.equal(entryIcon(entry({ code: 'markAsDone' })), 'check');
  assert.equal(entryIcon(entry({ code: 'archive' })), 'archive');
  assert.equal(entryIcon(entry({ code: 'snooze' })), 'clock');
  assert.equal(entryIcon(entry({ code: 'moveToTrash' })), 'trash');
  assert.equal(entryIcon(entry({ code: 'send' })), 'send');
  assert.equal(entryIcon(entry({ kind: 'event', code: 'syncComplete' })), 'refresh');
  assert.equal(entryIcon(entry({ kind: 'event', code: 'indexComplete' })), 'search');
  assert.equal(entryIcon(entry({ kind: 'event', code: 'modelInstalled' })), 'download');
  assert.equal(entryIcon(entry({ kind: 'event', code: 'contactsComplete' })), 'contacts');
  assert.equal(entryIcon(entry({ outcome: 'undone' })), 'reply');
  assert.equal(entryIcon(entry({ kind: 'error', code: 'syncFailed' })), 'info');
  assert.equal(entryIcon(entry({ code: 'send', outcome: 'failed' })), 'info');
});

test('time shows HH:MM today and day month before', () => {
  const now = at('2026-11-08T18:00:00Z');
  assert.equal(formatEntryTime(at('2026-11-08T14:05:00Z'), now, zone), '14:05');
  assert.match(formatEntryTime(at('2026-11-07T14:05:00Z'), now, zone), /^7 nov/);
  assert.match(formatEntryTime(at('2026-10-02T09:00:00Z'), now, zone), /^2 ott/);
});

test('groups split at midnight of the given zone', () => {
  const now = at('2026-11-08T00:30:00Z');
  const rows = [
    entry({ seq: 5, at_ms: at('2026-11-08T00:10:00Z') }),
    entry({ seq: 4, at_ms: at('2026-11-07T23:50:00Z') }),
    entry({ seq: 3, at_ms: at('2026-11-07T08:00:00Z') }),
    entry({ seq: 2, at_ms: at('2026-11-05T12:00:00Z') }),
    entry({ seq: 1, at_ms: at('2025-12-24T12:00:00Z') }),
  ];
  const groups = groupByDay(rows, now, italian, zone);
  assert.deepEqual(groups.map((group) => group.label.split(' ')[0]), ['Oggi', 'Ieri', '5', '24']);
  assert.deepEqual(groups.map((group) => group.entries.map((row) => row.seq)), [[5], [4, 3], [2], [1]]);
  assert.match(groups[2].label, /^5 nov(embre)?$/, 'a day of this year has no year');
  assert.match(groups[3].label, /^24 dic(embre)? 2025$/, 'a day of another year names it');
});

test('groups follow the time zone, not UTC', () => {
  // 23:30 UTC on the 7th is already the 8th at 00:30 in Rome (UTC+1 in November).
  const rome = { locale: 'it-IT', timeZone: 'Europe/Rome' };
  const now = at('2026-11-08T10:00:00Z');
  const groups = groupByDay([entry({ at_ms: at('2026-11-07T23:30:00Z') })], now, italian, rome);
  assert.equal(groups[0].label, 'Oggi');
});

test('no rows, no groups', () => {
  assert.deepEqual(groupByDay([], Date.now(), italian, zone), []);
});

test('live work is worded with its percent when it is known', () => {
  const live = (kind, percent = null, phase = '') => ({
    id: kind,
    kind,
    accountId: null,
    phase,
    percent,
    since: 0,
    last: 0,
  });
  assert.equal(liveLabel(live('sync'), italian), 'Sincronizzazione…');
  assert.equal(liveLabel(live('sync'), english), 'Syncing…');
  assert.equal(liveLabel(live('index', 40), italian), 'Indicizzazione 40%');
  assert.equal(liveLabel(live('index'), english), 'Indexing…');
  assert.equal(liveLabel(live('model', 7), english), 'Installing model 7%');
  assert.equal(liveLabel(live('model'), italian), 'Installazione modello…');
  assert.equal(liveLabel(live('contacts'), italian), 'Sincronizzazione contatti…');
  assert.equal(liveLabel(live('confirming', null, '1'), english), 'Confirming 1 action…');
  assert.equal(liveLabel(live('confirming', null, '3'), italian), 'Conferma 3 azioni…');
  assert.equal(liveLabel(live('history', null, '12400/48300'), italian), 'Recupero della posta precedente · 12.400 di 48.300');
  assert.equal(liveLabel(live('history', null, '12400/48300'), english), 'Bringing in older mail · 12,400 of 48,300');
});

// ---- bulk actions are one row (review I1) ----
const bulkRow = (seq, secondsAgo, overrides = {}) =>
  entry({ seq, at_ms: at('2026-11-08T14:00:00Z') - secondsAgo * 1000, account_id: 1, ...overrides });

test('a bulk of the same action within seconds is one row with the total', () => {
  const rows = [bulkRow(5, 0), bulkRow(4, 1), bulkRow(3, 2), bulkRow(2, 2), bulkRow(1, 4)];
  const merged = coalesceEntries(rows);
  assert.equal(merged.length, 1);
  assert.equal(merged[0].count, 5);
  assert.equal(merged[0].seq, 5, 'the newest row names the group');
  assert.equal(merged[0].at_ms, rows[0].at_ms);
  assert.deepEqual([merged[0].subject, merged[0].ref_kind, merged[0].ref_id, merged[0].undo_id], [null, null, null, null],
    'five messages point at none of them');
  assert.equal(entrySubtitle(merged[0], italian, () => null), '5 messaggi');
  assert.equal(entryTitle(merged[0], italian), 'Archiviato');
});

test('a pause longer than the window starts a new row', () => {
  const merged = coalesceEntries([bulkRow(4, 0), bulkRow(3, 1), bulkRow(2, 60), bulkRow(1, 61)]);
  assert.deepEqual(merged.map((row) => [row.seq, row.count]), [[4, 2], [2, 2]]);
});

test('rows that differ in code, source, outcome, account or kind are not merged', () => {
  const rows = [
    bulkRow(7, 0),
    bulkRow(6, 1, { code: 'snooze' }),
    bulkRow(5, 2, { source: 'assistant' }),
    bulkRow(4, 3, { outcome: 'failed' }),
    bulkRow(3, 4, { account_id: 2 }),
    bulkRow(2, 5, { kind: 'event', code: 'syncComplete' }),
    bulkRow(1, 6, { kind: 'event', code: 'syncComplete' }),
  ];
  assert.deepEqual(coalesceEntries(rows).map((row) => row.seq), [7, 6, 5, 4, 3, 2, 1], 'events are never merged either');
});

test('assistant batches add their own counts', () => {
  const rows = [bulkRow(2, 0, { source: 'assistant', count: 12 }), bulkRow(1, 3, { source: 'assistant', count: 3 })];
  assert.equal(coalesceEntries(rows)[0].count, 15);
});

test('a single row comes back untouched and the input is never changed', () => {
  const one = bulkRow(1, 0, { subject: 'Hello', ref_kind: 'email', ref_id: 3 });
  assert.equal(coalesceEntries([one])[0], one, 'the very same object');
  const rows = [bulkRow(3, 0), bulkRow(2, 1)];
  const frozen = structuredClone(rows);
  coalesceEntries(rows);
  assert.deepEqual(rows, frozen);
  assert.deepEqual(coalesceEntries([]), []);
});

// ---- sample data is invented (review M7) ----
test('the preview history holds invented text, nothing from a real mailbox', () => {
  const source = fs.readFileSync(new URL('../src/lib/preview.ts', import.meta.url), 'utf8');
  for (const real of ['5587053718', 'Cyber Resilience', 'Token Plan', 'Disdetta contratto', 'Flash Sale'])
    assert.ok(!source.includes(real), `preview.ts must not hold "${real}"`);
});
