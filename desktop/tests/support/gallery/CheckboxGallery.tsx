// Checkbox and Switch, side by side: the same props, the same tests (primitives-controls.mjs).
import { useState } from 'react';
import Checkbox from '../../../src/ds/primitives/Checkbox';
import Switch from '../../../src/ds/primitives/Switch';

export default function CheckboxGallery({ props = {} }: { props?: Record<string, any> }) {
  const [box, setBox] = useState<boolean>(props.checked ?? false);
  const [boxChanges, setBoxChanges] = useState<boolean[]>([]);
  const [toggle, setToggle] = useState<boolean>(props.checked ?? false);
  const [toggleChanges, setToggleChanges] = useState<boolean[]>([]);
  return (
    <section aria-label="Checkbox and Switch" style={{ padding: 48, display: 'grid', gap: 16, justifyItems: 'start' }}>
      <Checkbox
        checked={box}
        onChange={(value) => {
          setBox(value);
          setBoxChanges((all) => [...all, value]);
        }}
      >
        Notify me
      </Checkbox>
      <Checkbox checked={true} disabled>
        Locked box
      </Checkbox>
      <output aria-label="Box state">{String(box)}</output>
      <output aria-label="Box changes">{boxChanges.join(',')}</output>

      <Switch
        checked={toggle}
        onChange={(value) => {
          setToggle(value);
          setToggleChanges((all) => [...all, value]);
        }}
      >
        Dark mode
      </Switch>
      <Switch checked={false} disabled>
        Locked switch
      </Switch>
      <output aria-label="Switch state">{String(toggle)}</output>
      <output aria-label="Switch changes">{toggleChanges.join(',')}</output>
    </section>
  );
}
