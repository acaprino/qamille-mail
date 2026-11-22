// The strip of open drafts above the composer: a tab for each draft with what is inside it (an icon, the title, a mark when
// it has unsaved changes) and a button after it that closes it. Not Sigrid's Tabs: those have no room for a button
// beside a tab, for the caller's own attributes on it, or for a click on the chosen tab (asked of Sigrid as R17).
// Activation is manual: the arrows, Home and End move the focus, and Enter or Space picks. `onPick` hears every click,
// the one on the chosen tab too. The caller says which tab is the Tab stop (`props.tabIndex`).
import type { KeyboardEvent, ReactNode } from 'react';
import type { TabItem } from './Tabs';

export type DraftTabsProps = {
  value: string;
  items: TabItem[];
  'aria-label': string;
  ids?: { trigger?: (value: string) => string; content?: (value: string) => string };
  onPick?: (value: string) => void;
  tab: (item: TabItem) => ReactNode;
  after: (item: TabItem) => ReactNode;
};

type OwnProps = Record<string, unknown> & { onClick?: (event: unknown) => void; onFocus?: (event: unknown) => void };

export default function DraftTabs({ value, items, 'aria-label': ariaLabel, ids, onPick, tab, after }: DraftTabsProps) {
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key) || event.nativeEvent.isComposing) return;
    const tabs = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')];
    const at = tabs.indexOf(document.activeElement as HTMLElement);
    if (!tabs.length || at < 0) return;
    const to = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (at + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    event.preventDefault();
    tabs[to]?.focus();
  };
  return (
    <div role="tablist" aria-label={ariaLabel} onKeyDown={move}>
      {items.map((item) => {
        const selected = item.value === value;
        const { onClick, onFocus, ...own } = (item.props ?? {}) as OwnProps;
        return (
          <div key={item.value} className={item.className}>
            <button
              type="button"
              role="tab"
              id={ids?.trigger?.(item.value)}
              aria-selected={selected}
              aria-controls={selected ? ids?.content?.(item.value) : undefined}
              tabIndex={selected ? 0 : -1}
              {...own}
              onFocus={onFocus as never}
              onClick={(event) => {
                onClick?.(event);
                onPick?.(item.value);
              }}
            >
              {tab(item)}
            </button>
            {after(item)}
          </div>
        );
      })}
    </div>
  );
}
