// A centred dialog (ADR-0008) that opens as soon as it is mounted, on Ark UI Dialog. Esc calls
// `onClose` (the caller then unmounts it); a click on the veil does not: a half-filled form must
// not vanish on a stray click. The focus stays inside while it is open and goes back to the element
// that had it when it closes. It draws over everything in a Portal, on --z-modal.
// The look is in ds/styles/ark/dialog.css. `size` is `sm`, `md` (default) or `lg`.
import { Dialog } from '@ark-ui/react/dialog';
import { Portal } from '@ark-ui/react/portal';
import { useEffect, useRef, useState, type HTMLAttributes } from 'react';

export type ModalProps = HTMLAttributes<HTMLDivElement> & {
  size?: 'sm' | 'md' | 'lg';
  onClose?: () => void;
  'aria-labelledby'?: string;
  /** Where the focus starts; by default the first control. A confirmation whose first button
   *  deletes something starts on its Cancel, so Enter right after opening deletes nothing. */
  initialFocus?: () => HTMLElement | null;
};

/** The button that opened the popup (`aria-controls`, as Ark sets it) holding `element`, if any. */
function popupTrigger(element: HTMLElement | null): HTMLElement | null {
  for (let node = element?.parentElement; node; node = node.parentElement) {
    if (!node.id) continue;
    const trigger = document.querySelector<HTMLElement>(`[aria-controls="${CSS.escape(node.id)}"]`);
    if (trigger && !node.contains(trigger)) return trigger;
  }
  return null;
}

export default function Modal({ size = 'md', onClose, initialFocus, 'aria-label': ariaLabel, className, children, ...rest }: ModalProps) {
  // The caller closes the dialog by unmounting it, which gives Ark no chance to restore the focus:
  // put it back on the opener when it would fall to <body>.
  const [opener] = useState(() =>
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  // Opened from an item of a menu or popover that closes meanwhile: back to that popup's trigger.
  const [fallback] = useState(() => popupTrigger(opener));
  const content = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const dialog = content.current;
    return () => {
      const lost = document.activeElement === document.body || !!dialog?.contains(document.activeElement);
      const target = opener?.isConnected ? opener : fallback?.isConnected ? fallback : null;
      if (lost && target) target.focus();
    };
  }, [opener, fallback]);
  return (
    <Dialog.Root
      open
      onOpenChange={(details) => {
        if (!details.open) onClose?.();
      }}
      aria-label={ariaLabel}
      closeOnInteractOutside={false}
      initialFocusEl={initialFocus}
    >
      <Portal>
        <Dialog.Backdrop data-variant="modal" />
        <Dialog.Positioner data-variant="modal">
          <Dialog.Content ref={content} data-variant="modal" data-size={size} className={className} {...rest}>
            {children}
          </Dialog.Content>
        </Dialog.Positioner>
      </Portal>
    </Dialog.Root>
  );
}
