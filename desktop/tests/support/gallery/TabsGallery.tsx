import { useState } from 'react';
import Tabs from '../../../src/ds/primitives/Tabs';

const items = [
  { value: 'all', label: 'All', count: 48 },
  { value: 'unread', label: 'Unread', count: 12 },
  { value: 'files', label: 'With files' },
];

export default function TabsGallery({ props = {} }: { props?: Record<string, any> }) {
  const [value, setValue] = useState<string>(props.value ?? 'all');
  return (
    <section aria-label="Tabs" style={{ padding: 48 }}>
      <Tabs value={value} onChange={setValue} items={items} variant={props.variant} aria-label="Show" />
      <output aria-label="Tab value">{value}</output>
    </section>
  );
}
