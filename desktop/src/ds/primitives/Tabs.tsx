// The row of tabs of a list: only the tablist is drawn, the content under it stays with the caller.
// The chosen tab carries unlumen's sliding highlight (a motion layoutId, as in unlumen's tabs and
// highlight primitives): an underline under it in `underline`, a raised chip behind it in `segment`;
// `plain` draws nothing of its own. Arrows, Home and End move, and choose unless `activation` is
// `manual`. `ids` names the tab and its panel by value, `props` on an item goes on its tab, `tab`
// draws what is inside the tab, `after` draws what sits next to it (a close button), and `onPick`
// hears every click, the one on the chosen tab too. Without `ids` no tabpanel is drawn, so a tab
// carries no aria-controls. The look is in ds/styles/ark/tabs.css.
import { Tabs as ArkTabs } from '@ark-ui/react/tabs';
import { motion } from 'motion/react';
import { useEffect, useId, useRef, type ReactNode } from 'react';

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
  variant?: 'underline' | 'segment' | 'plain';
  'aria-label': string;
  activation?: 'automatic' | 'manual';
  ids?: { trigger?: (value: string) => string; content?: (value: string) => string };
  onPick?: (value: string) => void;
  tab?: (item: TabItem) => ReactNode;
  after?: (item: TabItem) => ReactNode;
};

function Trigger({
  item,
  selected,
  group,
  variant,
  linked,
  onPick,
  tab,
}: {
  item: TabItem;
  selected: boolean;
  group: string;
  variant: string;
  linked: boolean;
  onPick?: (value: string) => void;
  tab?: (item: TabItem) => ReactNode;
}) {
  const node = useRef<HTMLButtonElement>(null);
  // Ark gives the selected tab aria-controls (a panel id). Without `ids` no panel exists, so drop it,
  // and again whenever Ark writes it back after a change of tab.
  useEffect(() => {
    const element = node.current;
    if (linked || !element) return;
    const drop = () => element.removeAttribute('aria-controls');
    drop();
    const watch = new MutationObserver(drop);
    watch.observe(element, { attributes: true, attributeFilter: ['aria-controls'] });
    return () => watch.disconnect();
  }, [linked]);
  return (
    <ArkTabs.Trigger
      ref={node}
      value={item.value}
      {...item.props}
      onClick={(event) => {
        // The item's own click handler (in `props`) and `onPick` both run, as Svelte merged them.
        (item.props?.onClick as ((event: unknown) => void) | undefined)?.(event);
        onPick?.(item.value);
      }}
    >
      {selected && variant !== 'plain' && (
        <motion.span
          aria-hidden="true"
          data-part="highlight"
          layoutId={`tabs-highlight-${group}`}
          transition={{ type: 'spring', stiffness: 500, damping: 38 }}
        />
      )}
      <span data-part="trigger-label">
        {tab ? (
          tab(item)
        ) : (
          <>
            {item.label}
            {item.count !== undefined && <span className="count">{item.count}</span>}
          </>
        )}
      </span>
    </ArkTabs.Trigger>
  );
}

export default function Tabs({
  value,
  onChange,
  items,
  variant = 'underline',
  'aria-label': ariaLabel,
  activation = 'automatic',
  ids,
  onPick,
  tab,
  after,
}: TabsProps) {
  const group = useId();
  return (
    <ArkTabs.Root
      value={value}
      onValueChange={(details) => onChange?.(details.value)}
      activationMode={activation}
      ids={ids}
      data-variant={variant}
    >
      <ArkTabs.List aria-label={ariaLabel}>
        {items.map((item) => {
          const trigger = (
            <Trigger
              key={item.value}
              item={item}
              selected={item.value === value}
              group={group}
              variant={variant}
              linked={!!ids}
              onPick={onPick}
              tab={tab}
            />
          );
          return after ? (
            <div key={item.value} data-part="item" className={item.className}>
              {trigger}
              {after(item)}
            </div>
          ) : (
            trigger
          );
        })}
      </ArkTabs.List>
    </ArkTabs.Root>
  );
}
