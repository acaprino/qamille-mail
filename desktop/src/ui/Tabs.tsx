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
  // The ids the caller asked for go to Sigrid (Ark finds the tabs by them for the arrows). A tab points at a panel only when
  // the caller named it: without a name Ark's own panel does not exist here.
  useLayoutEffect(() => {
    if (ids?.content) return;
    root.current?.querySelectorAll<HTMLElement>('[role="tab"]').forEach((element) => element.removeAttribute('aria-controls'));
  });
  return (
    <div ref={root} style={{ display: 'contents' }}>
      <SigridTabs value={value} onValueChange={(next) => onChange?.(next)} {...(ids ? { ids } : {})}>
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
