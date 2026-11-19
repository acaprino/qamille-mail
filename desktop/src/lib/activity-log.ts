// The words, tone, icon and day groups of a row of the activity log (centro attività), shared by the
// sidebar panel and the Activity page. Pure: the interface language and the time zone come in as
// arguments (`Words`, `Zone`) because `i18n.ts` uses runes and plain Node cannot load it;
// the components pass `{ tr, backendText }` and the regional settings.
import type { Activity } from './activity.ts';
import type { ActivityEntry } from './types';
import { mailActionLabels } from './undo-labels.ts';

/** The two language helpers of `i18n.ts` the helpers below need. */
export interface Words {
  tr(it: string, en: string): string;
  backendText(message: string): string;
}
/** The locale and time zone dates are shown in (the regional settings); `undefined` = the device's. */
export interface Zone {
  locale: string | undefined;
  timeZone: string | undefined;
}
/** `error` also covers an action that failed; `undone` a row the undo window took back. */
export type ActivityTone = 'ok' | 'undone' | 'error' | 'event';

export const isFailure = (entry: ActivityEntry): boolean =>
  entry.kind === 'error' || entry.outcome === 'failed';

export function entryTone(entry: ActivityEntry): ActivityTone {
  if (isFailure(entry)) return 'error';
  if (entry.outcome === 'undone') return 'undone';
  return entry.kind === 'event' ? 'event' : 'ok';
}

/** Titles of the codes that are not a mail action, [Italian, English]. */
const titles: Record<string, [string, string]> = {
  send: ['Messaggio inviato', 'Message sent'],
  discardDraft: ['Bozza eliminata', 'Draft discarded'],
  scheduleDraft: ['Invio programmato', 'Send scheduled'],
  syncComplete: ['Nuovi messaggi', 'New messages'],
  indexComplete: ['Indicizzazione completata', 'Indexing finished'],
  modelInstalled: ['Modello di ricerca installato', 'Search model installed'],
  contactsComplete: ['Contatti aggiornati', 'Contacts updated'],
  syncFailed: ['Sincronizzazione non riuscita', 'Sync failed'],
  indexFailed: ['Indicizzazione non riuscita', 'Indexing failed'],
  modelFailed: ['Installazione del modello non riuscita', 'Model installation failed'],
  contactsFailed: ['Sincronizzazione contatti non riuscita', 'Contacts sync failed'],
};

export function entryTitle(entry: ActivityEntry, words: Words): string {
  if (entry.outcome === 'failed') {
    return entry.code === 'send'
      ? words.tr('Invio non riuscito', 'Sending failed')
      : words.tr('Azione non riuscita', 'Action failed');
  }
  const label = titles[entry.code] ?? mailActionLabels[entry.code];
  // A code from a newer release is not an error: a generic title, never an exception.
  return label ? words.tr(label[0], label[1]) : words.tr('Attività', 'Activity');
}

/** "12 messaggi", "1 nuovo messaggio": the count of a row that carries one, or null. */
function countText(entry: ActivityEntry, words: Words): string | null {
  const count = entry.count;
  if (count === null) return null;
  const one = count === 1;
  if (entry.code === 'syncComplete')
    return one
      ? words.tr('1 nuovo messaggio', '1 new message')
      : words.tr(`${count} nuovi messaggi`, `${count} new messages`);
  if (entry.code === 'contactsComplete')
    return one
      ? words.tr('1 contatto', '1 contact')
      : words.tr(`${count} contatti`, `${count} contacts`);
  return one ? words.tr('1 messaggio', '1 message') : words.tr(`${count} messaggi`, `${count} messages`);
}

/** The second line of a row: what it is about (or why it failed), then the account. */
export function entrySubtitle(
  entry: ActivityEntry,
  words: Words,
  accountName: (id: number) => string | null,
): string {
  const main = isFailure(entry) && entry.detail
    ? words.backendText(entry.detail)
    : (countText(entry, words) ?? entry.subject ?? '');
  const account = entry.account_id === null ? null : accountName(entry.account_id);
  return [main, account].filter(Boolean).join(' · ');
}

/** `Icon` primitive names, one per code; a failure or an unknown code shows `info`. */
const icons: Record<string, string> = {
  markAsDone: 'check',
  archive: 'archive',
  snooze: 'clock',
  moveToTrash: 'trash',
  send: 'send',
  discardDraft: 'trash',
  scheduleDraft: 'clock',
  syncComplete: 'refresh',
  indexComplete: 'search',
  modelInstalled: 'download',
  contactsComplete: 'contacts',
};

export function entryIcon(entry: ActivityEntry): string {
  if (isFailure(entry)) return 'info';
  if (entry.outcome === 'undone') return 'reply';
  return icons[entry.code] ?? 'info';
}

/** One short line for live work (sync, indexing, model, contacts, confirmations); the percent is part of it when known. */
export function liveLabel(activity: Activity, words: Words): string {
  const { tr } = words;
  switch (activity.kind) {
    case 'sync':
      return tr('Sincronizzazione…', 'Syncing…');
    case 'index':
      if (activity.phase === 'waiting')
        return tr('Indicizzazione: in attesa del server di calcolo', 'Indexing: waiting for the compute server');
      return activity.percent === null
        ? tr('Indicizzazione…', 'Indexing…')
        : tr(`Indicizzazione ${activity.percent}%`, `Indexing ${activity.percent}%`);
    case 'history': {
      const [fetched, total] = activity.phase.split('/').map(Number);
      const it = tr('it', 'en') === 'it';
      const count = (n: number) => (Number.isFinite(n) ? n.toLocaleString(it ? 'it-IT' : 'en-US') : '…');
      return tr(
        `Recupero della posta precedente · ${count(fetched)} di ${count(total)}`,
        `Bringing in older mail · ${count(fetched)} of ${count(total)}`,
      );
    }
    case 'model':
      return activity.percent === null
        ? tr('Installazione modello…', 'Installing model…')
        : tr(`Installazione modello ${activity.percent}%`, `Installing model ${activity.percent}%`);
    case 'contacts':
      return tr('Sincronizzazione contatti…', 'Syncing contacts…');
    case 'confirming': {
      const count = Number(activity.phase);
      return count === 1
        ? tr('Conferma 1 azione…', 'Confirming 1 action…')
        : tr(`Conferma ${count} azioni…`, `Confirming ${count} actions…`);
    }
  }
}

/** Rows of one action this close in time are one bulk action (a selection of 50 messages archived). */
export const coalesceWindowMs = 15_000;

/**
 * Merges the runs of the same user or assistant action done within `coalesceWindowMs` of each other
 * into one row with the total count: the app queues one action per message, and fifty identical rows
 * would bury the rest of the history. Newest first, as the rows come; the merged row takes the
 * newest row's place and time and points at none of the messages. Failures merge only with failures,
 * undone rows only with undone rows, and events, errors and rows of other accounts or sources never merge.
 */
export function coalesceEntries(entries: ActivityEntry[]): ActivityEntry[] {
  const merged: ActivityEntry[] = [];
  let lastAt = 0;
  for (const entry of entries) {
    const head = merged[merged.length - 1];
    const joins =
      head !== undefined &&
      entry.kind === 'action' &&
      head.kind === 'action' &&
      head.code === entry.code &&
      head.source === entry.source &&
      head.outcome === entry.outcome &&
      head.account_id === entry.account_id &&
      lastAt - entry.at_ms <= coalesceWindowMs;
    if (!joins) {
      merged.push(entry);
    } else {
      merged[merged.length - 1] = {
        ...head,
        count: (head.count ?? 1) + (entry.count ?? 1),
        subject: null,
        detail: head.detail,
        ref_kind: null,
        ref_id: null,
        undo_id: null,
      };
    }
    lastAt = entry.at_ms;
  }
  return merged;
}

/** The calendar day of an instant in a zone, as `YYYY-MM-DD` (sorts and compares as text). */
function dayKey(atMs: number, zone: Zone): string {
  return new Intl.DateTimeFormat('en-CA', {
    timeZone: zone.timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(atMs);
}

/** `HH:MM` for a row of today, `d mmm` for an earlier day (the year too when it is another year). */
export function formatEntryTime(atMs: number, now: number, zone: Zone): string {
  if (dayKey(atMs, zone) === dayKey(now, zone))
    return new Intl.DateTimeFormat(zone.locale, {
      timeZone: zone.timeZone,
      hour: '2-digit',
      minute: '2-digit',
      hourCycle: 'h23',
    }).format(atMs);
  return dayLabel(atMs, now, zone, 'short');
}

function dayLabel(atMs: number, now: number, zone: Zone, month: 'short' | 'long'): string {
  const sameYear = dayKey(atMs, zone).slice(0, 4) === dayKey(now, zone).slice(0, 4);
  return new Intl.DateTimeFormat(zone.locale, {
    timeZone: zone.timeZone,
    day: 'numeric',
    month,
    ...(sameYear ? {} : { year: 'numeric' }),
  }).format(atMs);
}

/** The key of the day before `key`, by calendar arithmetic (never "24 hours ago": DST days). */
function previousDay(key: string): string {
  const [year, month, day] = key.split('-').map(Number);
  return new Date(Date.UTC(year, month - 1, day - 1)).toISOString().slice(0, 10);
}

/** Rows grouped by calendar day, newest first as given: "Oggi", "Ieri", then the date. */
export function groupByDay(
  entries: ActivityEntry[],
  now: number,
  words: Words,
  zone: Zone,
): { label: string; entries: ActivityEntry[] }[] {
  const today = dayKey(now, zone);
  const yesterday = previousDay(today);
  const groups: { key: string; label: string; entries: ActivityEntry[] }[] = [];
  for (const entry of entries) {
    const key = dayKey(entry.at_ms, zone);
    const last = groups[groups.length - 1];
    if (last && last.key === key) {
      last.entries.push(entry);
      continue;
    }
    const label =
      key === today
        ? words.tr('Oggi', 'Today')
        : key === yesterday
          ? words.tr('Ieri', 'Yesterday')
          : dayLabel(entry.at_ms, now, zone, 'long');
    groups.push({ key, label, entries: [entry] });
  }
  return groups.map(({ label, entries }) => ({ label, entries }));
}
