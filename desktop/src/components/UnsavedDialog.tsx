import Button from '../ds/primitives/Button';
import Modal from '../ds/primitives/Modal';
import { tr } from '../lib/i18n';

export type UnsavedDialogProps = { save: () => void; discard: () => void; stay: () => void };

export default function UnsavedDialog({ save, discard, stay }: UnsavedDialogProps) {
  return (
    <Modal size="sm" className="unsaved-dialog" aria-labelledby="unsaved-title" onClose={stay}>
      <h2 id="unsaved-title">{tr('Vuoi salvare le modifiche?', 'Save your changes?')}</h2>
      <p>
        {tr(
          'Le impostazioni contengono modifiche non ancora salvate.',
          'Your settings have changes that are not saved yet.',
        )}
      </p>
      <div className="modal-actions">
        <Button variant="secondary" onClick={stay}>
          {tr('Continua a modificare', 'Keep editing')}
        </Button>
        <Button variant="secondary" onClick={discard}>
          {tr('Annulla modifiche', 'Discard')}
        </Button>
        <Button variant="primary" onClick={save}>
          {tr('Salva', 'Save')}
        </Button>
      </div>
    </Modal>
  );
}
