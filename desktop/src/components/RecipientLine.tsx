import { useRef, useState } from 'react';
import Button from '../ds/primitives/Button';
import { tr } from '../lib/i18n';
import { recipientView, type PersonIndex } from '../lib/people';
import { tick } from '../lib/tick';

export type RecipientLineProps = {
  label: string;
  kind: 'to' | 'cc';
  addresses: string[];
  index: PersonIndex;
  self: Set<string>;
};

export default function RecipientLine({ label, kind, addresses, index, self }: RecipientLineProps) {
  const [open, setOpen] = useState(false);
  const view = recipientView(addresses, index, self, tr('te', 'you'));
  const showMore = useRef<HTMLButtonElement>(null);
  const showLess = useRef<HTMLButtonElement>(null);
  // The control that replaces the one just used takes the focus, without moving the pane.
  async function toggle(next: boolean) {
    setOpen(next);
    await tick();
    (next ? showLess : showMore).current?.focus({ preventScroll: true });
  }

  if (!view.all.length) return null;
  return (
    <div className="recipient-line" data-kind={kind}>
      {open ? (
        <>
          <div className="recipient-head">
            <span className="recipient-label">{label}</span>
            <Button ref={showLess} variant="text" className="recipient-toggle" onClick={() => void toggle(false)}>
              {tr('Mostra meno', 'Show less')}
            </Button>
          </div>
          {/* A scrolling box has to take the focus, or the keyboard cannot reach the names below the fold. */}
          <div className="recipient-full" role="group" tabIndex={0} aria-label={tr('Tutti i destinatari', 'All recipients')}>
            {view.all.join(', ')}
          </div>
        </>
      ) : (
        <>
          <span className="recipient-label">{label}</span>
          <span className="recipient-names">{view.shown.join(', ')}</span>
          {view.more > 0 && (
            <>
              {', '}
              <Button
                ref={showMore}
                variant="text"
                className="recipient-toggle"
                aria-expanded="false"
                onClick={() => void toggle(true)}
              >
                {tr(`e altri ${view.more}`, `and ${view.more} more`)}
              </Button>
            </>
          )}
        </>
      )}
    </div>
  );
}
