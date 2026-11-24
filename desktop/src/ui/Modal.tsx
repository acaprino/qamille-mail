// A centred dialog that opens as soon as it is mounted, drawn by Sigrid's Dialog (Base UI's dialog). Esc calls `onClose`
// (the caller then unmounts it); a click on the veil does not: a half-filled form must not vanish on a stray click, and
// neither does the Esc that confirms an IME candidate. The focus stays inside while it is open and goes back to the element
// that had it when it closes. `size` is `sm`, `md` (default) or `lg`: the widths are Qamille's tokens, given to Sigrid's
// window. Sigrid's window has a close button of its own (its words name it).
import { useRef, type CSSProperties, type HTMLAttributes } from 'react';
import { DialogContent, DialogRoot } from '@acaprino/sigrid/primitives';
import { imeComposing } from '../lib/ime';
import { useReturnFocus } from '../lib/focus-return';

export type ModalProps = HTMLAttributes<HTMLDivElement> & {
  size?: 'sm' | 'md' | 'lg';
  onClose?: () => void;
  'aria-labelledby'?: string;
  /** Where the focus starts; by default the first control. A confirmation whose first button
   *  deletes something starts on its Cancel, so Enter right after opening deletes nothing. */
  initialFocus?: () => HTMLElement | null;
};

/** What Base UI tells about a close: why, the event behind it, and `cancel()` to keep the window open (Sigrid's types name it loosely). */
type CloseDetails = { reason?: string; event?: Event; cancel?: () => void };

export default function Modal({ size = 'md', onClose, initialFocus, 'aria-label': ariaLabel, className, style, children, ...rest }: ModalProps) {
  // The caller closes the dialog by unmounting it, which gives the dialog no chance to restore the focus.
  const content = useRef<HTMLDivElement>(null);
  useReturnFocus(content);
  const width: CSSProperties = { width: `min(var(--modal-${size}), calc(100vw - 48px))`, maxWidth: 'none', ...style };
  return (
    <DialogRoot
      open
      onOpenChange={(open: boolean, details?: CloseDetails) => {
        if (open) return;
        const key = details?.event instanceof KeyboardEvent ? details.event : null;
        if (details?.reason === 'outside-press' || (key && imeComposing(key))) {
          details?.cancel?.();
          return;
        }
        onClose?.();
      }}
    >
      <DialogContent
        ref={content}
        aria-label={ariaLabel}
        className={className}
        style={width}
        initialFocus={initialFocus ? () => initialFocus() ?? true : undefined}
        {...rest}
      >
        {children}
      </DialogContent>
    </DialogRoot>
  );
}
