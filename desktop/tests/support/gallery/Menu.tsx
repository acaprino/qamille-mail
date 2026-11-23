// Menu section of the gallery (Task 12): a command menu with every kind of item (an icon, a shortcut,
// a disabled one with its reason, a group of its own, a switch, one that opens a Modal, and a
// destructive one listed first, which the menu moves to the end), plus the same two items in a Popover
// and in a Menu, side by side, so the test can tell that the two look the same.
import { useState } from 'react';
import Button from '../../../src/ui/Button';
import Icon from '../../../src/ui/Icon';
import Menu from '../../../src/ui/Menu';
import Modal from '../../../src/ui/Modal';
import Popover from '../../../src/ui/Popover';

export default function MenuSection() {
  const [picked, setPicked] = useState<string[]>([]);
  const [pinned, setPinned] = useState(false);
  const [dialog, setDialog] = useState(false);
  const pick = (value: string) => () => setPicked((list) => [...list, value]);
  return (
    <section aria-label="Menu" style={{ padding: '24px 48px' }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <Menu
          label="Row actions"
          tip="More actions"
          trigger={
            <Button variant="icon" type="button" aria-label="More row actions">
              <Icon name="more" size={18} />
            </Button>
          }
          items={[
            { value: 'delete', label: 'Delete', icon: 'trash', destructive: true, onSelect: pick('delete') },
            { value: 'print', label: 'Print', icon: 'print', shortcut: 'Ctrl+P', onSelect: pick('print') },
            {
              value: 'download',
              label: 'Download',
              icon: 'download',
              disabled: true,
              title: 'Not while printing',
              onSelect: pick('download'),
            },
            { value: 'archive', label: 'Archive', icon: 'archive', onSelect: pick('archive') },
            {
              value: 'pin',
              label: 'Pin',
              group: 'state',
              checked: pinned,
              onSelect: () => {
                setPinned((on) => !on);
                pick('pin')();
              },
            },
            { value: 'details', label: 'Details', group: 'state', onSelect: () => setDialog(true) },
          ]}
        />
        <output aria-label="Picked">{picked.join(',') || 'none'}</output>
        <Button variant="secondary" type="button">
          Elsewhere
        </Button>
      </div>
      <div style={{ display: 'flex', gap: 240, alignItems: 'center', marginTop: 260 }}>
        <Popover
          label="Popover look"
          trigger={
            <Button variant="secondary" type="button">
              Open the popover
            </Button>
          }
        >
          <button type="button">
            <Icon name="print" size={15} />
            Print
          </button>
          <button type="button">
            <Icon name="archive" size={15} />
            Archive
          </button>
        </Popover>
        <Menu
          label="Menu look"
          trigger={
            <Button variant="secondary" type="button">
              Open the menu
            </Button>
          }
          items={[
            { value: 'print', label: 'Print', icon: 'print', onSelect: pick('look-print') },
            { value: 'archive', label: 'Archive', icon: 'archive', onSelect: pick('look-archive') },
          ]}
        />
      </div>
      {dialog && (
        <Modal size="sm" aria-label="Details" onClose={() => setDialog(false)}>
          <h2>Details</h2>
          <div className="modal-actions">
            <Button variant="primary" type="button" onClick={() => setDialog(false)}>
              Done
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
