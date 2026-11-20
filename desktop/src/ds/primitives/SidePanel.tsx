// A panel from the side with a scrim (ADR-0008), on Ark UI Dialog in its non-modal form: it fills the
// pane it sits in (the nearest positioned ancestor), does not trap the focus, and the rest of the page
// stays reachable. Esc calls `onClose`, or `onScrim` when there is none; a click on the scrim calls
// `onScrim`. The caller closes the panel by unmounting it, which gives Ark no chance to restore the
// focus: the panel puts it back on the element that had it when it opened (see focus-return.ts).
// `onOpen` runs once it is showing, to place the focus (without it the first control gets it).
// The look is in ds/styles/ark/dialog.css.
import { Dialog } from '@ark-ui/react/dialog';
import { useCallback, useRef, type HTMLAttributes, type Ref } from 'react';
import { useReturnFocus } from '../focus-return';

export type SidePanelProps = HTMLAttributes<HTMLDivElement> & {
  narrow?: boolean;
  ref?: Ref<HTMLDivElement>;
  onScrim: () => void;
  onOpen?: () => void;
  onClose?: () => void;
};

export default function SidePanel({
  narrow = false,
  className,
  ref,
  'aria-label': ariaLabel,
  onScrim,
  onOpen,
  onClose,
  children,
  ...rest
}: SidePanelProps) {
  const content = useRef<HTMLDivElement | null>(null);
  useReturnFocus(content);
  // The panel keeps its own handle on the node and hands the same one to the caller's `ref` (a cleanup
  // that a callback ref returns is passed on: React 19 runs it in place of a call with null).
  const setContent = useCallback(
    (node: HTMLDivElement | null) => {
      content.current = node;
      if (typeof ref === 'function') return ref(node);
      if (ref) ref.current = node;
    },
    [ref],
  );
  return (
    <Dialog.Root
      open
      onOpenChange={(details) => {
        if (!details.open) (onClose ?? onScrim)();
      }}
      modal={false}
      trapFocus={false}
      preventScroll={false}
      closeOnInteractOutside={false}
      aria-label={ariaLabel}
      initialFocusEl={() => {
        // Ark calls this once the panel is showing: the caller's choice stays, or the first control.
        if (!onOpen) return null;
        onOpen();
        return document.activeElement instanceof HTMLElement ? document.activeElement : null;
      }}
    >
      <Dialog.Positioner data-variant="side">
        {/* Pointer shortcut only: the panel's close button and Esc are the accessible ways out. */}
        <Dialog.Backdrop data-variant="side" onClick={onScrim} />
        <Dialog.Content
          ref={setContent}
          data-variant="side"
          data-narrow={narrow ? '' : undefined}
          className={className}
          {...rest}
        >
          {children}
        </Dialog.Content>
      </Dialog.Positioner>
    </Dialog.Root>
  );
}
