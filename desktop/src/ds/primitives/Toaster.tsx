// The app's notices, on Ark UI Toast. `notify(message, error)` shows one notice at a time (a new one
// replaces the old): a status for outcomes, an alert for errors, with a close button. It closes by
// itself (4.5 s, 12 s for an error) and waits while the pointer or focus is on it.
// The look is in ds/styles/ark/toast.css; the region sits on --z-toast and above the composer dock.
// In a Portal on <body>: an open Modal hides everything outside it from screen readers (aria-hidden)
// except live regions at that level, so a notice raised from a dialog is still read (native audit T14).
import { Portal } from '@ark-ui/react/portal';
import { Toast, Toaster as ArkToaster, createToaster } from '@ark-ui/react/toast';
import Icon from './Icon';
import Tooltip from './Tooltip';

/** How long a notice stays: a status is read at a glance, an error has to be read in full. */
const STATUS_MS = 4500;
const ERROR_MS = 12000;

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

export function notify(message: string, error = false) {
  // In a microtask: Ark's toaster renders each change at once (flushSync), so that the removal is
  // seen before the new notice is added. Called from a React effect, flushSync cannot run, the add
  // reads the list from before the removal and the old notice stayed on screen next to the new one.
  queueMicrotask(() => {
    toaster.remove();
    toaster.create({
      description: message,
      type: error ? 'error' : 'success',
      duration: error ? ERROR_MS : STATUS_MS,
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
          return (
            <Toast.Root
              className={failed ? 'toast error' : 'toast'}
              role={failed ? 'alert' : 'status'}
            >
              <Icon name={failed ? 'info' : 'check'} size={18} />
              <Toast.Description>{item.description}</Toast.Description>
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
