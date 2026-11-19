// SidePanel section of the gallery: the panel fills a positioned pane, as it does in the app.
// The "After the pane" button sits after the pane in the DOM, so Tab can reach it only if the
// panel does not trap the focus.
import { useRef, useState } from 'react';
import Button from '../../../src/ds/primitives/Button';
import SidePanel from '../../../src/ds/primitives/SidePanel';

export default function SidePanelSection() {
  const [open, setOpen] = useState(false);
  const [narrow, setNarrow] = useState(false);
  const [scrims, setScrims] = useState(0);
  const [closes, setCloses] = useState(0);
  const [opened, setOpened] = useState(0);
  const closeButton = useRef<HTMLButtonElement>(null);
  return (
    <section aria-label="Side panel" style={{ padding: '24px 48px' }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center', marginBottom: 12 }}>
        <Button
          variant="secondary"
          type="button"
          onClick={() => {
            setNarrow(false);
            setOpen(true);
          }}
        >
          Open wide
        </Button>
        <Button
          variant="secondary"
          type="button"
          onClick={() => {
            setNarrow(true);
            setOpen(true);
          }}
        >
          Open narrow
        </Button>
        <output aria-label="Scrim count">{scrims}</output>
        <output aria-label="Close count">{closes}</output>
        <output aria-label="Open count">{opened}</output>
      </div>
      <div style={{ position: 'relative', height: 420, border: '1px solid var(--border)' }}>
        <p style={{ padding: 12 }}>The pane behind the panel.</p>
        {open && (
          <SidePanel
            narrow={narrow}
            aria-label="Details"
            onScrim={() => setScrims((n) => n + 1)}
            onClose={() => {
              setCloses((n) => n + 1);
              setOpen(false);
            }}
            onOpen={() => {
              setOpened((n) => n + 1);
              closeButton.current?.focus();
            }}
          >
            <div className="side-panel-bar">
              <h2 className="panel-title">Details</h2>
              <Button variant="icon" className="panel-close" aria-label="Close details" ref={closeButton} onClick={() => setOpen(false)}>
                x
              </Button>
            </div>
            <div className="side-panel-body">
              <p style={{ padding: 12 }}>Panel body.</p>
              <Button variant="secondary" type="button">
                Inside action
              </Button>
            </div>
          </SidePanel>
        )}
      </div>
      <Button variant="secondary" type="button" style={{ marginTop: 12 }}>
        After the pane
      </Button>
    </section>
  );
}
