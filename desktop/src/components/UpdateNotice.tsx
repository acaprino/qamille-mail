// A new version of the installed Professional, downloaded and verified (src-tauri/src/updates.rs): a
// notice that stays until the person acts, never a toast that goes away by itself. «Riavvia e aggiorna»
// quits as «Esci» does, through the close guards: a guard that keeps the window keeps Qamille as it
// is, with the notice still here. Builds without the updater never announce anything, so the notice
// never shows there.
import { useEffect, useState } from 'react';
import Button from '../ui/Button';
import { call, desktop, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import './UpdateNotice.css';

type UpdateInfo = { version: string; notes: string };

export default function UpdateNotice({ onerror }: { onerror: (message: string) => void }) {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    // A package ready before this page loaded (a reload) is asked for; a later one is announced.
    void call<UpdateInfo | null>('update_status')
      .then((ready) => {
        if (!disposed && ready) setUpdate(ready);
      })
      .catch(() => {});
    void import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen<UpdateInfo>('update-available', (event) => setUpdate(event.payload)),
      )
      .then((unlisten) => (disposed ? unlisten() : (stop = unlisten)))
      .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  if (!update) return null;
  const notes = update.notes.trim();
  return (
    <div className="update-notice" role="status">
      <strong>
        {tr(
          `È pronta la versione ${update.version} di Qamille`,
          `Qamille ${update.version} is ready`,
        )}
      </strong>
      <p>
        {tr(
          'Qamille si chiude come con Esci, poi si riapre aggiornato.',
          'Qamille quits as with Quit, then opens again updated.',
        )}
      </p>
      {notes && <p className="update-notice-notes">{notes}</p>}
      <div className="button-row">
        <Button
          variant="primary"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void call('update_install')
              .catch((error) => onerror(errorText(error)))
              .finally(() => setBusy(false));
          }}
        >
          {tr('Riavvia e aggiorna', 'Restart and update')}
        </Button>
      </div>
    </div>
  );
}
