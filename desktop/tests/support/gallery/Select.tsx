// The Select section of the gallery: a Language select, a disabled item, and a window keydown
// listener that stands for App.tsx's shortcut handler (same guard, shortcutIgnored): it counts
// the keys the page would have acted on, so a test can see that a key typed in the list is not one.
import { useEffect, useState } from 'react';
import Modal from '../../../src/ds/primitives/Modal';
import Select from '../../../src/ds/primitives/Select';
import SidePanel from '../../../src/ds/primitives/SidePanel';
import { shortcutIgnored } from '../../../src/lib/shortcut-guard';

const items = [
  { value: 'it', label: 'Italiano' },
  { value: 'en', label: 'English' },
  { value: 'es', label: 'Español' },
  { value: 'de', label: 'Deutsch', disabled: true },
  { value: 'fr', label: 'Français' },
];

export default function SelectSection({ props = {} }: { props?: Record<string, any> }) {
  const [value, setValue] = useState('it');
  const [changes, setChanges] = useState(0);
  const [shortcuts, setShortcuts] = useState(0);
  const [closes, setCloses] = useState(0);
  useEffect(() => {
    const keyboard = (e: KeyboardEvent) => {
      if (shortcutIgnored(e.target) || e.ctrlKey || e.metaKey || e.altKey) return;
      if (['n', 'r', 'e'].includes(e.key.toLowerCase())) setShortcuts((n) => n + 1);
    };
    window.addEventListener('keydown', keyboard);
    return () => window.removeEventListener('keydown', keyboard);
  }, []);
  const close = () => setCloses((n) => n + 1);
  const body = (
    <>
      <button type="button" id="before">
        Before
      </button>
      <Select
        value={value}
        items={items}
        label="Language"
        onChange={(next) => {
          setValue(next);
          setChanges((n) => n + 1);
        }}
      />
      <Select value="" items={items} aria-label="Spare" disabled />
      <Select value="" items={items} aria-label="Needed" aria-required="true" aria-invalid="true" />
    </>
  );
  return (
    <section aria-label="Select" style={{ padding: 48, display: 'flex', flexDirection: 'column', gap: 16, width: 280 }}>
      {props.modal ? (
        <Modal aria-label="Preferences" onClose={close} style={{ overflow: 'auto', maxHeight: 220 }}>
          {body}
        </Modal>
      ) : props.panel ? (
        <div style={{ position: 'relative', height: 320, border: '1px solid var(--border)' }}>
          <SidePanel aria-label="Preferences" onScrim={close}>
            {body}
          </SidePanel>
        </div>
      ) : (
        body
      )}
      <output id="value" aria-label="Value">
        {value}
      </output>
      <output id="changes" aria-label="Changes">
        {changes}
      </output>
      <output id="closes" aria-label="Closes">
        {closes}
      </output>
      <output id="shortcuts" aria-label="Shortcuts">
        {shortcuts}
      </output>
    </section>
  );
}
