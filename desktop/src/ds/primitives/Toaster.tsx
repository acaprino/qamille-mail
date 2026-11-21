// The app's notices, on Ark UI Toast. `notify(message, error, actions)` shows one notice at a time (a
// new one replaces the old): a status for outcomes, an alert for errors, with a close button. It closes by
// itself (4.5 s, 12 s for an error) and waits while the pointer or focus is on it. A notice may carry up to
// two actions, `ds` buttons inside it: a click runs the action and closes the notice, and with actions the
// notice stays 10 s, since there is something to decide.
// The look is in ds/styles/ark/toast.css; the region sits on --z-toast and above the composer dock.
// In a Portal on <body>: an open Modal hides everything outside it from screen readers (aria-hidden)
// except live regions at that level, so a notice raised from a dialog is still read (native audit T14).
import { Portal } from '@ark-ui/react/portal';
import { Toast, Toaster as ArkToaster, createToaster } from '@ark-ui/react/toast';
import Button from './Button';
import Icon from './Icon';
import Tooltip from './Tooltip';

/** A button in a notice, such as Open after a file was saved. Whatever it does is a convenience. */
export type NoticeAction = { label: string; run: () => void };

/** How long a notice stays: a status is read at a glance, an error has to be read in full. */
const STATUS_MS = 4500;
const ERROR_MS = 12000;
/** A notice with actions stays at least this long: the person may want one of them. */
const ACTIONS_MS = 10000;
/** More do not fit beside the message and the close button. */
const MAX_ACTIONS = 2;

const toaster = createToaster({
  placement: 'bottom',
  overlap: false,
  max: 1,
  removeDelay: 200,
  // The padding between the notice and the window edge is --toast-bottom (styles/ark/toast.css).
  offsets: { top: '0px', left: '24px', right: '24px', bottom: 'var(--toast-bottom, 24px)' },
  // Ark's own shortcut (Alt+T) is not one of the app's: a key no keyboard sends switches it off.
  hotkey: ['F24'],
});

export function notify(message: string, error = false, actions: NoticeAction[] = []) {
  const shown = actions.slice(0, MAX_ACTIONS);
  const duration = error ? ERROR_MS : STATUS_MS;
  // In a microtask: Ark's toaster renders each change at once (flushSync), so that the removal is
  // seen before the new notice is added. Called from a React effect, flushSync cannot run, the add
  // reads the list from before the removal and the old notice stayed on screen next to the new one.
  queueMicrotask(() => {
    toaster.remove();
    toaster.create({
      description: message,
      type: error ? 'error' : 'success',
      duration: shown.length ? Math.max(duration, ACTIONS_MS) : duration,
      meta: { actions: shown },
    });
  });
}

// The words come from the caller: the module does not reach into the app's translations.
export default function Toaster({ label, closeLabel }: { label: string; closeLabel: string }) {
  return (
    <Portal>
      <ArkToaster toaster={toaster} aria-label={label} style={{ zIndex: 'var(--z-toast)' }}>
        {(item) => {
          const failed = item.type === 'error';
          const actions: NoticeAction[] = item.meta?.actions ?? [];
          return (
            <Toast.Root
              className={failed ? 'toast error' : 'toast'}
              role={failed ? 'alert' : 'status'}
            >
              <Icon name={failed ? 'info' : 'check'} size={18} />
              <Toast.Description>{item.description}</Toast.Description>
              {actions.map((action) => (
                <Button
                  key={action.label}
                  variant="secondary"
                  type="button"
                  className="toast-action"
                  onClick={() => {
                    // The notice closes even when the action throws.
                    try {
                      action.run();
                    } finally {
                      toaster.dismiss(item.id);
                    }
                  }}
                >
                  {action.label}
                </Button>
              ))}
              <Tooltip text={closeLabel}>
                <Toast.CloseTrigger className="icon-button" aria-label={closeLabel}>
                  <Icon name="close" size={16} />
                </Toast.CloseTrigger>
              </Tooltip>
            </Toast.Root>
          );
        }}
      </ArkToaster>
    </Portal>
  );
}
