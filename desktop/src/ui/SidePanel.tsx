// A panel from the side (looks at or edits one thing without leaving the view), drawn by Sigrid's Sheet in its non-modal
// form: it does not trap the focus, and the rest of the page stays reachable. Esc, or a click outside, calls `onClose`
// (or `onScrim` when there is none). The caller closes the panel by unmounting it, which gives the sheet no chance to
// restore the focus: the panel puts it back on the element that had it when it opened (ds/focus-return.ts). `onOpen`
// runs once it is showing, to place the focus (without it the first control gets it). The panel has its own bar with a
// close button, so Sigrid's is off. The sheet opens at the edge of the window (it no longer fills the pane it sits in).
import { useCallback, useRef, type CSSProperties, type HTMLAttributes, type Ref } from 'react';
import { Sheet, SheetContent } from '@acaprino/sigrid';
import { useReturnFocus } from '../ds/focus-return';

export type SidePanelProps = HTMLAttributes<HTMLDivElement> & {
  narrow?: boolean;
  ref?: Ref<HTMLDivElement>;
  onScrim: () => void;
  onOpen?: () => void;
  onClose?: () => void;
};

export default function SidePanel({ narrow = false, className, style, ref, 'aria-label': ariaLabel, onScrim, onOpen, onClose, children, ...rest }: SidePanelProps) {
  const content = useRef<HTMLDivElement | null>(null);
  useReturnFocus(content);
  // The panel keeps its own handle on the node and hands the same one to the caller's `ref`.
  const setContent = useCallback(
    (node: HTMLDivElement | null) => {
      content.current = node;
      if (typeof ref === 'function') return ref(node);
      if (ref) ref.current = node;
    },
    [ref],
  );
  const width: CSSProperties = { width: narrow ? 'min(var(--panel-narrow), 100%)' : 'min(var(--panel-wide), 78%)', maxWidth: 'none', ...style };
  return (
    <Sheet open modal={false} onOpenChange={(open) => !open && (onClose ?? onScrim)()}>
      <SheetContent
        ref={setContent}
        side="right"
        showCloseButton={false}
        aria-label={ariaLabel}
        data-narrow={narrow ? '' : undefined}
        className={className}
        style={width}
        initialFocus={
          onOpen
            ? () => {
                // Once the panel is showing: the caller's choice stays.
                onOpen();
                return document.activeElement instanceof HTMLElement ? document.activeElement : true;
              }
            : undefined
        }
        {...rest}
      >
        {children}
      </SheetContent>
    </Sheet>
  );
}
