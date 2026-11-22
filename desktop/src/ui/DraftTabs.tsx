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
  // Sigrid's tabs give their own ids and point the chosen tab at a panel of their own: the caller's panels have other ids.
  useLayoutEffect(() => {
    const triggers = [...(root.current?.querySelectorAll<HTMLElement>('[role="tab"]') ?? [])];
    triggers.forEach((element, index) => {
      const item = items[index];
      if (!item) return;
      const id = ids?.trigger?.(item.value);
      if (id) element.id = id;
      const panel = ids?.content?.(item.value);
      if (panel && item.value === value) element.setAttribute('aria-controls', panel);
      else element.removeAttribute('aria-controls');
    });
  });
  return (
    <div ref={root} style={{ display: 'contents' }}>
      <SigridTabs value={value} activation="manual" onValueChange={(next) => onPick?.(next)}>
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
