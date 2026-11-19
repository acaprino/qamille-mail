// A hint for a control that has no room for words: it names the action and, when there is one,
// its shortcut. It is a hint, never the name: the control keeps its own aria-label.
// `children` is the one trigger element; Ark merges its props and handlers into it (asChild).
// Opens on keyboard focus at once and on hover after 400 ms; closes on Escape, on blur and when the
// pointer leaves; a touch tap opens nothing. The tip exists in the DOM only while it is open
// (lazyMount, unmountOnExit). The look is in ds/styles/ark.css.
import { Portal } from '@ark-ui/react/portal';
import { Tooltip as ArkTooltip } from '@ark-ui/react/tooltip';
import type { ReactElement } from 'react';

export type TooltipProps = {
  text: string;
  shortcut?: string;
  children: ReactElement;
};

export default function Tooltip({ text, shortcut, children }: TooltipProps) {
  return (
    <ArkTooltip.Root
      lazyMount
      unmountOnExit
      openDelay={400}
      closeDelay={100}
      positioning={{ placement: 'bottom', gutter: 8 }}
    >
      <ArkTooltip.Trigger asChild>{children}</ArkTooltip.Trigger>
      <Portal>
        <ArkTooltip.Positioner>
          <ArkTooltip.Content>
            <span>{text}</span>
            {shortcut && <kbd>{shortcut}</kbd>}
          </ArkTooltip.Content>
        </ArkTooltip.Positioner>
      </Portal>
    </ArkTooltip.Root>
  );
}
