// A filter, a picker or a small form anchored to its trigger, drawn by Sigrid's Popover (Base UI's popover): a dialog, not
// a menu (a command menu is a Menu). `trigger` is the one button element; `label` is the accessible name of the surface.
// Esc, a press outside and the focus leaving close it, and the focus goes back to the trigger. `open` and `onOpenChange`
// make it controlled: an action that has done its job calls onOpenChange(false). `tip` gives an icon-only trigger its
// hint: the Tooltip wraps the trigger.
import { useState, type ReactElement, type ReactNode } from 'react';
import { Popover as SigridPopover, PopoverContent, PopoverTrigger } from '@acaprino/sigrid';
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
  const anchored = <PopoverTrigger asChild>{trigger}</PopoverTrigger>;
  return (
    <SigridPopover
      open={shown}
      onOpenChange={(next) => {
        if (open === undefined) setOwn(next);
        onOpenChange?.(next);
      }}
    >
      {tip ? <Tooltip text={tip}>{anchored}</Tooltip> : anchored}
      <PopoverContent aria-label={label} className={className} align="end" side="bottom" sideOffset={4}>
        {children}
      </PopoverContent>
    </SigridPopover>
  );
}
