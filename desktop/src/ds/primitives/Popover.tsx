// A small menu or filter anchored to its trigger (ADR-0008), on Ark UI Popover. `trigger` is the one
// button element (Ark merges its props into it); `label` is the accessible name of the surface. Esc, a
// press outside and the focus leaving close it, and the focus goes back to the trigger. `open` and
// `onOpenChange` make it controlled: a menu item that does its job calls onOpenChange(false).
// The surface exists only while open, in a Portal on --z-popover. The look is in ds/styles/ark/popover.css.
// `tip` gives an icon-only trigger its hint: the Tooltip wraps Ark's trigger, so both merge their props
// into the one button (a trigger made with Button's own `tip` would hide it from the popover).
import { Popover as ArkPopover } from '@ark-ui/react/popover';
import { Portal } from '@ark-ui/react/portal';
import { useId, useState, type ReactElement, type ReactNode } from 'react';
import Tooltip from './Tooltip';

export type PopoverProps = {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  trigger: ReactElement;
  label: string;
  tip?: string;
  className?: string;
  children?: ReactNode;
};

export default function Popover({ open, onOpenChange, trigger, label, tip, className, children }: PopoverProps) {
  const [own, setOwn] = useState(false);
  const shown = open ?? own;
  // The popover finds its trigger by id to hang from it; a Tooltip on the same button would give it its own
  // id, and the surface would open at the window's corner. Both roots share one id.
  const triggerId = `popover-trigger-${useId()}`;
  return (
    <ArkPopover.Root
      ids={{ trigger: triggerId }}
      open={shown}
      onOpenChange={(details) => {
        if (open === undefined) setOwn(details.open);
        onOpenChange?.(details.open);
      }}
      lazyMount
      unmountOnExit
      positioning={{ placement: 'bottom-end', gutter: 4 }}
    >
      {tip ? (
        <Tooltip text={tip} triggerId={triggerId}>
          <ArkPopover.Trigger asChild>{trigger}</ArkPopover.Trigger>
        </Tooltip>
      ) : (
        <ArkPopover.Trigger asChild>{trigger}</ArkPopover.Trigger>
      )}
      {shown && (
        <Portal>
          <ArkPopover.Positioner>
            <ArkPopover.Content aria-label={label} className={className}>
              {children}
            </ArkPopover.Content>
          </ArkPopover.Positioner>
        </Portal>
      )}
    </ArkPopover.Root>
  );
}
