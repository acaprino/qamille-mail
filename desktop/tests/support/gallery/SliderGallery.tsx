import { useState } from 'react';
import Slider from '../../../src/ds/primitives/Slider';

export default function SliderGallery({ props = {} }: { props?: Record<string, any> }) {
  const [value, setValue] = useState<number>(props.value ?? 5);
  const [changes, setChanges] = useState<number[]>([]);
  return (
    <section aria-label="Slider" style={{ padding: 48, width: 320 }}>
      <Slider
        value={value}
        min={props.min ?? 0}
        max={props.max ?? 10}
        step={props.step ?? 1}
        aria-label="Volume"
        onChange={(next) => {
          setValue(next);
          setChanges((all) => [...all, next]);
        }}
      />
      <output aria-label="Slider value">{value}</output>
      <output aria-label="Slider changes">{changes.join(',')}</output>
    </section>
  );
}
