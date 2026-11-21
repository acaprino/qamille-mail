// Browser-only adapter. It never connects to Gmail or reads local user data.
// Its sample fixture is exported from the Rust domain, not a second product store.
import fixture from './sample.json';
import { downloadSample, sampleThread } from './preview-export';
import { REPLY_CAP, conversationIds, groupConversations, replyRows } from './conversations';
import { sampleSearch, sampleSnippet } from './preview-search';
import { previewContacts } from './preview-contacts';
import { meetingEngine } from './meeting-engine';
import { applyPreviewStorage, initializePreviewStorage, previewStorageStatus } from './preview-storage';
import {
  configureRegional,
  fullDate,
  regionalLocale,
  regionalTimeZone,
  regionalWarning,
} from './regional';
import type {
  ActivityEntry,
  ActivityFilter,
  AppEvent,
  MailRow,
  Store,
  Email,
  Draft,
  CalendarEvent,
  Signature,
  UndoAction,
  UndoOperation,
  AccountStoragePolicy,
} from './types';
declare global {
  interface Window {
    /** Command names handled by the browser preview, in call order; tests count backend calls. */
    __previewCalls?: string[];
    /** `offset` and requested `limit` of every `list_mail` call, before the backend clamp. */
    __previewListQueries?: { offset: number; limit: number }[];
    /** Test hook: milliseconds `list_mail` waits before answering. */
    __previewDelay?: number;
    /**
     * Test hook: milliseconds each named command waits before answering, e.g. `{ read_mail: 1500 }`.
     * The reply is built before the wait, like a slow backend. A list is consumed one entry per
     * call (`{ read_mail: [900, 0] }`: the first call is slow, the next ones are not).
     */
    __previewCommandDelay?: Record<string, number | number[]>;
    /** Test hook: each named command rejects with this message, after its delay. */
    __previewCommandFail?: Record<string, string>;
    /**
     * Test hook: milliseconds an action stays `committing` after its undo window closes, like a
     * confirmation waiting behind a sync; the action is then applied and leaves the list.
     */
    __previewCommitHold?: number;
    /**
     * Test hook: the server holds `total` older messages matching any search. Searches then say
     * their results are partial and «Cerca sul server» brings them in pages of 50.
     */
    __previewRemote?: { total: number };
    /**
     * Test hook: every Gmail-like account has a history walk in this state, over `total` messages
     * of All Mail. Settings → Search then shows the coverage and «Include Spam and Trash».
     */
    __previewHistory?: { state: 'running' | 'complete' | 'space'; total: number };
    /**
     * Test hook for the link to a Qamille server (access wave 2): the edition of the build, what
     * the server answers to the probe and to a sign-in, and whether it is offline afterwards.
     */
    __previewServer?: {
      edition?: 'self-hosted' | 'professional';
      probe?: 'found' | 'unreachable' | 'not_qamille' | 'incompatible';
      login?: 'ok' | 'wrong' | 'temporary' | 'limit';
      offline?: boolean;
      /** Gateway LLM (wave 5): the server declares an assistant, internal or forwarding to `llm.label`. */
      llm?: { external: boolean; label: string };
      /** The server changed its assistant's provider after the person agreed. */
      llmChanged?: boolean;
      /**
       * Mailbox synchronization (wave 7): the server declares it; `removals` are mailboxes
       * «removed on another computer»; `revoke` is what Google answers to a revocation;
       * `paused` is a server that asks for a subscription.
       */
      sync?: { available: boolean; removals?: string[]; revoke?: 'ok' | 'failed'; paused?: boolean; gmail?: boolean };
      /** What `GET /v1/info` says (access wave 3); the sign-up code is 12345678. */
      signup?: boolean;
      password_reset?: boolean;
      /** Compute wave 3: the server answers `401` after the sign-in, until «Sign in again». */
      unlinked?: boolean;
      /** Compute wave 3 (D4): the server serves another model than the index, until «Reindex». */
      mismatch?: boolean;
      /** «Vendita Professional»: the server offers «Continue with Google». */
      google?: boolean;
      /** What the Google sign-in finds: an account (default) or none. */
      googleAnswer?: 'ok' | 'no_account';
      googleUser?: string;
      /** The server sells subscriptions (fake processor); a Google sign-up gets a trial unless `trial: false`. */
      billing?: boolean;
      /** Wave 9: the plans carry monthly quotas and the account carries the consumption; `full` fills the named meters. */
      quotas?: { full?: ('audio_minutes' | 'embedded_tokens' | 'llm_tokens')[] };
      /** Wave 9: the server declares its Terms; a new account (sign-up or Google) must send `terms_version`. */
      legal?: { terms_url?: string; privacy_url?: string; terms_version?: string };
      trial?: boolean;
      /** The fake payment page declines the card. */
      declined?: boolean;
      /** Wave 2: the preview starts signed in to a server, in `server` mode. */
      connected?: boolean;
      /** Wave 2: a compute mode saved by an older version (`local`, `remote`); it reads as `off` or `server`. */
      savedMode?: 'local' | 'remote' | 'off' | 'server';
      /** Wave 2: a Qamille server answers on this same PC (`http://127.0.0.1:8787`). */
      local?: boolean;
    };
    /** Test hook (wave 2): what the old local mode left; `legacy_local_status` answers from it. */
    __previewLegacy?: { models_bytes: number; had_server: boolean; dismissed?: boolean };
    /** Test hook (wave 2): the `confirmed` argument of the last `legacy_local_free_space`. */
    __legacyFreed?: boolean;
    /** Test hook: appends `count` read Inbox messages so list paging can be exercised. */
    __previewSeed?: (count: number) => void;
    /** `ids` of every `read_rows` call, before the backend clamp. */
    __previewReadRows?: number[][];
    /** Test hook: keys `read_rows` leaves out of every row, e.g. `['snoozed_until']`. */
    __previewReadRowsOmit?: string[];
    /**
     * Test hook: changes fields of a stored message without announcing it, like a write another
     * process made; a test then sends the matching `app-event`.
     */
    __previewEdit?: (id: number, fields: Partial<Email>) => void;
    /**
     * Test hook: announces a typed event exactly like the preview backend does, so tests can
     * simulate the events this adapter never produces itself (`sync-progress`, `index-progress`,
     * `model-progress`, `contacts-sync`).
     */
    __previewAnnounce?: (event: AppEvent) => void;
  }
}
const store = structuredClone(fixture) as unknown as Store;
store.undo_actions = [];
store.settings.undo_seconds = 5;
store.settings.time_zone ??= '';
store.settings.locale ??= '';
store.settings.meeting_reminders ??= true;
store.settings.reminder_lead_minutes ??= 1;
store.settings.meeting_language ??= 'it';
store.settings.meeting_engine = meetingEngine(store.settings.meeting_engine);
store.settings.meeting_model ??= 'base';
store.settings.meeting_remote_model ??= '';
store.settings.meeting_microphone ??= '';
store.settings.meeting_system_audio ??= '';
initializePreviewStorage(store);
const now = () => Math.floor(Date.now() / 1000);
const section = (e: Email) =>
  e.pinned
    ? 'Pins'
    : e.read
      ? 'Seen'
      : { Priority: 'Priority', Notification: 'Notifications', Newsletter: 'Newsletters' }[
          e.category
        ] || 'New';
const summary = (e: Email) => ({
  ...e,
  message_ids: [e.id],
  snippet: e.body.replace(/\s+/g, ' ').slice(0, 140),
  section: section(e),
  attachment_count: e.attachments.length,
});
// One list row, as `list_mail` answers it (EmailSummary::from in mail.rs).
const listRow = (email: Email, searching = false, terms: string[] = []) => ({
  ...summary(email),
  date: searching ? email.date : (email.returned_at ?? email.date),
  date_missing: !!email.date_missing,
  match_terms: terms,
  ...(searching ? sampleSnippet(email, terms) : {}),
});
// One row as `read_rows` answers it: the summary fields only (never the whole Email) plus the
// flags that decide Inbox / Tasks / Snoozed, explicit `null` / `false` when unset. `date` is the
// inbox date, like EmailSummary::from. The hook drops keys to play a backend that breaks the
// contract (see MailRow in types.ts).
const readRow = (e: Email): MailRow => {
  const row: Record<string, unknown> = {
    folder: e.folder,
    date_missing: !!e.date_missing,
    match_reason: '',
    match_terms: [],
    id: e.id,
    message_ids: [e.id],
    account_id: e.account_id,
    from_name: e.from_name,
    from_addr: e.from_addr,
    subject: e.subject,
    snippet: e.body.replace(/\s+/g, ' ').slice(0, 140),
    date: e.returned_at ?? e.date,
    read: e.read,
    pinned: e.pinned,
    done: e.done,
    category: e.category,
    section: section(e),
    labels: [...e.labels],
    attachment_count: e.attachments.length,
    due: e.due ?? null,
    snoozed_until: e.snoozed_until ?? null,
    set_aside: !!e.set_aside,
    reminder_at: e.reminder_at ?? null,
  };
  for (const key of window.__previewReadRowsOmit ?? []) delete row[key];
  return row as unknown as MailRow;
};
const inbox = (e: Email) =>
  e.folder === 'Inbox' && !e.done && !(e.snoozed_until && e.snoozed_until > now());
const labels = () => [...new Set(store.emails.flatMap((e) => e.labels))].sort();

function applyMail(target: Store, id: number, action: string, value?: string | null) {
  const e = target.emails.find((e) => e.id === id);
  if (!e) throw new Error('Message not found');
  switch (action) {
    case 'pin':
      e.pinned = true;
      break;
    case 'unpin':
      e.pinned = false;
      break;
    case 'markAsSeen':
      e.read = true;
      break;
    case 'markAsUnseen':
      e.read = false;
      break;
    case 'archive':
      e.folder = 'Archive';
      break;
    case 'moveToInbox':
      e.folder = 'Inbox';
      e.done = false;
      break;
    case 'moveToTrash':
      e.folder = 'Trash';
      break;
    case 'markAsDone':
      e.done = true;
      e.folder = 'Archive';
      e.pinned = false;
      e.set_aside = false;
      e.read = true;
      break;
    case 'markAsUndone':
      e.done = false;
      break;
    case 'setAside':
      e.set_aside = !e.set_aside;
      break;
    case 'snooze':
      e.snoozed_until = now() + 86400;
      break;
    case 'unsnooze':
      e.snoozed_until = null;
      break;
    case 'attachLabel':
      if (value && !e.labels.includes(value)) e.labels.push(value);
      break;
    case 'detachLabel':
      e.labels = e.labels.filter((l) => l !== value);
      break;
    default:
      throw new Error('This action is available in the desktop app.');
  }
}
function projected(): Store {
  const view = structuredClone(store);
  for (const pending of view.undo_actions) {
    if (pending.status === 'failed') continue;
    const op = pending.operation;
    if (op.kind === 'mail') applyMail(view, op.email_id, op.action, op.value);
    else view.drafts = view.drafts.filter((d) => d.id !== op.draft_id);
  }
  return view;
}
// The activity log of the browser preview: seven rows that cover every tone (done, undone, error,
// event, the assistant), kept in memory like the rest of this adapter. Rows of "today" are clamped
// to after midnight so the day groups look the same at any hour.
const startOfToday = new Date().setHours(0, 0, 0, 0);
const hoursAgo = (hours: number) => Date.now() - hours * 3_600_000;
const today = (minutesAgo: number, order: number) =>
  Math.max(Date.now() - minutesAgo * 60_000, startOfToday + order * 1000);
const activityRows: ActivityEntry[] = (
  [
    { at_ms: startOfToday - 3_600_000 * 4, kind: 'event', code: 'syncComplete', source: 'system', account_id: 1, count: 42 },
    { at_ms: startOfToday - 3_600_000 * 3, kind: 'action', code: 'archive', source: 'user', account_id: 1, subject: 'Quarterly report: draft for your review', ref_kind: 'email', ref_id: 3 },
    { at_ms: today(180, 1), kind: 'action', code: 'send', source: 'user', account_id: 1, subject: 'Re: Contract renewal', detail: 'Server unreachable', outcome: 'failed' },
    // A selection of four messages moved to the trash: the app queues one action per message.
    ...[0, 1, 2, 3].map((step) => ({
      at_ms: today(150, 2) + step * 1000,
      kind: 'action' as const,
      code: 'moveToTrash',
      source: 'user' as const,
      account_id: 1,
      subject: `Old newsletter ${step + 1}`,
      ref_kind: 'email',
      ref_id: 100 + step,
    })),
    { at_ms: today(120, 2), kind: 'action', code: 'archive', source: 'assistant', account_id: 1, count: 12 },
    { at_ms: today(40, 3), kind: 'action', code: 'snooze', source: 'user', account_id: 1, subject: 'Venue booking: change to your reservation', outcome: 'undone' },
    { at_ms: today(12, 4), kind: 'event', code: 'indexComplete', source: 'system', account_id: 2 },
    { at_ms: today(5, 5), kind: 'action', code: 'markAsDone', source: 'user', account_id: 1, subject: 'Spring sale: 20% off this week' },
  ] as Partial<ActivityEntry>[]
).map((row, at) => ({
  seq: at + 1,
  kind: 'event',
  code: '',
  source: 'user',
  account_id: null,
  subject: null,
  detail: null,
  count: null,
  ref_kind: null,
  ref_id: null,
  undo_id: null,
  outcome: 'done',
  ...row,
  at_ms: row.at_ms ?? hoursAgo(1),
})) as ActivityEntry[];
let activitySeenMs = 0;
const activityIsError = (row: ActivityEntry) => row.kind === 'error' || row.outcome === 'failed';
const activityMatches = (row: ActivityEntry, filter: ActivityFilter) =>
  filter === 'all' ||
  (filter === 'error' ? activityIsError(row) : row.kind === filter);
function queue(operation: UndoOperation): UndoAction {
  const view = projected();
  if (operation.kind === 'mail')
    applyMail(view, operation.email_id, operation.action, operation.value);
  const target =
    operation.kind === 'mail'
      ? view.emails.find((e) => e.id === operation.email_id)
      : view.drafts.find((d) => d.id === operation.draft_id);
  if (!target) throw new Error('Message not found or already pending');
  const created_at_ms = Date.now();
  const pending: UndoAction = {
    id: ++store.next_id,
    operation,
    subject: target.subject,
    created_at_ms,
    expires_at_ms: created_at_ms + store.settings.undo_seconds * 1000,
    status: 'pending',
    error: null,
  };
  store.undo_actions.push(pending);
  setTimeout(() => commitPreview(pending.id), store.settings.undo_seconds * 1000);
  return structuredClone(pending);
}
// The desktop backend announces what it changed with typed events (app/src/application/events.rs);
// the preview sends the same ones as DOM events, with the same `kind` and fields.
function announce(...events: AppEvent[]) {
  for (const detail of events) window.dispatchEvent(new CustomEvent('app-event', { detail }));
}
window.__previewAnnounce = (event) => announce(event);
function commitPreview(id: number) {
  const pending = store.undo_actions.find((a) => a.id === id);
  if (!pending) return;
  if (window.__previewCommitHold && pending.status === 'pending') {
    pending.status = 'committing';
    announce({ kind: 'undo-changed', action: structuredClone(pending) });
    setTimeout(() => commitPreview(id), window.__previewCommitHold);
    return;
  }
  const op = pending.operation;
  // Preserve command order for several actions on the same message.
  if (
    store.undo_actions.some(
      (a) =>
        a.id < id &&
        a.status !== 'failed' &&
        a.operation.kind === 'mail' &&
        op.kind === 'mail' &&
        a.operation.email_id === op.email_id,
    )
  ) {
    setTimeout(() => commitPreview(id), 100);
    return;
  }
  const account_id =
    (op.kind === 'mail'
      ? store.emails.find((e) => e.id === op.email_id)?.account_id
      : store.drafts.find((d) => d.id === op.draft_id)?.account_id) ?? store.accounts[0].id;
  // What the commit changed on screen: the message or draft, and the counters.
  const touched: AppEvent[] = [
    op.kind === 'mail'
      ? { kind: 'email-changed', account_id, ids: [op.email_id] }
      : { kind: 'drafts-changed', ids: [op.draft_id] },
    { kind: 'counts-changed', account_id },
  ];
  try {
    if (op.kind === 'mail') applyMail(store, op.email_id, op.action, op.value);
    else if (op.kind === 'discard_draft')
      store.drafts = store.drafts.filter((d) => d.id !== op.draft_id);
    else if (op.kind === 'schedule_draft') {
      const draft = store.drafts.find((d) => d.id === op.draft_id);
      if (draft) draft.send_at = op.send_at;
    } else throw new Error('Browser preview cannot send mail. Open the desktop app to send.');
    store.undo_actions = store.undo_actions.filter((a) => a.id !== id);
  } catch (error) {
    // A failed action no longer hides its message: it comes back, and the notice says why.
    pending.status = 'failed';
    pending.error = String(error);
    announce({ kind: 'undo-changed', action: structuredClone(pending) }, ...touched);
    return;
  }
  announce({ kind: 'undo-removed', id }, ...touched);
}
window.__previewSeed = (count) => {
  const template = store.emails.find((e) => e.folder === 'Inbox');
  if (!template) return;
  for (let i = 0; i < count; i++) {
    const id = ++store.next_id;
    store.emails.push({
      ...structuredClone(template),
      id,
      thread_id: `seed-${id}`,
      message_id: `<seed-${id}@qamille.local>`,
      subject: `Seed message ${id}`,
      date: template.date - 1 - i,
      read: true,
      pinned: false,
      due: null,
      attachments: [],
      labels: [],
    });
  }
  window.dispatchEvent(new Event('preview-workspace-changed'));
};
// The server search of the preview: which query it answered and how many results came in.
const remoteSearch = { query: '', shown: 0 };
window.__previewEdit = (id, fields) => {
  const email = store.emails.find((e) => e.id === id);
  if (email) Object.assign(email, fields);
};

export async function previewCommand(
  command: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  (window.__previewCalls ??= []).push(command);
  const view = projected();
  const wait = window.__previewCommandDelay?.[command];
  const delay = Array.isArray(wait) ? wait.shift() : wait;
  if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
  const failure = window.__previewCommandFail?.[command];
  if (failure) throw new Error(failure);
  configureRegional(view.settings);
  const contacts = previewContacts(command, args, view.accounts, view.emails);
  if (contacts.handled) return contacts.value;
  switch (command) {
    case 'get_account_storage':
      return previewStorageStatus(store, Number(args.accountId));
    case 'update_account_storage': {
      const status = applyPreviewStorage(store, Number(args.accountId), args.policy as AccountStoragePolicy, args.confirmCleanup === true);
      window.dispatchEvent(new Event('preview-workspace-changed'));
      return status;
    }
    case 'workspace_snapshot':
      return structuredClone({
        ...view,
        regional: {
          time_zone: regionalTimeZone(),
          locale: regionalLocale(),
          warning: regionalWarning(),
        },
        labels: labels(),
        has_ai_key: false,
        has_google_secret: false,
        counts: {
          Inbox: view.emails.filter((e) => inbox(e) && !e.read).length,
          Tasks: view.emails.filter((e) => (e.pinned || e.due || e.set_aside) && !e.done).length,
          Snoozed: view.emails.filter((e) => e.snoozed_until && e.snoozed_until > now()).length,
          Drafts: view.drafts.length,
          Archive: view.emails.filter((e) => e.folder === 'Archive').length,
          Trash: view.emails.filter((e) => e.folder === 'Trash').length,
        },
      });
    case 'list_mail': {
      const q = args.query as {
        folder: string;
        search: string;
        account_id: number | null;
        offset: number;
        limit: number;
        threads?: boolean;
        sort?: string;
      };
      (window.__previewListQueries ??= []).push({ offset: q.offset, limit: q.limit });
      if (window.__previewDelay)
        await new Promise((resolve) => setTimeout(resolve, window.__previewDelay));
      const searching = !!q.search.trim();
      let terms: string[] = [];
      let emails = view.emails.filter((e) => !q.account_id || e.account_id === q.account_id);
      if (searching) {
        const result = sampleSearch(view, q.search, q.account_id, q.sort);
        emails = result.emails;
        terms = result.terms;
      } else
        emails = emails.filter((e) =>
          q.folder === 'Tasks'
            ? (e.pinned || e.due || e.set_aside) && !e.done
            : q.folder === 'Snoozed'
              ? e.snoozed_until && e.snoozed_until > now()
              : q.folder.startsWith('label:')
                ? e.labels.includes(q.folder.slice(6))
                : q.folder === 'Inbox'
                  ? inbox(e)
                  : e.folder === q.folder,
        );
      if (!searching)
        emails.sort((a, b) => (b.returned_at ?? b.date) - (a.returned_at ?? a.date) || b.id - a.id);
      const summaries = emails.map((email) => listRow(email, searching, terms));
      const items = q.threads ? groupConversations(summaries, view.emails) : summaries;
      // Like the Rust backend (mail.rs): 0 means 60, anything else is clamped to 1..=100.
      const end = q.offset + (q.limit ? Math.min(Math.max(q.limit, 1), 100) : 60);
      // The Inbox headline, like the backend: counted over the whole view (the account filter
      // included), not over the page; nothing for other folders and for a search.
      const counted = !searching && q.folder === 'Inbox';
      // The other messages of each conversation hang under the rows of this page only, and a
      // search stays a flat list of matches. Empty values are left out, like the backend's JSON.
      const withReplies = (item: (typeof items)[number]) => {
        const rows = replyRows(view.emails, item.id, q.folder);
        return {
          ...item,
          conversation_id: conversationIds(view.emails).get(item.id) ?? item.id,
          ...(rows.length ? { replies: rows.slice(0, REPLY_CAP) } : {}),
          ...(rows.length > REPLY_CAP ? { replies_more: rows.length - REPLY_CAP } : {}),
          // An unread reply beyond the cap: the closed arrow still says so.
          ...(rows.slice(REPLY_CAP).some((reply) => !reply.read) ? { replies_more_unread: true } : {}),
        };
      };
      const shown = items.slice(q.offset, end);
      return {
        items: q.threads && !searching ? shown.map(withReplies) : shown,
        total: items.length,
        next_offset: end < items.length ? end : null,
        unread_total: counted ? emails.filter((e) => !e.read).length : null,
        priority_total: counted ? items.filter((item) => item.section === 'Priority').length : null,
        search: searching
          ? {
              timezone: regionalTimeZone(),
              locale: regionalLocale(),
              ...previewCoverage(q.search),
            }
          : null,
      };
    }
    case 'read_rows': {
      const ids = args.ids as number[];
      (window.__previewReadRows ??= []).push([...ids]);
      // Like the Rust backend (mail.rs): at most 100 rows per call, in the order asked, and an id
      // the store no longer has is left out.
      return [...new Set(ids)].slice(0, 100).flatMap((id): MailRow[] => {
        const email = view.emails.find((e) => e.id === id);
        return email ? [readRow(email)] : [];
      });
    }
    case 'search_suggestions': {
      const people = new Map<string, { name: string; address: string }>();
      for (const email of view.emails) {
        if (args.accountId && email.account_id !== args.accountId) continue;
        people.set(email.from_addr.toLowerCase(), {
          name: email.from_name,
          address: email.from_addr,
        });
        for (const recipient of [...email.to, ...email.cc]) {
          const match = /^(.*?)\s*<([^>]+)>$/.exec(recipient);
          const address = match?.[2] || recipient;
          if (!people.has(address.toLowerCase()))
            people.set(address.toLowerCase(), { name: match?.[1] || '', address });
        }
      }
      return [...people.values()].sort(
        (a, b) => a.name.localeCompare(b.name) || a.address.localeCompare(b.address),
      );
    }
    case 'cancel_remote_search':
      return;
    case 'search_remote_mail': {
      const remote = window.__previewRemote;
      if (!remote)
        throw new Error(
          'The browser preview searches sample mail only. Remote search is available in the desktop app.',
        );
      const query = String(args.query);
      if (!args.cursor || remoteSearch.query !== query) Object.assign(remoteSearch, { query, shown: 0 });
      const template = store.emails.find((e) => e.folder === 'Inbox');
      const start = remoteSearch.shown;
      const end = Math.min(start + 50, remote.total);
      // Old messages, metadata only, that stay in the Archive like the Rust backend's rows.
      for (let n = start; n < end && template; n++) {
        const id = ++store.next_id;
        store.emails.push({
          ...structuredClone(template),
          id,
          thread_id: `server-${id}`,
          message_id: `<server-${id}@qamille.local>`,
          subject: `${query} · server ${n + 1}`,
          body: '',
          remote_only: true,
          folder: 'Archive',
          date: template.date - 86_400 * 365 * 5 - n,
          read: true,
          pinned: false,
          due: null,
          attachments: [],
          labels: [],
        });
      }
      remoteSearch.shown = end;
      window.dispatchEvent(new Event('preview-workspace-changed'));
      return {
        added: end - start,
        errors: [],
        ids: [],
        total: remote.total,
        next: end < remote.total ? { accounts: [{ account_id: 1, cursor: { folder: 'All', uid_validity: 1, before_uid: end } }] } : null,
        local_only: false,
        notice: null,
      };
    }
    case 'prefetch_mail':
      return;
    case 'read_mail':
      return structuredClone(view.emails.find((e) => e.id === args.id));
    case 'read_mail_thread': {
      const anchor = view.emails.find((email) => email.id === args.id);
      if (!anchor) throw new Error('Message not found');
      return structuredClone(sampleThread(view.emails, anchor));
    }
    case 'read_replies': {
      // Like the Rust backend (mail.rs `replies`): a limit of 0 means the row's cap, anything
      // else is clamped to 1..=100, and an unknown message is an error.
      if (!view.emails.some((email) => email.id === args.mainId)) throw new Error('Message not found');
      const offset = Number(args.offset ?? 0);
      const limit = args.limit ? Math.min(Math.max(Number(args.limit), 1), 100) : REPLY_CAP;
      return structuredClone(
        replyRows(view.emails, args.mainId as number, String(args.folder ?? '')).slice(offset, offset + limit),
      );
    }
    case 'prepare_print':
    case 'export_mail': {
      const ids = [...new Set(args.ids as number[])];
      if (!ids.length) throw new Error('Select at least one message');
      let emails = ids
        .map((id) => {
          const email = view.emails.find((e) => e.id === id);
          if (!email) throw new Error('Selected message not found');
          return email;
        })
        .sort((a, b) => a.date - b.date || a.id - b.id);
      if (command === 'export_mail') return downloadSample(emails, args.format as 'eml' | 'mbox');
      if (args.thread) {
        if (emails.length !== 1) throw new Error('Select one conversation');
        emails = sampleThread(view.emails, emails[0]);
      }
      return structuredClone(emails);
    }
    case 'queue_mail_action':
      return queue({
        kind: 'mail',
        email_id: Number(args.id),
        action: String(args.action),
        value: args.value ? String(args.value) : null,
      });
    case 'show_row_menu': {
      // No native menu here: keep what was asked and answer the entry the test prepared
      // (`__previewRowMenu`, declared in lib/row-menu.ts). Without one, the menu closes unchosen.
      const menu = (window.__previewRowMenu ??= { calls: [], choice: null });
      type Shown = (typeof menu.calls)[number];
      menu.calls.push({
        items: structuredClone(args.items) as Shown['items'],
        x: Number(args.x),
        y: Number(args.y),
      });
      return menu.choice;
    }
    case 'queue_draft_action': {
      const draft_id = Number(args.id);
      if (args.action === 'send') return queue({ kind: 'send_draft', draft_id });
      if (args.action === 'discard') return queue({ kind: 'discard_draft', draft_id });
      return queue({ kind: 'schedule_draft', draft_id, send_at: Number(args.sendAt) });
    }
    case 'activity_available':
      return true;
    case 'activity_recent':
      return structuredClone(
        [...activityRows].reverse().slice(0, Math.min(Math.max(Number(args.limit) || 1, 1), 20)),
      );
    case 'activity_page': {
      const before = args.beforeSeq == null ? Infinity : Number(args.beforeSeq);
      const filter = (args.filter ?? 'all') as ActivityFilter;
      const limit = Math.min(Math.max(Number(args.limit) || 1, 1), 200);
      return structuredClone(
        [...activityRows]
          .reverse()
          .filter((row) => row.seq < before && activityMatches(row, filter))
          .slice(0, limit),
      );
    }
    case 'activity_unseen_errors':
      return activityRows.filter((row) => activityIsError(row) && row.at_ms > activitySeenMs).length;
    case 'activity_mark_seen':
      activitySeenMs = Date.now();
      return;
    case 'undo_action': {
      const pending = store.undo_actions.find((a) => a.id === args.id);
      if (!pending || pending.status !== 'pending' || pending.expires_at_ms <= Date.now())
        throw new Error('The undo time has expired');
      store.undo_actions = store.undo_actions.filter((a) => a.id !== args.id);
      return structuredClone(pending);
    }
    case 'dismiss_action':
      store.undo_actions = store.undo_actions.filter(
        (a) => a.id !== args.id || a.status !== 'failed',
      );
      return;
    case 'mail_action': {
      applyMail(
        store,
        Number(args.id),
        String(args.action),
        args.value ? String(args.value) : null,
      );
      return;
    }
    case 'list_drafts':
      return structuredClone(view.drafts);
    case 'create_draft': {
      const email = store.emails.find((e) => e.id === args.replyTo);
      const d: Draft = {
        id: ++store.next_id,
        account_id: Number(args.accountId || email?.account_id || store.accounts[0].id),
        to: email && !args.forward ? email.from_addr : '',
        cc: '',
        subject: email ? `${args.forward ? 'Fwd' : 'Re'}: ${email.subject}` : '',
        body: '',
        reply_to: email?.id || null,
        send_at: null,
        attachments: [],
        updated: now(),
        format: 'Plain',
        signature_id: store.signatures[0]?.id || null,
        quote: email
          ? `On ${fullDate(email.date, email.date_missing)}, ${email.from_name} wrote:\n${email.body}`
          : '',
      };
      store.drafts.push(d);
      return structuredClone(d);
    }
    case 'pick_attachments':
      return { paths: [], lease_id: null };
    case 'release_attachment_imports':
      return;
    case 'save_draft': {
      const draft = structuredClone(args.draft) as Draft;
      const i = store.drafts.findIndex((d) => d.id === draft.id);
      if (i < 0) throw new Error('Draft not found');
      if (store.drafts[i].updated !== draft.updated)
        throw new Error('This draft changed elsewhere. Reopen it before saving.');
      draft.updated = Math.max(now(), draft.updated + 1);
      store.drafts[i] = draft;
      return structuredClone(draft);
    }
    case 'discard_draft':
      store.drafts = store.drafts.filter((d) => d.id !== args.id);
      return;
    case 'send_draft':
      throw new Error('Browser preview cannot send mail. Open the desktop app to send.');
    case 'sync_mail':
      return { added: 0, errors: [] };
    case 'save_settings': {
      const update = args.update as { settings: Store['settings'] };
      if (update.settings.time_zone)
        new Intl.DateTimeFormat('en', { timeZone: update.settings.time_zone });
      if (update.settings.locale) Intl.getCanonicalLocales(update.settings.locale);
      store.settings = structuredClone(update.settings);
      // Like the backend: the names from before the compute server are saved as the new ones.
      store.settings.meeting_engine = meetingEngine(store.settings.meeting_engine);
      return;
    }
    // Background presence (src-tauri/src/background.rs): a browser has no tray and no sign-in entry.
    case 'close_to_tray_get':
      return true;
    case 'close_to_tray_set':
    case 'restore_main_window':
      return;
    case 'autostart_status':
    case 'autostart_set':
      return { enabled: false, supported: false };
    // The undo queue at quit (lib/undo-close.ts): the preview sends nothing, so nothing is left.
    case 'undo_send_now':
      return { remaining: 0 };
    case 'undo_quit_anyway':
      return true;
    case 'list_events':
      return structuredClone(store.events);
    case 'save_event': {
      const e = structuredClone(args.event) as CalendarEvent;
      if (!e.id) {
        e.id = ++store.next_id;
        store.events.push(e);
      } else store.events[store.events.findIndex((x) => x.id === e.id)] = e;
      return structuredClone(e);
    }
    case 'delete_event':
      store.events = store.events.filter((e) => e.id !== args.id);
      return;
    case 'list_meetings':
      return structuredClone(store.meetings);
    case 'save_meeting_notes': {
      const m = store.meetings.find((m) => m.id === args.id)!;
      m.notes = String(args.notes);
      return;
    }
    case 'save_signature': {
      const s = structuredClone(args.signature) as Signature;
      if (!s.id) {
        s.id = ++store.next_id;
        store.signatures.push(s);
      } else store.signatures[store.signatures.findIndex((x) => x.id === s.id)] = s;
      return structuredClone(s);
    }
    case 'delete_signature':
      store.signatures = store.signatures.filter((s) => s.id !== args.id);
      return;
    case 'read_attachment': {
      const a = store.emails
        .find((e) => e.id === args.emailId)
        ?.attachments.find((a) => a.id === args.attachmentId);
      return a?.inline_text || 'Open the desktop app to read this attachment.';
    }
    case 'read_attached_message':
      throw new Error('Open the desktop app to read attached messages.');
    case 'attach_mail_to_draft': {
      const ids = [...new Set(args.ids as number[])];
      if (!ids.length) throw new Error('Select at least one message');
      const paths = ids.map((id) => {
        const email = store.emails.find((e) => e.id === id);
        if (!email) throw new Error('Selected message not found');
        const name = email.subject.replace(/[\\/:*?"<>|\x00-\x1f]/g, '_').slice(0, 80) || 'message';
        return `draft-attachments/preview/email-${id}-${name}.eml`;
      });
      return { paths, lease_id: null };
    }
    case 'attachment_save':
      throw {
        message: 'The browser preview cannot save attachments. Open the desktop app.',
        code: 'preview_unsupported',
      };
    case 'saved_file_open':
    case 'saved_file_reveal':
      // A save here is a download and gives no token: only a test that fakes one reaches these.
      throw {
        message: 'The browser preview cannot open saved files. Open the desktop app.',
        code: 'preview_unsupported',
      };
    case 'attachment_preview_source':
      throw {
        message: 'The browser preview cannot read attachment files. Open the desktop app.',
        code: 'preview_unsupported',
      };
    case 'summarize_mail':
      return (
        store.emails
          .find((e) => e.id === args.id)
          ?.body.split('\n')
          .filter(Boolean)
          .slice(1, 3)
          .join('\n') || ''
      );
    case 'search_model_installed':
      return false;
    case 'legacy_local_status': {
      const legacy = window.__previewLegacy;
      const bytes = legacy?.models_bytes ?? 0;
      return {
        show: !!legacy && (legacy.had_server || bytes > 0) && !legacy.dismissed,
        models_bytes: bytes,
        had_server: !!legacy?.had_server,
        guide_url: 'https://github.com/acaprino/qamille-mail/blob/main/docs/server-sul-pc.md',
      };
    }
    case 'legacy_local_free_space': {
      if (!args.confirmed) throw new Error('confirmation_required');
      window.__legacyFreed = true;
      const freed = window.__previewLegacy?.models_bytes ?? 0;
      if (window.__previewLegacy) window.__previewLegacy.models_bytes = 0;
      return freed;
    }
    case 'legacy_local_dismiss':
      if (window.__previewLegacy) window.__previewLegacy.dismissed = true;
      return;
    case 'search_model_status':
      return { installed: false, running: false, done_bytes: 0, total_bytes: 0, error: null };
    case 'indexing_status': {
      const walk = window.__previewHistory;
      return store.accounts.map((account) => ({
        account_id: account.id, account: account.email, indexed_present: 0,
        progress: {
          enabled: true,
          // D4 in the preview: a server with another model blocks indexing until «Reindex».
          state: !server.session ? 'needs_server' : window.__previewServer?.mismatch && !server.reindexed ? 'model_mismatch' : 'waiting_model',
          scanned: 0,
          partial: 0,
          notice: null,
          folders: [],
        },
        listed: store.emails.filter((email) => email.account_id === account.id).length,
        include_spam_trash: !!account.storage_policy?.include_spam_trash,
        history: walk
          ? {
              total: walk.total,
              fetched: walk.state === 'complete' ? walk.total : Math.floor(walk.total / 3),
              complete: walk.state === 'complete',
              last_error: null,
              stopped_for_space: walk.state === 'space' ? 1_600_000_000 : null,
            }
          : null,
      }));
    }
    case 'set_include_spam_trash': {
      const account = store.accounts.find((a) => a.id === Number(args.accountId));
      if (!account) throw new Error('Account not found');
      account.storage_policy = { ...account.storage_policy, include_spam_trash: !!args.enabled };
      return;
    }
    case 'mark_old_unread_read': {
      const before = Date.now() / 1000 - 30 * 86_400;
      const changed = store.emails.filter(
        (email) =>
          email.account_id === Number(args.accountId) && email.folder === 'Inbox' && !email.read && email.date < before,
      );
      for (const email of changed) email.read = true;
      if (changed.length)
        announce(
          { kind: 'email-changed', account_id: Number(args.accountId), ids: changed.map((email) => email.id) },
          { kind: 'counts-changed', account_id: Number(args.accountId) },
        );
      return changed.length;
    }
    case 'history_fetch_folder':
      return;
    case 'app_edition':
    case 'server_status':
    case 'server_probe':
    case 'server_record_consent':
    case 'server_record_llm_consent':
    case 'server_llm_status':
    case 'server_connect':
    case 'server_change_temporary_password':
    case 'server_disable_device_with_password':
    case 'server_devices':
    case 'server_revoke_device':
    case 'server_change_password':
    case 'server_change_server':
    case 'server_sign_out':
    case 'server_delete_account':
    case 'server_startup_check':
    case 'server_finish_onboarding':
    case 'sync_status':
    case 'sync_set_enabled':
    case 'sync_set_mailbox':
    case 'sync_now':
    case 'sync_resolve_removal':
    case 'sync_revoke_mailbox':
    case 'sync_revoke_all':
    case 'account_remove':
    case 'compute_set_mode':
    case 'compute_set_ca_pem':
    case 'compute_forget_pending_ca':
    case 'server_signup_start':
    case 'server_signup_confirm':
    case 'server_reset_start':
    case 'server_reset_confirm':
    case 'server_connect_browser':
    case 'server_cancel_browser_sign_in':
    case 'server_browser_disable_device':
    case 'server_account_info':
    case 'server_start_checkout':
    case 'server_checkout_status':
    case 'server_open_billing_portal':
    case 'server_delete_account_browser':
    case 'compute_status':
    case 'search_reindex':
      return previewServer(command, args);
    case 'configure_indexing':
      throw new Error('Open the desktop app to configure local mail indexing.');
    case 'open_link':
      window.open(String(args.url), '_blank', 'noopener,noreferrer');
      return;
    default:
      throw new Error('This feature needs the desktop app. The browser shows sample mail only.');
  }
}

/** Coverage of a search in the preview: complete, unless `__previewRemote` holds server mail. */
function previewCoverage(query: string) {
  const remote = window.__previewRemote;
  if (!remote) return { coverage: 'complete', notice: null, remote_available: false };
  const italian = regionalLocale().startsWith('it');
  const searched = remoteSearch.query === query;
  const complete = searched && remoteSearch.shown >= remote.total;
  const notice = searched
    ? italian
      ? `Risultati dal server: ${remoteSearch.shown} di ${remote.total}.`
      : `Results from the server: ${remoteSearch.shown} of ${remote.total}.`
    : italian
      ? `Cerchi in ${store.emails.length} messaggi su ${store.emails.length + remote.total} di questo account.`
      : `Searching ${store.emails.length} of ${store.emails.length + remote.total} messages in this account.`;
  return { coverage: complete ? 'complete' : 'partial', notice, remote_available: true };
}

// The link to a Qamille server in the preview (access wave 2). Session and devices live here,
// never a device key; errors carry the backend's English texts.
const PRESET = 'https://qamille.example';
const server = {
  session: null as null | { url: string; username: string; device_id: string },
  devices: [] as { id: string; name: string; created_at: string; last_seen_at: string | null; current: boolean }[],
  limitCleared: false,
  /** Signed in again over a session the server had refused. */
  relinked: false,
  llmConsent: null as { url: string; label: string; external: boolean } | null,
  /** Mailbox synchronization: the general switch, the consent, the mailboxes kept only here. */
  sync: { enabled: false, consented: false, last: null as number | null, off: [] as number[], revoked: [] as number[], removals: null as null | string[] },
  reindexed: false,
  /** Signed in with Google: no password (spec «Vendita Professional»). */
  passwordless: false,
  /** The subscription of the preview account, when the server sells them. */
  subscription: null as null | Record<string, unknown>,
  /** A payment open «in the browser»: the next status poll completes it. */
  checkout: null as null | { id: string; plan: string },
};
// Wave 2 test hooks: a preview that starts connected, or with a mode an older version saved.
{
  const start = window.__previewServer;
  const saved = store.settings as unknown as Record<string, unknown>;
  if (start?.connected) {
    server.session = { url: 'https://calcolo.ufficio.it', username: 'mario', device_id: 'this-device' };
    saved.compute_mode = 'server';
    saved.compute_consented_urls = ['https://calcolo.ufficio.it'];
    if (start.llm) server.llmConsent = { url: 'https://calcolo.ufficio.it', label: start.llm.label, external: start.llm.external };
  } else if (start?.savedMode) saved.compute_mode = start.savedMode;
  // Mailbox synchronization: the first sample account is a connected Gmail mailbox.
  if (start?.sync?.gmail) {
    const first = store.accounts[0] as unknown as Record<string, unknown>;
    first.kind = 'Gmail';
    first.provider_id = 'gmail';
  }
}
const PREVIEW_PLANS = [
  { id: 'pro-monthly', name: 'Qamille Professional mensile', interval: 'month', price_cents: 900, currency: 'EUR' },
  { id: 'pro-yearly', name: 'Qamille Professional annuale', interval: 'year', price_cents: 9000, currency: 'EUR' },
];
/** Wave 9: what the plans include each month, and the consumption of the preview account. */
const PREVIEW_QUOTAS = {
  'pro-monthly': { audio_minutes: 600, embedded_tokens: 2000000, llm_tokens: 100000 },
  'pro-yearly': { audio_minutes: 1200, embedded_tokens: 4000000, llm_tokens: 400000 },
  trial: { audio_minutes: 60, embedded_tokens: 200000, llm_tokens: 10000 },
} as Record<string, { audio_minutes: number; embedded_tokens: number; llm_tokens: number }>;
function previewUsage(planId: unknown, full: string[]) {
  const quota = PREVIEW_QUOTAS[String(planId)] ?? PREVIEW_QUOTAS.trial;
  const meter = (key: 'audio_minutes' | 'embedded_tokens' | 'llm_tokens', share: number) => ({
    used: full.includes(key) ? quota[key] : Math.floor(quota[key] * share),
    limit: quota[key],
  });
  return {
    period_start: '2026-11-01',
    renews_at: '2026-12-01T00:00:00.000Z',
    audio_minutes: meter('audio_minutes', 0.02),
    embedded_tokens: meter('embedded_tokens', 0.5),
    llm_tokens: meter('llm_tokens', 0.12),
  };
}
function previewServer(command: string, args: Record<string, unknown>) {
  const options = window.__previewServer ?? {};
  const professional = options.edition === 'professional';
  const settings = store.settings as unknown as Record<string, unknown>;
  const consented = () => ((settings.compute_consented_urls as string[] | undefined) ??= []);
  const normalize = (url: unknown) => String(url ?? '').trim().replace(/\/$/, '').toLowerCase();
  const fail = (message: string) => {
    throw new Error(message);
  };
  const address = (url: unknown) => {
    const value = normalize(url);
    if (professional && value !== PRESET) fail('This edition connects only to its own Qamille server');
    return value;
  };
  const status = () => ({
    edition: professional ? { kind: 'professional', server: PRESET } : { kind: 'self-hosted' },
    connected: !!server.session,
    url: server.session?.url ?? null,
    username: server.session?.username ?? null,
    device_id: server.session?.device_id ?? null,
    // The backend reads the modes older versions saved (`local`, `remote`) as `off` and `server`.
    compute_mode: computeMode(),
    consented_urls: consented(),
  });
  const computeMode = () => {
    const mode = settings.compute_mode as string | undefined;
    return mode === 'server' || mode === 'remote' ? 'server' : 'off';
  };
  const connect = (url: unknown, temporaryChanged = false) => {
    const value = address(url);
    if (!consented().includes(value)) fail('Agree to send content to this server before signing in');
    const login = temporaryChanged ? 'ok' : (options.login ?? 'ok');
    if (login === 'wrong') fail('Wrong username, email or password');
    if (login === 'temporary') return { kind: 'password_change_required' };
    if (login === 'limit' && !server.limitCleared)
      return {
        kind: 'device_limit',
        devices: [
          { id: 'old-1', name: 'Vecchio portatile', created_at: '2026-01-01', last_seen_at: null, current: false },
          { id: 'old-2', name: 'Ufficio', created_at: '2026-02-01', last_seen_at: null, current: false },
          { id: 'old-3', name: 'Casa', created_at: '2026-03-01', last_seen_at: null, current: false },
        ],
      };
    if (server.session?.url === value) server.relinked = true;
    server.session = { url: value, username: String(args.identifier ?? 'mario'), device_id: 'this-device' };
    server.devices = [
      { id: 'this-device', name: String(args.deviceName ?? 'Qamille'), created_at: '2026-11-13', last_seen_at: null, current: true },
      { id: 'phone', name: 'Portatile di casa', created_at: '2026-10-01', last_seen_at: null, current: false },
    ];
    settings.compute_mode = 'server';
    return { kind: 'connected', status: status() };
  };
  const session = () => server.session ?? fail('This computer is not connected to a Qamille server');
  const none = () => ({ status: 'none', entitled: false, plan_id: null, plan_name: null, provider: null, current_period_end: null, cancel_at_period_end: false, manageable: false });
  const trial = () => ({ ...none(), status: 'trialing', entitled: true, plan_id: 'trial', plan_name: 'Prova', provider: 'trial', current_period_end: '2026-11-30T09:00:00.000Z' });
  // Wave 9: with quotas on, a subscription that is open carries the month's consumption.
  const withUsage = (sub: Record<string, unknown>) =>
    options.quotas && sub.entitled ? { ...sub, usage: previewUsage(sub.plan_id, options.quotas.full ?? []) } : sub;
  const entitled = () => !!(server.subscription as { entitled?: boolean } | null)?.entitled;
  switch (command) {
    case 'app_edition':
      return {
        ...(professional ? { kind: 'professional', server: PRESET } : { kind: 'self-hosted' }),
        default_device_name: professional ? 'Qamille Professional su Windows' : 'Qamille su Windows',
      };
    case 'server_status':
      return status();
    case 'server_probe': {
      const value = String(args.url ?? '').trim();
      // The server of the Docker guide on this PC answers only when the test says so.
      if (value.replace(/\/$/, '') === 'http://127.0.0.1:8787' && !options.local) return { kind: 'unreachable' };
      if (/^http:\/\//i.test(value) && !/^http:\/\/(127\.0\.0\.1|localhost)/i.test(value)) return { kind: 'https_required' };
      if (!/^https?:\/\/[^/\s]+\/?$/i.test(value)) return { kind: 'invalid' };
      address(value);
      const info = {
        version: '1.0.0',
        api: 1,
        signup: !!options.signup,
        password_reset: !!options.password_reset,
        edition: professional ? 'professional' : 'self-hosted',
        login_providers: options.google ? ['google'] : [],
        google_signup: !!options.google && !!options.signup,
        billing: options.billing ? { provider: 'fake', test_mode: true, currency: 'EUR', trial_days: 14, plans: options.quotas ? PREVIEW_PLANS.map((plan) => ({ ...plan, quotas: PREVIEW_QUOTAS[plan.id] })) : PREVIEW_PLANS } : null,
        legal: options.legal ?? null,
        llm: options.llm
          ? { model: 'm', models: ['m'], external: options.llm.external, label: options.llm.label, context_tokens: 8192 }
          : null,
        sync: options.sync?.available ? { available: true } : null,
      };
      const probe = options.probe ?? 'found';
      if (probe === 'found') return { kind: 'found', info };
      if (probe === 'incompatible') return { kind: 'incompatible', info: { ...info, version: '9.0.0', api: 9 } };
      return { kind: probe };
    }
    case 'server_record_consent': {
      const value = address(args.url);
      if (options.sync?.available) server.sync = { ...server.sync, enabled: true, consented: true };
      if (!consented().includes(value)) consented().push(value);
      if (options.llm) server.llmConsent = { url: value, label: options.llm.label, external: options.llm.external };
      return;
    }
    case 'sync_status': {
      const accounts = (store.accounts as { id: number; kind: string; email: string }[]).filter((account) => account.kind !== 'Sample');
      const removals = (server.sync.removals ??= [...(options.sync?.removals ?? [])]);
      const mode = !server.session ? 'no_server' : !options.sync?.available ? 'unavailable' : !server.sync.enabled ? 'off' : options.sync.paused ? 'paused_for_subscription' : 'idle';
      return {
        mode,
        enabled: server.sync.enabled,
        consented: server.sync.consented,
        last_sync: mode === 'idle' ? (server.sync.last ??= Math.floor(Date.now() / 1000)) : null,
        error: null,
        removals: removals.map((email) => ({ item_id: `item-${email}`, name: email, email })),
        mailboxes: accounts.map((account) => ({ account_id: account.id, enabled: !server.sync.off.includes(account.id), needs_reconnect: server.sync.revoked.includes(account.id) })),
      };
    }
    case 'sync_set_enabled':
      session();
      if (args.enabled && !server.sync.consented) fail('Agree to keep the mailboxes on this server first');
      server.sync.enabled = !!args.enabled;
      return;
    case 'sync_set_mailbox': {
      const id = Number(args.accountId);
      server.sync.off = args.enabled ? server.sync.off.filter((value) => value !== id) : [...new Set([...server.sync.off, id])];
      return;
    }
    case 'sync_now':
      return { downloaded: 0, uploaded: 0, removal_notices: 0, paused: false, unavailable: false, skipped: false };
    case 'sync_resolve_removal': {
      const email = String(args.itemId).replace(/^item-/, '');
      server.sync.removals = (server.sync.removals ?? []).filter((value) => value !== email);
      const account = (store.accounts as { id: number; email: string }[]).find((value) => value.email === email);
      if (account && args.removeHere) {
        store.accounts = store.accounts.filter((value) => value.id !== account.id);
        window.dispatchEvent(new Event('preview-workspace-changed'));
      } else if (account) server.sync.off.push(account.id);
      return;
    }
    case 'sync_revoke_mailbox':
    case 'sync_revoke_all': {
      session();
      const accounts = (store.accounts as { id: number; kind: string; email: string; oauth_client_id?: string; provider_id?: string }[]).filter((account) => account.kind !== 'Sample');
      const targets = command === 'sync_revoke_all' ? accounts : accounts.filter((account) => account.id === Number(args.accountId));
      const results = targets.map((account) => {
        const google = (account.provider_id ?? 'gmail') === 'gmail';
        const outcome = !google ? 'manual_password' : options.sync?.revoke === 'failed' ? 'failed' : 'revoked';
        if (outcome === 'revoked') server.sync.revoked.push(account.id);
        return { account_id: account.id, email: account.email, provider_id: account.provider_id ?? 'gmail', outcome };
      });
      return command === 'sync_revoke_all' ? results : results[0];
    }
    case 'account_remove':
      store.accounts = store.accounts.filter((account) => account.id !== Number(args.accountId));
      window.dispatchEvent(new Event('preview-workspace-changed'));
      return;
    case 'server_record_llm_consent': {
      if (options.llm) server.llmConsent = { url: address(args.url), label: options.llm.label, external: options.llm.external };
      return;
    }
    case 'server_llm_status': {
      if (!server.session || !options.llm) return { kind: 'unavailable' };
      const { label, external } = options.llm;
      const agreed = server.llmConsent && server.llmConsent.label === label && server.llmConsent.external === external && !options.llmChanged;
      return { kind: agreed ? 'ready' : 'consent_needed', label, external };
    }
    case 'server_connect':
      return connect(args.url);
    case 'server_change_temporary_password':
      return connect(args.url, true);
    case 'server_disable_device_with_password':
      server.limitCleared = true;
      return;
    case 'server_devices':
      session();
      return structuredClone(server.devices);
    case 'server_revoke_device':
      session();
      server.devices = server.devices.filter((device) => device.id !== args.deviceId);
      return;
    case 'server_change_password':
      session();
      if (String(args.newPassword ?? '').length < 10) fail('The password must be at least 10 characters long');
      if (args.signOutOtherDevices) server.devices = server.devices.filter((device) => device.current);
      return;
    case 'server_change_server': {
      if (professional) fail('This edition cannot change server');
      if (normalize(args.url) === server.session?.url) fail('This computer is already connected to this server');
      return connect(args.url);
    }
    case 'server_sign_out': {
      const offline = !!options.offline;
      server.session = null;
      server.sync = { enabled: false, consented: false, last: null, off: [], revoked: [], removals: null };
      settings.compute_mode = 'off';
      return offline ? 'signed_out_offline' : 'signed_out';
    }
    case 'server_delete_account':
      session();
      if (!args.password) fail('Wrong username, email or password');
      server.session = null;
      settings.compute_mode = 'off';
      return;
    case 'server_startup_check':
      return false;
    case 'server_finish_onboarding':
      settings.onboarding_done = true;
      return;
    case 'compute_set_mode':
      if (args.mode === 'server' && !server.session) fail('This computer is not connected to a Qamille server');
      settings.compute_mode = args.mode;
      return;
    case 'compute_set_ca_pem':
    case 'compute_forget_pending_ca':
      return;
    case 'server_signup_start':
      if (!professional) fail('Accounts are created by the server administrator');
      if (!consented().includes(address(args.url))) fail('Agree to send content to this server before signing in');
      if (!options.signup) fail('This server does not offer sign-up');
      return;
    case 'server_signup_confirm': {
      if (!professional) fail('Accounts are created by the server administrator');
      if (String(args.code ?? '').trim() !== '12345678') fail('The code is wrong or expired. Ask for a new one');
      if (String(args.password ?? '').length < 10) fail('The password must be 10 to 256 characters long');
      const email = String(args.email ?? '');
      if (options.legal?.terms_version && args.acceptedTerms !== options.legal.terms_version)
        fail('The Terms and the privacy notice must be accepted to create the account');
      connect(args.url, true);
      server.session = { url: address(args.url), username: String(args.username || email.split('@')[0]), device_id: 'this-device' };
      return status();
    }
    case 'server_reset_start':
      address(args.url);
      if (!options.password_reset) fail('This server does not offer password recovery');
      return;
    case 'server_reset_confirm':
      if (String(args.code ?? '').trim() !== '12345678') fail('The code is wrong or expired. Ask for a new one');
      if (String(args.newPassword ?? '').length < 10) fail('The password must be 10 to 256 characters long');
      return;
    case 'server_connect_browser': {
      if (!options.google) fail('This server does not offer browser sign-in');
      if (options.googleAnswer === 'no_account')
        fail('No Qamille account uses this Google address. Ask the administrator, or create an account');
      if (options.legal?.terms_version && args.acceptedTerms !== options.legal.terms_version)
        fail('The Terms and the privacy notice must be accepted to create the account');
      const result = connect(args.url) as { kind: string };
      if (result.kind === 'connected') {
        server.session = { url: address(args.url), username: String(options.googleUser ?? 'giulia.verdi'), device_id: 'this-device' };
        server.passwordless = true;
        if (options.billing && !server.subscription)
          server.subscription = options.trial === false ? null : trial();
        if (options.billing && !entitled()) settings.compute_mode = 'off';
        return { kind: 'connected', status: status() };
      }
      return result;
    }
    case 'server_cancel_browser_sign_in':
      return;
    case 'server_browser_disable_device':
      server.limitCleared = true;
      return previewServer('server_connect_browser', { url: server.session?.url ?? PRESET, deviceName: 'Qamille' });
    case 'server_account_info': {
      const current = session();
      return {
        user_id: 'u1',
        username: current.username,
        email: server.passwordless ? `${current.username}@gmail.com` : null,
        has_password: !server.passwordless,
        identities: server.passwordless ? ['google'] : [],
        billing_enabled: !!options.billing,
        subscription: options.billing ? withUsage(server.subscription ?? none()) : null,
      };
    }
    case 'server_start_checkout': {
      session();
      if (!options.billing) fail('This server does not sell subscriptions');
      const plan = PREVIEW_PLANS.find((p) => p.id === args.planId) ?? fail('This server does not sell subscriptions');
      server.checkout = { id: 'checkout-preview', plan: plan.id };
      return { id: 'checkout-preview', url: `${PRESET}/billing/fake/checkout/preview`, expires_at: '2026-11-16T12:00:00Z' };
    }
    case 'server_checkout_status': {
      const pending = server.checkout ?? fail('Unknown payment session');
      server.checkout = null;
      const plan = PREVIEW_PLANS.find((p) => p.id === pending.plan)!;
      if (options.declined) return { id: pending.id, status: 'failed', plan_id: plan.id };
      server.subscription = {
        status: 'active',
        entitled: true,
        plan_id: plan.id,
        plan_name: plan.name,
        provider: 'fake',
        current_period_end: plan.interval === 'year' ? '2027-11-16T09:00:00.000Z' : '2026-12-16T09:00:00.000Z',
        cancel_at_period_end: false,
        manageable: true,
      };
      return { id: pending.id, status: 'completed', plan_id: plan.id };
    }
    case 'server_open_billing_portal':
      session();
      if (!(server.subscription as { manageable?: boolean } | null)?.manageable) fail('This account has no subscription to manage');
      return;
    case 'server_delete_account_browser':
      session();
      server.session = null;
      server.passwordless = false;
      server.subscription = null;
      settings.compute_mode = 'off';
      return;
    case 'compute_status': {
      const mode = computeMode();
      if (mode !== 'server' || !server.session)
        return { route: mode === 'server' ? 'not_connected' : 'off', url: null, reachable: false, unlinked: false, version: null, models: [], model_mismatch: null };
      const unlinked = !!options.unlinked && !server.relinked;
      return {
        route: 'server',
        url: server.session.url,
        reachable: !unlinked,
        unlinked,
        version: unlinked ? null : '1.0.0',
        models: unlinked
          ? []
          : [
              { id: 'embeddinggemma-2', state: 'ready', percent: 100 },
              { id: 'whisper-base', state: 'downloading', percent: 40 },
              { id: 'whisper-small', state: 'missing', percent: null },
            ],
        model_mismatch:
          options.mismatch && !server.reindexed && !unlinked
            ? {
                server: { model: 'nomic-embed-text', dimensions: 768, quantization: 'fp16' },
                index: { model: 'embeddinggemma-2', dimensions: 256, quantization: 'q4f16' },
              }
            : null,
      };
    }
    case 'search_reindex':
      server.reindexed = true;
      return;
  }
}
