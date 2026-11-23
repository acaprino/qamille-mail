// The strip of open drafts above the composer: a tab for each draft with what is inside it (an icon, the title, a mark when it
// has unsaved changes) and a button after it that closes it. Drawn by Sigrid's Tabs (its `TabsTrigger` takes the caller's own
// attributes and handlers, and `end`, the button beside the tab). Activation is manual: the arrows, Home and End move the focus,
// and Enter or Space picks. `onPick` hears a pick of another tab (a click, or Enter or Space on it) and every click or Enter on
// the chosen one, which the composer uses to minimize it. The caller says which tab is the Tab stop (`props.tabIndex`).
import { useLayoutEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Tabs as SigridTabs, TabsList, TabsTrigger } from '@acaprino/sigrid';
import { imeComposing } from '../lib/ime';
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

type OwnProps = Record<string, unknown> & { onClick?: (event: unknown) => void; onKeyDown?: (event: KeyboardEvent<HTMLButtonElement>) => void };

export default function DraftTabs({ value, items, 'aria-label': ariaLabel, ids, onPick, tab, after }: DraftTabsProps) {
  const root = useRef<HTMLDivElement>(null);
  // The ids the caller asked for go to Sigrid (Ark finds the tabs by them for the arrows); a tab points at a panel only when named.
  useLayoutEffect(() => {
    if (ids?.content) return;
    root.current?.querySelectorAll<HTMLElement>('[role="tab"]').forEach((element) => element.removeAttribute('aria-controls'));
  });
  return (
    <div ref={root} style={{ display: 'contents' }}>
      <SigridTabs value={value} activation="manual" onValueChange={(next) => onPick?.(next)} {...(ids ? { ids } : {})}>
        <TabsList aria-label={ariaLabel}>
          {items.map((item) => {
            const { onClick, onKeyDown, ...own } = (item.props ?? {}) as OwnProps;
            const chosen = item.value === value;
            return (
              <TabsTrigger
                key={item.value}
                value={item.value}
                itemClassName={item.className}
                end={after(item)}
                {...own}
                // Another tab is picked through the value (a click, Enter or Space); the chosen one is picked here.
                onClick={(event) => {
                  onClick?.(event);
                  if (chosen) onPick?.(item.value);
                }}
                onKeyDown={(event) => {
                  onKeyDown?.(event);
                  if (chosen && (event.key === 'Enter' || event.key === ' ') && !imeComposing(event.nativeEvent)) {
                    event.preventDefault();
                    onPick?.(item.value);
                  }
                }}
              >
                {tab(item)}
              </TabsTrigger>
            );
          })}
        </TabsList>
      </SigridTabs>
    </div>
  );
}
