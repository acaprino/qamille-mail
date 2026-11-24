// The app's notices, in Sigrid's column. `notify(message, error, actions)` raises one: a status for outcomes that goes by
// itself (10 s, and it waits while the pointer or focus is on it), an error that stays until it is closed. A notice may
// carry up to two actions, buttons inside it: a click runs the action and closes the notice. The column holds up to ten,
// the newest at the bottom; more push the oldest out. The region's name and the words come from Sigrid's words
// (ui/words.ts), so `Toaster` takes no labels. Drawn by Sigrid's Toasts (Ark's toast).
import { useSyncExternalStore } from 'react';
import { Toasts, type ToastItem } from '@acaprino/sigrid/components';
import { tr } from '../lib/i18n';

/** A button in a notice, such as Open after a file was saved. Whatever it does is a convenience. */
export type NoticeAction = { label: string; run: () => void };

/** How long a status stays: Sigrid has one time for all (R8 of the plan's asks); an error never counts down. */
const STATUS_MS = 10000;
/** More do not fit beside the message and the close button. */
const MAX_ACTIONS = 2;

let notices: ToastItem[] = [];
let counter = 0;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((listener) => listener());
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => void listeners.delete(listener);
};

function dismiss(id: string) {
  if (!notices.some((notice) => notice.id === id)) return;
  notices = notices.filter((notice) => notice.id !== id);
  emit();
}

export function notify(message: string, error = false, actions: NoticeAction[] = []) {
  const id = `notice-${++counter}`;
  const shown = actions.slice(0, MAX_ACTIONS);
  const item: ToastItem = {
    id,
    text: message,
    ...(error ? { kind: 'error' as const } : {}),
    ...(shown.length
      ? {
          actions: [
            {
              label: tr('Azioni dell’avviso', 'Notice actions'),
              items: shown.map((action, index) => ({
                id: `${id}-${index}`,
                label: action.label,
                onClick: () => {
                  // The notice closes even when the action throws.
                  try {
                    action.run();
                  } finally {
                    dismiss(id);
                  }
                },
              })),
            },
          ],
        }
      : {}),
  };
  notices = [...notices, item];
  emit();
}

// The words are Sigrid's (WordsProvider in SigridRoot): the props are kept so the callers need not change.
export default function Toaster(_props: { label?: string; closeLabel?: string }) {
  const items = useSyncExternalStore(subscribe, () => notices);
  return <Toasts items={items} durationMs={STATUS_MS} onDismiss={dismiss} />;
}
