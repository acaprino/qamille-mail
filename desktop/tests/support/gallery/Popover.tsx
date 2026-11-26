// Popover section of the gallery, built as the nested scenario of the review: a Popover whose
// item opens a Modal (the Modal stays in the section, so it outlives the Popover's own content),
// plus a side panel and a tipped button so the layer order can be read with all four in use.
import { useState } from 'react';
import Button from '../../../src/ui/Button';
import Modal, { ModalFooter } from '../../../src/ui/Modal';
import Popover from '../../../src/ui/Popover';
import SidePanel from '../../../src/ui/SidePanel';

export default function PopoverSection() {
  const [menu, setMenu] = useState(false);
  const [dialog, setDialog] = useState(false);
  const [panel, setPanel] = useState(false);
  const [picked, setPicked] = useState('none');
  return (
    <section aria-label="Popover" style={{ padding: '24px 48px' }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <Button variant="secondary" type="button" onClick={() => setPanel(true)}>
          Open panel
        </Button>
        <Button variant="icon" type="button" tip="Hint" aria-label="Hint">
          ?
        </Button>
        <Popover
          open={menu}
          onOpenChange={setMenu}
          label="Row actions"
          trigger={
            <Button variant="secondary" type="button">
              Options
            </Button>
          }
        >
          <button
            type="button"
            onClick={() => {
              setPicked('first');
              setMenu(false);
            }}
          >
            First action
          </button>
          <span className="popover-divider"></span>
          <button type="button" onClick={() => setDialog(true)}>
            Open dialog
          </button>
        </Popover>
        <output aria-label="Picked">{picked}</output>
      </div>
      <div style={{ position: 'relative', height: 300, marginTop: 12, border: '1px solid var(--color-line)' }}>
        {panel && (
          <SidePanel narrow aria-label="Side details" onScrim={() => setPanel(false)} onClose={() => setPanel(false)}>
            <div className="side-panel-body">
              <p style={{ padding: 12 }}>Panel body.</p>
            </div>
          </SidePanel>
        )}
      </div>
      {dialog && (
        <Modal size="sm" aria-label="Confirm" onClose={() => setDialog(false)}>
          <h2>Confirm</h2>
          <ModalFooter>
            <Button variant="primary" type="button" onClick={() => setDialog(false)}>
              Done
            </Button>
          </ModalFooter>
        </Modal>
      )}
    </section>
  );
}
