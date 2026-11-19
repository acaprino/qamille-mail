// How the interface applies the typed events the backend announces (ADR-0007 decision 5,
// INV-004). Everything here is pure: the caller owns the state and decides when to read rows.
import type {
  AppEvent,
  EmailSummary,
  ListFilter,
  MailPage,
  MailRow,
  SyncPhase,
  UndoAction,
  Workspace,
} from './types';

/** `read_rows` answers at most this many rows per call, like `list_mail` (mail.rs clamps to 100). */
export const readRowsMax = 100;

export interface EventEffects {
  /** The page after the event; the very same object when the event changes nothing in it. */
  page: MailPage;
  /** The workspace after the event; the very same object when the event changes nothing in it. */
  workspace: Workspace;
  /** Ids to read with `read_rows({ ids })`, then to hand to `applyRows`. At most `readRowsMax`. */
  needsRows: number[];
  /** The list cannot be patched row by row: run the full coalesced refresh. */
  reload: boolean;
  /** The counters or the drafts changed: take one coalesced `workspace_snapshot`. */
  snapshot: boolean;
}

const idList = (value: unknown): value is number[] =>
  Array.isArray(value) && value.every(isId);
const isId = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const phases: readonly string[] = ['syncing', 'complete', 'error'] satisfies SyncPhase[];
const undoStatuses: readonly string[] = ['pending', 'committing', 'failed'] satisfies UndoAction['status'][];
/** A backend `Option<u8>` percent: an integer 0..=100, or `null` when unknown. */
const isPercent = (value: unknown): value is number | null =>
  value === null || (isId(value) && value <= 100);

function isUndoAction(value: unknown): value is UndoAction {
  if (!value || typeof value !== 'object') return false;
  const action = value as Record<string, unknown>;
  const operation = action.operation as Record<string, unknown> | null;
  return (
    isId(action.id) &&
    typeof action.subject === 'string' &&
    typeof action.created_at_ms === 'number' &&
    typeof action.expires_at_ms === 'number' &&
    typeof action.status === 'string' &&
    undoStatuses.includes(action.status) &&
    !!operation &&
    typeof operation === 'object' &&
    typeof operation.kind === 'string'
  );
}

/**
 * Checks a payload that came from outside (a Tauri event, a DOM event) and returns it as an
 * `AppEvent`, or null when it is not one the backend serializes. Unknown fields are dropped.
 */
export function parseAppEvent(payload: unknown): AppEvent | null {
  if (!payload || typeof payload !== 'object') return null;
  const event = payload as Record<string, unknown>;
  switch (event.kind) {
    case 'email-changed':
      return isId(event.account_id) && idList(event.ids)
        ? { kind: 'email-changed', account_id: event.account_id, ids: event.ids }
        : null;
    case 'counts-changed':
      return isId(event.account_id) ? { kind: 'counts-changed', account_id: event.account_id } : null;
    case 'undo-changed':
      return isUndoAction(event.action) ? { kind: 'undo-changed', action: event.action } : null;
    case 'undo-removed':
      return isId(event.id) ? { kind: 'undo-removed', id: event.id } : null;
    case 'drafts-changed':
      return idList(event.ids) ? { kind: 'drafts-changed', ids: event.ids } : null;
    case 'sync-progress':
      return isId(event.account_id) &&
        typeof event.phase === 'string' &&
        phases.includes(event.phase) &&
        isId(event.added)
        ? {
            kind: 'sync-progress',
            account_id: event.account_id,
            phase: event.phase as SyncPhase,
            added: event.added,
          }
        : null;
    case 'index-progress':
      return isId(event.account_id) && isId(event.percent)
        ? { kind: 'index-progress', account_id: event.account_id, percent: event.percent }
        : null;
    case 'index-waiting':
      return isId(event.account_id) && typeof event.waiting === 'boolean'
        ? { kind: 'index-waiting', account_id: event.account_id, waiting: event.waiting }
        : null;
    case 'history-progress':
      return isId(event.account_id) &&
        typeof event.fetched === 'number' &&
        Number.isInteger(event.fetched) &&
        typeof event.total === 'number' &&
        Number.isInteger(event.total) &&
        typeof event.complete === 'boolean'
        ? {
            kind: 'history-progress',
            account_id: event.account_id,
            fetched: event.fetched,
            total: event.total,
            complete: event.complete,
          }
        : null;
    case 'history-unread':
      return isId(event.account_id) && isId(event.count)
        ? { kind: 'history-unread', account_id: event.account_id, count: event.count }
        : null;
    case 'model-progress':
      return isPercent(event.percent) && typeof event.done === 'boolean'
        ? { kind: 'model-progress', percent: event.percent, done: event.done }
        : null;
    case 'contacts-sync':
      return (event.account_id === null || isId(event.account_id)) &&
        typeof event.running === 'boolean'
        ? { kind: 'contacts-sync', account_id: event.account_id, running: event.running }
        : null;
    case 'notice':
      return typeof event.text === 'string' ? { kind: 'notice', text: event.text } : null;
    case 'accounts-changed':
      return { kind: 'accounts-changed' };
    case 'activity-changed':
      return idList(event.ids) ? { kind: 'activity-changed', ids: event.ids } : null;
    default:
      return null;
  }
}

/**
 * What an event does to the screen. Undo events are applied here, with no call. An
 * `email-changed` only says which messages changed, so it cannot be applied yet: it names the
 * rows to read (`needsRows`), and `applyRows` places them once they arrive. A list that cannot
 * be patched row by row (conversations, a search) or a change too big for one read reloads.
 * Pure: neither argument is touched.
 */
export function applyEvent(
  page: MailPage,
  workspace: Workspace,
  event: AppEvent,
  filter: ListFilter,
): EventEffects {
  const effects: EventEffects = { page, workspace, needsRows: [], reload: false, snapshot: false };
  switch (event.kind) {
    case 'undo-changed': {
      const held = workspace.undo_actions.some((action) => action.id === event.action.id);
      const undo_actions = held
        ? workspace.undo_actions.map((action) => (action.id === event.action.id ? event.action : action))
        : [...workspace.undo_actions, event.action];
      return { ...effects, workspace: { ...workspace, undo_actions } };
    }
    case 'undo-removed': {
      if (!workspace.undo_actions.some((action) => action.id === event.id)) return effects;
      const undo_actions = workspace.undo_actions.filter((action) => action.id !== event.id);
      return { ...effects, workspace: { ...workspace, undo_actions } };
    }
    case 'counts-changed':
    case 'drafts-changed':
      return { ...effects, snapshot: true };
    case 'email-changed': {
      // A list of one account does not show the mail of another.
      if (filter.account_id !== null && filter.account_id !== event.account_id) return effects;
      if (filter.threads || filter.search.trim()) return { ...effects, reload: true };
      const needsRows = [...new Set(event.ids)];
      return needsRows.length > readRowsMax
        ? { ...effects, reload: true }
        : { ...effects, needsRows };
    }
    default:
      return effects;
  }
}

/** Whether two filters name the same list: same view, account, search text and display mode. */
export function sameFilter(a: ListFilter, b: ListFilter): boolean {
  return (
    a.folder === b.folder &&
    a.account_id === b.account_id &&
    a.search === b.search &&
    a.threads === b.threads
  );
}

/** The keys a `read_rows` row must always carry, `null` or `false` when unset (see `MailRow`). */
const flagKeys = ['snoozed_until', 'set_aside', 'reminder_at'] as const;

/**
 * Whether every row carries the three flags that decide membership of Inbox, Tasks and Snoozed.
 * The backend serializes an unset `Option` as `null`, never as a missing key, so a row without
 * one means the contract is broken: reading it as "not set" would put a snoozed or set-aside
 * message back in the Inbox. The caller reloads the list instead of patching it.
 */
export function carriesFlags(rows: unknown): rows is MailRow[] {
  return (
    Array.isArray(rows) &&
    rows.every((row) => !!row && typeof row === 'object' && flagKeys.every((key) => key in row))
  );
}

const hiddenFolders = ['Trash', 'Spam'];
const snoozed = (row: MailRow, now: number) => (row.snoozed_until ?? 0) > now;

/**
 * Whether a message belongs to the list `filter` describes, by the rules of `MailService::list`
 * (mail.rs) for a list that is not a search. `now` is in seconds, like the dates of the rows.
 */
export function rowMatches(row: MailRow, filter: ListFilter, now: number): boolean {
  if (filter.account_id !== null && filter.account_id !== row.account_id) return false;
  const folder = row.folder ?? '';
  const hidden = hiddenFolders.includes(folder);
  const wanted: string = filter.folder;
  if (wanted === '' || wanted === 'Inbox')
    return folder === 'Inbox' && !row.done && !snoozed(row, now);
  if (wanted === 'Tasks')
    return (
      !row.done &&
      !hidden &&
      (row.pinned || row.due != null || row.reminder_at != null || !!row.set_aside)
    );
  if (wanted === 'Snoozed') return snoozed(row, now) && !hidden;
  if (wanted.startsWith('label:')) return row.labels.includes(wanted.slice(6)) && !hidden;
  return folder.toLowerCase() === wanted.toLowerCase();
}

// The backend lists newest first: by date, then by id, both descending.
const sortsAfter = (a: EmailSummary, b: EmailSummary) =>
  a.date < b.date || (a.date === b.date && a.id < b.id);

/**
 * Where `row` goes among `items`, or -1 when it sorts after every loaded row while more rows
 * remain on the backend: "Load more" brings it in its turn.
 */
function slotOf(items: EmailSummary[], row: EmailSummary, complete: boolean): number {
  const at = items.findIndex((item) => sortsAfter(item, row));
  return at >= 0 ? at : complete ? items.length : -1;
}

/**
 * Applies the rows `read_rows` answered for `requested` ids to a list that is not a search nor
 * grouped into conversations. A row that still matches the filter replaces the one on the page
 * (or joins the page at its place in the order); a row that does not match, or that the backend
 * no longer has, leaves it. `total` and `next_offset` follow, so "Load more" continues right
 * after the last loaded row. Rows already loaded are never dropped for any other reason
 * (INV-007). Pure; returns the same page when nothing changed.
 */
export function applyRows(
  page: MailPage,
  requested: number[],
  rows: MailRow[],
  filter: ListFilter,
  now: number,
): MailPage {
  const fresh = new Map(rows.map((row) => [row.id, row]));
  let items = page.items;
  let total = page.total;
  let next = page.next_offset;
  let changed = false;
  for (const id of new Set(requested)) {
    const at = items.findIndex((item) => item.id === id);
    const row = fresh.get(id);
    const belongs = !!row && rowMatches(row, filter, now);
    const rest = at >= 0 ? items.filter((_, index) => index !== at) : items;
    if (at >= 0 && row && belongs) {
      const to = slotOf(rest, row, next === null);
      if (to === at) items = items.map((item, index) => (index === at ? row : item));
      else if (to >= 0) items = [...rest.slice(0, to), row, ...rest.slice(to)];
      else {
        // It now sorts past the loaded rows: it leaves the page and comes back with "Load more".
        items = rest;
        next = next === null ? null : Math.max(0, next - 1);
      }
      changed = true;
    } else if (at >= 0) {
      items = rest;
      total = Math.max(0, total - 1);
      next = next === null ? null : Math.max(0, next - 1);
      changed = true;
    } else if (row && belongs) {
      const to = slotOf(items, row, next === null);
      // Beyond the loaded rows it is neither shown nor counted: the total is the backend's to say.
      if (to < 0) continue;
      items = [...items.slice(0, to), row, ...items.slice(to)];
      total += 1;
      next = next === null ? null : next + 1;
      changed = true;
    }
  }
  return changed ? { ...page, items, total, next_offset: next } : page;
}

/**
 * Makes `task` safe to call from any number of events: one pass runs at a time and at most one
 * waits behind it. A caller gets the first pass that starts after its call, so it waits for the
 * running pass plus one more however many events keep arriving; callers that arrive during the
 * same pass share it. The returned promise never rejects: `task` reports its own errors.
 */
export function coalescing(task: () => Promise<void>): () => Promise<void> {
  type Pass = { done: Promise<void>; finish: () => void };
  let running = false;
  let queued: Pass | null = null;
  async function drain(first: Pass) {
    running = true;
    try {
      for (let pass: Pass | null = first; pass; pass = queued) {
        queued = null;
        try {
          await task();
        } finally {
          pass.finish();
        }
      }
    } finally {
      running = false;
      queued?.finish();
      queued = null;
    }
  }
  return () => {
    if (queued) return queued.done;
    let finish!: () => void;
    const pass: Pass = { done: new Promise<void>((resolve) => (finish = resolve)), finish };
    if (running) queued = pass;
    else void drain(pass);
    return pass.done;
  };
}
