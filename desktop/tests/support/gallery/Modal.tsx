// Modal section of the gallery: three openers, one per size. `onClose` counts and closes.
import { useState } from 'react';
import Button from '../../../src/ui/Button';
import Modal, { ModalFooter } from '../../../src/ui/Modal';

type Size = 'sm' | 'md' | 'lg';

export default function ModalSection() {
  const [size, setSize] = useState<Size | undefined>(undefined);
  const [open, setOpen] = useState(false);
  const [closes, setCloses] = useState(0);
  const show = (next: Size) => {
    setSize(next);
    setOpen(true);
  };
  return (
    <section aria-label="Modal" style={{ padding: 48 }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <Button variant="secondary" type="button" onClick={() => show('sm')}>
          Open small
        </Button>
        <Button variant="secondary" type="button" onClick={() => show('md')}>
          Open medium
        </Button>
        <Button variant="secondary" type="button" onClick={() => show('lg')}>
          Open large
        </Button>
        <output aria-label="Close count">{closes}</output>
      </div>
      <p>Page text behind the dialog.</p>
      {open && (
        <Modal
          size={size}
          aria-labelledby="sample-title"
          onClose={() => {
            setCloses((n) => n + 1);
            setOpen(false);
          }}
        >
          <h2 id="sample-title">Sample dialog</h2>
          <p>Decide before going on.</p>
          <ModalFooter>
            <Button variant="secondary" type="button" onClick={() => setOpen(false)}>
              Keep
            </Button>
            <Button variant="primary" type="button" onClick={() => setOpen(false)}>
              Confirm
            </Button>
          </ModalFooter>
        </Modal>
      )}
    </section>
  );
}
