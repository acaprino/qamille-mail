// The native context menu of the mail rows (docs/aspetto-nativo.md, rule 33). The entries are decided
// here and only here; `src-tauri/src/row_menu.rs` draws them as a Windows menu under the pointer and
// answers with the one chosen. The actions are the row's and the reader's (`action`, `compose` in
// App.tsx), with their locks. Pure but for `showRowMenu`: the language comes in as `tr`, because
// `i18n.ts` uses the app's store and plain Node (tests/row-menu.mjs) cannot load it.
import type { EmailSummary, Folder } from './types';

/** One entry of the menu, as `show_row_menu` takes it; `value: null` is a separator. */
export interface RowMenuEntry {
  value: string | null;
  label: string;
  enabled: boolean;
  checked: boolean | null;
}
type Tr = (italian: string, english: string) => string;

declare global {
  interface Window {
    /**
     * Test hook: the browser preview has no native menu. Its `show_row_menu` appends what it was
     * asked to show to `calls` and answers `choice` (`null`: the menu closed without a choice).
     */
    __previewRowMenu?: { calls: { items: RowMenuEntry[]; x: number; y: number }[]; choice: string | null };
  }
}

// The entries that open the composer instead of acting on the message.
const composing = new Set(['reply', 'reply-all', 'forward']);
// The entries that are actions of `queue_mail_action`, with the value some of them carry.
const actions: Record<string, string | undefined> = {
  archive: undefined,
  moveToTrash: undefined,
  markAsDone: undefined,
  snooze: 'Tomorrow',
  markAsSeen: undefined,
  markAsUnseen: undefined,
  pin: undefined,
  unpin: undefined,
  moveToInbox: undefined,
};

/**
 * What the menu offers for `rows`: the row that was right-clicked, or every checked row when it is
 * one of them. One row can be answered; several get the group actions only. `locked` (a message of
 * the rows is waiting for the backend) greys out what would change the messages, as in the reader.
 */
export function rowMenuEntries(rows: EmailSummary[], view: Folder, locked: boolean, tr: Tr): RowMenuEntry[] {
  if (!rows.length) return [];
  const entry = (value: string, label: string, enabled = true): RowMenuEntry => ({
    value,
    label,
    enabled,
    checked: null,
  });
  const change = (value: string, label: string) => entry(value, label, !locked);
  const separator: RowMenuEntry = { value: null, label: '', enabled: false, checked: null };
  const entries: RowMenuEntry[] = [];
  if (rows.length === 1)
    entries.push(
      entry('reply', tr('Rispondi', 'Reply')),
      entry('reply-all', tr('Rispondi a tutti', 'Reply all')),
      entry('forward', tr('Inoltra', 'Forward')),
      separator,
    );
  entries.push(
    change('archive', tr('Archivia', 'Archive')),
    change('moveToTrash', tr('Sposta nel cestino', 'Move to trash')),
    change('markAsDone', tr('Fatto', 'Done')),
    change('snooze', tr('Posticipa a domani', 'Snooze until tomorrow')),
    separator,
  );
  if (rows.some((row) => !row.read)) entries.push(change('markAsSeen', tr('Segna come letto', 'Mark as read')));
  if (rows.some((row) => row.read)) entries.push(change('markAsUnseen', tr('Segna come non letto', 'Mark as unread')));
  entries.push(
    separator,
    rows.every((row) => row.pinned)
      ? change('unpin', tr('Rimuovi fissaggio', 'Unpin'))
      : change('pin', tr('Fissa', 'Pin')),
  );
  // Only for mail that is not in the Inbox, as in the reader: the folder of the row, not the view,
  // since the Tasks and Snoozed views hold Inbox mail and a search holds mail of every folder.
  if (rows.some((row) => (row.folder || view) !== 'Inbox'))
    entries.push(separator, change('moveToInbox', tr('Sposta in Posta in arrivo', 'Move to inbox')));
  return entries;
}

/** The mail action behind a chosen entry; `null` for the entries that open the composer. */
export function rowMenuAction(value: string): { action: string; value?: string } | null {
  if (composing.has(value) || !Object.hasOwn(actions, value)) return null;
  const extra = actions[value];
  return extra === undefined ? { action: value } : { action: value, value: extra };
}

/**
 * Shows the menu at `at`, in CSS pixels of the page, and answers the chosen entry, or `null`
 * when the menu closes without a choice. Rust takes physical pixels of the client area, like the
 * system menu (`lib/window-shell.ts`): `devicePixelRatio` carries both the app zoom and the scale.
 */
export async function showRowMenu(items: RowMenuEntry[], at: { x: number; y: number }): Promise<string | null> {
  if (!items.some((item) => item.value !== null)) return null;
  const scale = window.devicePixelRatio || 1;
  // Loaded here, not at the top: Node, which tests the entries, cannot resolve the app's imports.
  const { call } = await import('./api');
  try {
    return (await call<string | null>('show_row_menu', { items, x: at.x * scale, y: at.y * scale })) ?? null;
  } catch {
    // No menu, as when it closes without a choice.
    return null;
  }
}
