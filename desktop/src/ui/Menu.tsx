// A command menu anchored to its button, drawn by Sigrid's DropdownMenu (Base UI's menu): role menu with menu items, so the
// arrows move between the items, Home and End go to the ends, a letter jumps to the item that starts with it, and Enter
// or a click chooses. Esc and a choice close it and give the focus back to the trigger. `trigger` is the one button
// element, `label` is the menu's accessible name, `items` are the commands in order: a separator goes between two groups,
// and the destructive items go last, after a separator of their own. `tip` gives an icon-only trigger its hint.
// `onOpenChange` tells the caller when it opens and closes, and `open={false}` closes it. A filter, a picker or a small
// form is not a menu: that stays a Popover.
import { Fragment, cloneElement, useEffect, useId, useState, type ReactElement } from 'react';
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger } from '@acaprino/sigrid/primitives';
import Icon from '../ui/Icon';
import Tooltip from './Tooltip';

export type MenuItem = {
  value: string;
  label: string;
  onSelect: () => void;
  /** Deleting and the like: the item goes last, after a separator. */
  destructive?: boolean;
  disabled?: boolean;
  /** A shortcut the app really has: shown at the end of the item, announced as aria-keyshortcuts. */
  shortcut?: string;
  /** An Icon name, drawn at 15px before the label. */
  icon?: string;
  /** Items of one group stay together: a separator goes between two groups. */
  group?: string;
  /** Makes the item a switch (menuitemcheckbox): checked, it shows a check mark where the icon goes. */
  checked?: boolean;
  /** A hint on the item, such as why it is disabled. */
  title?: string;
};

export type MenuProps = {
  trigger: ReactElement<{ id?: string }>;
  label: string;
  items: MenuItem[];
  tip?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export default function Menu({ trigger, label, items, tip, open, onOpenChange }: MenuProps) {
  const [own, setOwn] = useState(false);
  const generated = useId();
  const triggerId = trigger.props.id ?? `menu-trigger-${generated}`;
  // The caller closes it (the reader, when the message changes): only a change of `open` is an order; the menu's own
  // state answers in onOpenChange.
  useEffect(() => {
    if (open !== undefined) setOwn(open);
  }, [open]);
  function choose(item: MenuItem) {
    // The focus is back on the trigger before the command runs: a Modal or a SidePanel the command opens remembers the
    // trigger, and gives the focus back to it when it closes.
    document.getElementById(triggerId)?.focus({ preventScroll: true });
    item.onSelect();
  }
  const regular = items.filter((item) => !item.destructive);
  const destructive = items.filter((item) => item.destructive);
  const anchored = <DropdownMenuTrigger asChild>{cloneElement(trigger, { id: triggerId })}</DropdownMenuTrigger>;
  return (
    <DropdownMenu
      open={own}
      onOpenChange={(next) => {
        setOwn(next);
        onOpenChange?.(next);
      }}
    >
      {tip ? <Tooltip text={tip}>{anchored}</Tooltip> : anchored}
      <DropdownMenuContent aria-label={label} align="end" side="bottom" sideOffset={4}>
        {regular.map((item, index) => (
          <Fragment key={item.value}>
            {index > 0 && item.group !== regular[index - 1]?.group && <DropdownMenuSeparator />}
            <Entry item={item} choose={choose} />
          </Fragment>
        ))}
        {regular.length > 0 && destructive.length > 0 && <DropdownMenuSeparator />}
        {destructive.map((item) => (
          <Entry key={item.value} item={item} choose={choose} />
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** One item: a menu item, or a menu item checkbox when it has a `checked` state. */
function Entry({ item, choose }: { item: MenuItem; choose: (item: MenuItem) => void }) {
  const mark = item.checked === undefined ? item.icon : item.checked ? 'check' : undefined;
  const inside = (
    <>
      {mark && <Icon name={mark} size={15} />}
      <span>{item.label}</span>
      {item.shortcut && <DropdownMenuShortcut aria-hidden="true">{item.shortcut}</DropdownMenuShortcut>}
    </>
  );
  const common = { disabled: item.disabled, title: item.title, 'aria-keyshortcuts': item.shortcut };
  return item.checked === undefined ? (
    <DropdownMenuItem {...common} variant={item.destructive ? 'destructive' : 'default'} onSelect={() => choose(item)}>
      {inside}
    </DropdownMenuItem>
  ) : (
    <DropdownMenuCheckboxItem {...common} checked={item.checked} closeOnClick onCheckedChange={() => choose(item)}>
      {inside}
    </DropdownMenuCheckboxItem>
  );
}
