// A centred dialog (ADR-0008) that opens as soon as it is mounted, on Ark UI Dialog. Esc calls
// `onClose` (the caller then unmounts it); a click on the veil does not: a half-filled form must
// not vanish on a stray click. The focus stays inside while it is open and goes back to the element
// that had it when it closes. It draws over everything in a Portal, on --z-modal.
// The look is in ds/styles/ark/dialog.css. `size` is `sm`, `md` (default) or `lg`.
import { Dialog } from '@ark-ui/react/dialog';
import { Portal } from '@ark-ui/react/portal';
import { useRef, type HTMLAttributes } from 'react';
import { useReturnFocus } from '../focus-return';

export type ModalProps = HTMLAttributes<HTMLDivElement> & {
  size?: 'sm' | 'md' | 'lg';
  onClose?: () => void;
  'aria-labelledby'?: string;
  /** Where the focus starts; by default the first control. A confirmation whose first button
   *  deletes something starts on its Cancel, so Enter right after opening deletes nothing. */
  initialFocus?: () => HTMLElement | null;
};

export default function Modal({ size = 'md', onClose, initialFocus, 'aria-label': ariaLabel, className, children, ...rest }: ModalProps) {
  // The caller closes the dialog by unmounting it, which gives Ark no chance to restore the focus.
  const content = useRef<HTMLDivElement>(null);
  useReturnFocus(content);
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
