// A command menu anchored to its button (ADR-0008), on Ark UI Menu: role menu with menu items, so the
// arrows move between the items, Home and End go to the ends, a letter jumps to the item that starts
// with it, and Enter or a click chooses. Esc and a choice close it and give the focus back to the
// trigger, as Ark does; a press outside closes it and leaves the focus where the press put it.
// `trigger` is the one button element (Ark merges its props and ref into it), `label` is the menu's
// accessible name, `items` are the commands in order: a separator goes between two groups, and the
// destructive items go last, after a separator of their own. The surface exists only while open, in
// a Portal on --z-popover, with the look of the Popover menus it replaces (ds/styles/ark/menu.css).
// `tip` gives an icon-only trigger its hint, as Popover's does. `onOpenChange` tells the caller when it
// opens and closes, and `open={false}` closes it. A filter, a picker or a small form is not a menu:
// that stays a Popover.
import { Menu as ArkMenu, useMenu } from '@ark-ui/react/menu';
import { Portal } from '@ark-ui/react/portal';
import { Fragment, useEffect, useId, useState, type PointerEvent, type ReactElement } from 'react';
import Icon from './Icon';
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
  trigger: ReactElement;
  label: string;
  items: MenuItem[];
  tip?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export default function Menu({ trigger, label, items, tip, open, onOpenChange }: MenuProps) {
  const id = useId();
  // The menu finds its trigger by id; a Tooltip on the same button would give it its own id, and the
  // surface would open at the window's corner. Both roots share one id (as in Popover).
  const triggerId = `menu-trigger-${id}`;
  const contentId = `menu-content-${id}`;
  function choose(value: string) {
    const item = items.find((one) => one.value === value);
    if (!item) return;
    // The focus is back on the trigger before the command runs (Ark puts it there too, a moment later,
    // as the menu closes): a Modal or a SidePanel the command opens remembers the trigger, and gives
    // the focus back to it when it closes.
    document.getElementById(triggerId)?.focus({ preventScroll: true });
    item.onSelect();
  }
  // Ark keeps the open state itself. Told what to show from outside (`open`), it would give the focus
  // back to the trigger on every close, even after a press on another control that took the focus.
  const menu = useMenu({
    ids: { trigger: triggerId, content: contentId },
    onOpenChange: (details) => onOpenChange?.(details.open),
    onSelect: (details) => choose(details.value),
    positioning: { placement: 'bottom-end', gutter: 4 },
  });
  const shown = menu.api.open;
  // The caller closes it (the reader, when the message changes) through Ark, which then gives the
  // focus back to the trigger.
  useEffect(() => {
    if (open !== undefined && open !== menu.api.open) menu.api.setOpen(open);
    // Only a change of `open` is an order: the menu's own state answers in onOpenChange.
  }, [open]);
  // Whether the keys lead: the item Ark marks as current then carries the focus ring (menu.css), as a
  // focused button does, while under the pointer an item is tinted. :focus-visible cannot tell: Ark
  // leaves the focus where it was when the trigger is pressed, so a menu opened with the mouse after a
  // key elsewhere would match it.
  const [keyboard, setKeyboard] = useState(false);
  const pointerMoved = (event: PointerEvent<HTMLElement>) => {
    // A real move: the browser also sends moves without one when a surface appears under the pointer.
    if (event.movementX || event.movementY) setKeyboard(false);
  };
  const regular = items.filter((item) => !item.destructive);
  const destructive = items.filter((item) => item.destructive);
  const anchored = (
    <ArkMenu.Trigger asChild onPointerDown={() => setKeyboard(false)} onKeyDown={() => setKeyboard(true)}>
      {trigger}
    </ArkMenu.Trigger>
  );
  return (
    <ArkMenu.RootProvider value={menu} lazyMount unmountOnExit>
      {tip ? (
        <Tooltip text={tip} triggerId={triggerId}>
          {anchored}
        </Tooltip>
      ) : (
        anchored
      )}
      {shown && (
        <Portal>
          <ArkMenu.Positioner>
            {/* Named by `label`, as the Popover was. Ark names a menu after its trigger with
                aria-labelledby; pointing that at the menu itself makes its own aria-label the name. */}
            <ArkMenu.Content
              aria-label={label}
              aria-labelledby={contentId}
              data-keyboard={keyboard ? '' : undefined}
              onKeyDown={() => setKeyboard(true)}
              onPointerMove={pointerMoved}
            >
              {regular.map((item, index) => (
                <Fragment key={item.value}>
                  {index > 0 && item.group !== regular[index - 1].group && <ArkMenu.Separator />}
                  <Entry item={item} />
                </Fragment>
              ))}
              {regular.length > 0 && destructive.length > 0 && <ArkMenu.Separator />}
              {destructive.map((item) => (
                <Entry key={item.value} item={item} />
              ))}
            </ArkMenu.Content>
          </ArkMenu.Positioner>
        </Portal>
      )}
    </ArkMenu.RootProvider>
  );
}

/** One item: a menu item, or a menu item checkbox when it has a `checked` state. */
function Entry({ item }: { item: MenuItem }) {
  const mark = item.checked === undefined ? item.icon : item.checked ? 'check' : undefined;
  const inside = (
    <>
      {mark && <Icon name={mark} size={15} />}
      <span>{item.label}</span>
      {item.shortcut && <kbd aria-hidden="true">{item.shortcut}</kbd>}
    </>
  );
  // valueText is what a typed letter matches: the label, never the shortcut.
  const common = {
    value: item.value,
    valueText: item.label,
    disabled: item.disabled,
    title: item.title,
    'aria-keyshortcuts': item.shortcut,
  };
  return item.checked === undefined ? (
    <ArkMenu.Item {...common}>{inside}</ArkMenu.Item>
  ) : (
    <ArkMenu.CheckboxItem {...common} checked={item.checked}>
      {inside}
    </ArkMenu.CheckboxItem>
  );
}
