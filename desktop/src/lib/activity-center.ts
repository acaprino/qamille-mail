// The state of the activity center the sidebar footer and the Activity page share (centro attività):
// whether the panel is open, whether a history exists, its newest rows and the errors not seen yet.
// The live work (sync, indexing…) is not here: it stays in `activity.ts` and the App's registry.
import { call } from './api';
import { reactive } from './store';
import { coalesceEntries } from './activity-log';
import type { ActivityEntry } from './types';

/** Rows read for the panel: enough that a bulk action does not push everything else out of its three. */
const RECENT_READ = 50;
const RECENT_SHOWN = 3;

export const activityCenter = reactive({
  open: false,
  /** False with the JSON store: the panel then says the history needs the SQLite engine. */
  available: true,
  /** The newest rows the panel shows (3), after bulk actions are merged. */
  recent: [] as ActivityEntry[],
  /** The last read of the history failed: the surfaces say so instead of showing an empty list. */
  failed: false,
  /** Errors and failed actions newer than the last time the panel or the page was opened. */
  unseen: 0,
  /** Counts the reads, one per `activity-changed`: the Activity page reloads when it moves. */
  version: 0,
});

let reading: Promise<void> | null = null;
let readAgain = false;

/**
 * Re-reads what the footer shows. A burst of `activity-changed` events costs one more read, not
 * one each. While the panel is open its rows are being looked at: they count as seen at once.
 */
export function refreshActivity(): Promise<void> {
  activityCenter.version += 1;
  if (reading) {
    readAgain = true;
    return reading;
  }
  reading = (async () => {
    try {
      do {
        readAgain = false;
        if (activityCenter.open) await call('activity_mark_seen');
        const [available, recent, unseen] = await Promise.all([
          call<boolean>('activity_available'),
          call<ActivityEntry[]>('activity_recent', { limit: RECENT_READ }),
          call<number>('activity_unseen_errors'),
        ]);
        activityCenter.available = available;
        activityCenter.recent = coalesceEntries(recent).slice(0, RECENT_SHOWN);
        activityCenter.unseen = activityCenter.open ? 0 : unseen;
        activityCenter.failed = false;
      } while (readAgain);
    } catch {
      // The history is a convenience, but an unreadable one must not look like an empty one.
      activityCenter.failed = true;
    } finally {
      reading = null;
    }
  })();
  return reading;
}

export function openActivityPanel(): void {
  if (activityCenter.open) return;
  activityCenter.open = true;
  // Opening is the "seen": the dot goes out now, the backend marker follows.
  activityCenter.unseen = 0;
  void refreshActivity();
}

export function closeActivityPanel(): void {
  activityCenter.open = false;
}

export function toggleActivityPanel(): void {
  if (activityCenter.open) closeActivityPanel();
  else openActivityPanel();
}
