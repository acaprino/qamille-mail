// The single registry of live work (ADR-0007 v3 decision 7, INV-009): sync, indexing,
// model install, contacts sync and the confirmations past their undo window live
// in one list, fed only by the typed events of decision 5. Everything here is
// pure: the ActivityCenter component (card and panel in the sidebar footer) owns the
// state, the clock and the show/hide delays. What already happened is not here: it is
// the persistent history of `activity-log.ts` and the Activity page.
import type { AppEvent, UndoAction } from './types';
import { confirmingUndoActions } from './undo-notices.ts';

export type ActivityKind = 'sync' | 'index' | 'history' | 'model' | 'contacts' | 'confirming';

export interface Activity {
  /** e.g. "sync:2", "index:2", "model", "contacts", "confirming". */
  id: string;
  kind: ActivityKind;
  accountId: number | null;
  /** Translation key chosen by the caller, never raw backend text. */
  phase: string;
  /** 0-100, or null while indeterminate. */
  percent: number | null;
  /** Epoch ms of the start; it never moves while the activity lives. */
  since: number;
  /** Epoch ms of the last event that moved it: the quiet rule (isStale) reads this. */
  last: number;
}

/** An activity younger than this is invisible: short work must not flash (EX-008). */
export const activityMinVisibleMs = 400;
/** The indicator stays this long after the last activity ends, so consecutive work does not blink. */
export const activityHideDelayMs = 1000;
/**
 * Without events for this long an activity goes quiet: the spinner stops and the icon dims, with no
 * word (EX-008, softened). It must stay far above the backend's heartbeat of an unchanged indexing
 * percent (INDEX_HEARTBEAT_SECS, app/src/application/indexing.rs), or a slow account looks stuck.
 */
export const activityStaleMs = 300_000;

function upsert(current: Activity[], fresh: Omit<Activity, 'since' | 'last'>, now: number): Activity[] {
  const at = current.findIndex((activity) => activity.id === fresh.id);
  if (at < 0) return [...current, { ...fresh, since: now, last: now }];
  const previous = current[at];
  const activity: Activity = { ...previous, ...fresh, since: previous.since, last: now };
  return current.map((item, index) => (index === at ? activity : item));
}
const remove = (current: Activity[], id: string): Activity[] =>
  current.some((activity) => activity.id === id)
    ? current.filter((activity) => activity.id !== id)
    : current;

/**
 * Applies one typed event to the registry and returns the new list (the very
 * same object when the event moves no activity). `sync-progress` opens and
 * closes per account; `index-progress` ends at 100%; `model-progress` ends on
 * `done`, success or failure alike (errors stay separate toasts, INV-009);
 * `contacts-sync` keeps one activity for the whole run, per-book events moving
 * it to the account being read.
 */
export function applyActivityEvent(current: Activity[], event: AppEvent, now: number): Activity[] {
  switch (event.kind) {
    case 'sync-progress':
      return event.phase === 'syncing'
        ? upsert(
            current,
            {
              id: `sync:${event.account_id}`,
              kind: 'sync',
              accountId: event.account_id,
              phase: 'syncing',
              percent: null,
            },
            now,
          )
        : remove(current, `sync:${event.account_id}`);
    case 'index-progress':
      return event.percent >= 100
        ? remove(current, `index:${event.account_id}`)
        : upsert(
            current,
            {
              id: `index:${event.account_id}`,
              kind: 'index',
              accountId: event.account_id,
              // A percent that moves ends a pause of the compute server.
              phase: 'indexing',
              percent: event.percent,
            },
            now,
          );
    case 'index-waiting': {
      // A pause of the compute server is the state of the indexing activity (INV-6), never an
      // error: the activity keeps its percent and says it waits.
      const id = `index:${event.account_id}`;
      const previous = current.find((activity) => activity.id === id);
      if (!event.waiting && !previous) return current;
      return upsert(
        current,
        {
          id,
          kind: 'index',
          accountId: event.account_id,
          phase: event.waiting ? 'waiting' : 'indexing',
          percent: previous?.percent ?? null,
        },
        now,
      );
    }
    case 'history-progress':
      // "fetched/total" in the phase: the indicator says "X of Y", no percentage (INV-7).
      return event.complete
        ? remove(current, `history:${event.account_id}`)
        : upsert(
            current,
            {
              id: `history:${event.account_id}`,
              kind: 'history',
              accountId: event.account_id,
              phase: `${event.fetched}/${event.total}`,
              percent: null,
            },
            now,
          );
    case 'model-progress':
      return event.done
        ? remove(current, 'model')
        : upsert(
            current,
            { id: 'model', kind: 'model', accountId: null, phase: 'installing', percent: event.percent },
            now,
          );
    case 'contacts-sync':
      return event.running
        ? upsert(
            current,
            { id: 'contacts', kind: 'contacts', accountId: event.account_id, phase: 'syncing', percent: null },
            now,
          )
        : remove(current, 'contacts');
    default:
      return current;
  }
}

/**
 * Adds (or refreshes) the `confirming` activity from the workspace's undo
 * actions: the confirmations past their undo window, exactly the count the old
 * "Salvataggio di N azioni…" row showed, while the actionable rows stay in
 * UndoStack. The caller feeds the previous result back in (the component keeps
 * it in its own state): the start of an existing count never moves, so the
 * entry can age into the stale state like any other activity, and a call that
 * changes nothing returns the very same list — safe to re-run on every tick.
 */
export function withConfirming(current: Activity[], actions: UndoAction[], now: number): Activity[] {
  const count = confirmingUndoActions(actions, now).length;
  const previous = current.find((activity) => activity.id === 'confirming');
  if (!count) return remove(current, 'confirming');
  const phase = String(count);
  if (previous && previous.phase === phase) return current;
  const activity: Activity = previous
    ? { ...previous, phase, last: now }
    : { id: 'confirming', kind: 'confirming', accountId: null, phase, percent: null, since: now, last: now };
  return previous
    ? current.map((item) => (item.id === 'confirming' ? activity : item))
    : [...current, activity];
}

/** What the one indicator line shows: confirming > sync > model > index > contacts, oldest first. */
const priority: readonly ActivityKind[] = ['confirming', 'sync', 'model', 'index', 'history', 'contacts'];
export function primaryActivity(list: Activity[]): Activity | null {
  let best: Activity | null = null;
  for (const activity of list) {
    if (best === null) {
      best = activity;
      continue;
    }
    const rank = priority.indexOf(activity.kind) - priority.indexOf(best.kind);
    if (rank < 0 || (rank === 0 && activity.since < best.since)) best = activity;
  }
  return best;
}

/** The activities old enough to be shown: everything younger stays invisible (no flashing). */
export function displayableActivities(list: Activity[], now: number): Activity[] {
  return list.filter((activity) => now - activity.since >= activityMinVisibleMs);
}

/** Whether an activity went quiet for too long: it stays listed, but its spinner stops and it dims. */
export function isStale(activity: Activity, now: number): boolean {
  return now - activity.last > activityStaleMs;
}

/** Whole seconds the activity has been running, for the list rows. */
export function activitySeconds(activity: Activity, now: number): number {
  return Math.max(0, Math.floor((now - activity.since) / 1000));
}
