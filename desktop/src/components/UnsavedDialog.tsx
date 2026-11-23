import { useRef } from 'react';
import Button from '../ui/Button';
import Modal from '../ui/Modal';
import { tr } from '../lib/i18n';

export type UnsavedDialogProps = { save: () => void; discard: () => void; stay: () => void };

export default function UnsavedDialog({ save, discard, stay }: UnsavedDialogProps) {
  // The focus starts on Keep editing, not on the first button: Enter right away throws nothing away.
  const keep = useRef<HTMLButtonElement>(null);
  return (
    <Modal
      size="sm"
      className="unsaved-dialog"
      aria-labelledby="unsaved-title"
      onClose={stay}
      initialFocus={() => keep.current}
    >
      <h2 id="unsaved-title">{tr('Vuoi salvare le modifiche?', 'Save your changes?')}</h2>
      <p>
        {tr(
          'Le impostazioni contengono modifiche non ancora salvate.',
          'Your settings have changes that are not saved yet.',
        )}
      </p>
      <div className="modal-actions">
        {/* Throwing the changes away stands apart, at the left, from the two that keep them. */}
        <Button variant="secondary" className="modal-action-apart" onClick={discard}>
          {tr('Annulla modifiche', 'Discard')}
        </Button>
        <Button ref={keep} variant="secondary" onClick={stay}>
          {tr('Continua a modificare', 'Keep editing')}
        </Button>
        <Button variant="primary" onClick={save}>
          {tr('Salva', 'Save')}
        </Button>
      </div>
    </Modal>
  );
}
