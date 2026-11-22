// A hint for a control that has no room for words: it names the action and, when there is one, its shortcut. It is a hint,
// never the name: the control keeps its own aria-label. `children` is the one trigger element, which receives the
// hint's handlers. Opens on keyboard focus at once and on hover after Sigrid's delay; closes on Escape, on blur and when
// the pointer leaves. Drawn by Sigrid's Tooltip (Base UI's tooltip).
import type { ReactElement } from 'react';
import { Tooltip as SigridTooltip } from '@acaprino/sigrid';

export type TooltipProps = {
  text: string;
  shortcut?: string;
  children: ReactElement;
  /** Not read: Sigrid's tooltip and popover share a button without help. */
  triggerId?: string;
};

export default function Tooltip({ text, shortcut, children }: TooltipProps) {
  return (
    <SigridTooltip label={text} shortcut={shortcut}>
      {children}
    </SigridTooltip>
  );
}
