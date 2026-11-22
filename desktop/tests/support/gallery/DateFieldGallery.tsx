import { useState } from 'react';
import DateField from '../../../src/ui/DateField';
import Modal from '../../../src/ui/Modal';
import SidePanel from '../../../src/ui/SidePanel';

export default function DateFieldGallery({ props = {} }: { props?: Record<string, any> }) {
  const [value, setValue] = useState<string>(props.value ?? '');
  const [closes, setCloses] = useState(0);
  const close = () => setCloses((n) => n + 1);
  const field = (
    <>
      <DateField
        value={value}
        onChange={setValue}
        aria-label="Due date"
        min={props.min}
        max={props.max}
        locale={props.locale}
        disabled={props.disabled}
      />
      <button type="button">After</button>
    </>
  );
  return (
    <section aria-label="DateField" style={{ padding: 48, width: 280 }}>
      {props.modal ? (
        <Modal aria-label="Due" onClose={close} style={{ maxHeight: 260 }}>
          {field}
        </Modal>
      ) : props.panel ? (
        <div style={{ position: 'relative', height: 320, border: '1px solid var(--color-line)' }}>
          <SidePanel aria-label="Due" onScrim={close}>
            {field}
          </SidePanel>
        </div>
      ) : (
        field
      )}
      <output aria-label="Date value">{value}</output>
      <output aria-label="Closes">{closes}</output>
    </section>
  );
}
