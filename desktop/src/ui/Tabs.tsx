// A row of tabs for a list or a view: only the tablist is drawn, the content under it stays with the caller. Drawn by
// Sigrid's Tabs (Ark's tabs): the chosen tab on a tinted sheet that slides, the others muted. Arrows, Home and End move
// and choose. `ids` names the tab and its panel by value for a panel the caller draws itself; without `ids` no tab
// carries aria-controls. A count is Sigrid's Count after the label.
import { useLayoutEffect, useRef } from 'react';
import { Count, Tabs as SigridTabs, TabsList, TabsTrigger } from '@acaprino/sigrid';

export type TabItem = {
  value: string;
  label: string;
  count?: number;
  className?: string;
  props?: Record<string, unknown>;
};

export type TabsProps = {
  value: string;
  onChange?: (value: string) => void;
  items: TabItem[];
  'aria-label': string;
  ids?: { trigger?: (value: string) => string; content?: (value: string) => string };
};

export default function Tabs({ value, onChange, items, 'aria-label': ariaLabel, ids }: TabsProps) {
  const root = useRef<HTMLDivElement>(null);
  // Sigrid's tabs give their own ids and point the chosen tab at a panel of their own. The caller's panel has other
  // ids, or none: the tabs take the ids the caller asked for (in the order of `items`, which is the order of the DOM),
  // and a tab points at a panel only when the caller named it.
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
      <SigridTabs value={value} onValueChange={(next) => onChange?.(next)}>
        <TabsList aria-label={ariaLabel}>
          {items.map((item) => (
            <TabsTrigger key={item.value} value={item.value}>
              {item.label}
              {item.count !== undefined && <Count value={item.count} />}
            </TabsTrigger>
          ))}
        </TabsList>
      </SigridTabs>
    </div>
  );
}
