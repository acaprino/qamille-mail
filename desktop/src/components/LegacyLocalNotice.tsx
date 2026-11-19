// The one-time notice about the old local compute mode (wave 2, E10): semantic search and
// transcription now need a server. It points to the Docker guide, offers to free the space of the
// downloaded models (always behind a confirmation in this window, never a browser `confirm()`)
// and goes away for good with «Ho capito».
import { useEffect, useRef, useState } from 'react';
import Button from '../ds/primitives/Button';
import Modal from '../ds/primitives/Modal';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { SERVER_GUIDE_URL } from '../lib/types';
import './LegacyLocalNotice.css';

type LegacyLocalStatus = { show: boolean; models_bytes: number; had_server: boolean; guide_url: string };

function gigabytes(bytes: number): string {
  const value = bytes / 1_000_000_000;
  return value >= 0.1 ? `${value.toFixed(1)} GB` : `${Math.max(1, Math.round(bytes / 1_000_000))} MB`;
}

export default function LegacyLocalNotice({ onerror }: { onerror: (message: string) => void }) {
  const [status, setStatus] = useState<LegacyLocalStatus | null>(null);
  const [confirming, setConfirming] = useState(false);
  const keepModels = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    void call<LegacyLocalStatus>('legacy_local_status')
      .then((value) => live && setStatus(value))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  async function run(work: () => Promise<void>) {
    setBusy(true);
    try {
      await work();
      setStatus(await call<LegacyLocalStatus>('legacy_local_status'));
    } catch (e) {
      onerror(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  if (!status?.show) return null;
  const guide = status.guide_url || SERVER_GUIDE_URL;
  return (
    <>
      <div className="legacy-local-notice" role="status">
        <strong>{tr('Il calcolo sul tuo PC è cambiato', 'The compute on your PC has changed')}</strong>
        <p>
          {tr(
            'Ricerca per significato e trascrizione ora richiedono un server. Puoi avviarlo sul tuo PC con Docker.',
            'Semantic search and transcription now require a server. You can start it on your PC with Docker.',
          )}
        </p>
        <div className="button-row">
          <Button variant="secondary" onClick={() => void call('open_link', { url: guide }).catch((e) => onerror(errorText(e)))}>
            {tr('Apri la guida', 'Open the guide')}
          </Button>
          {status.models_bytes > 0 && (
            <Button variant="secondary" disabled={busy} onClick={() => setConfirming(true)}>
              {tr(`Libera spazio (${gigabytes(status.models_bytes)})`, `Free space (${gigabytes(status.models_bytes)})`)}
            </Button>
          )}
          <Button variant="primary" disabled={busy} onClick={() => void run(() => call('legacy_local_dismiss'))}>
            {tr('Ho capito', 'I understand')}
          </Button>
        </div>
      </div>
      {confirming && (
        <Modal size="sm" aria-labelledby="legacy-free-title" onClose={() => setConfirming(false)} initialFocus={() => keepModels.current}>
          <h2 id="legacy-free-title">{tr('Liberare lo spazio dei modelli?', 'Free the space of the models?')}</h2>
          <p>
            {tr(
              `Si cancellano i modelli scaricati dalla modalità locale (${gigabytes(status.models_bytes)}). Posta e indice restano.`,
              `The models downloaded by the local mode are deleted (${gigabytes(status.models_bytes)}). Mail and the index stay.`,
            )}
          </p>
          <div className="button-row">
            <Button
              variant="primary"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await call('legacy_local_free_space', { confirmed: true });
                  setConfirming(false);
                })
              }
            >
              {tr('Libera spazio', 'Free space')}
            </Button>
            <Button ref={keepModels} variant="text" onClick={() => setConfirming(false)}>
              {tr('Annulla', 'Cancel')}
            </Button>
          </div>
        </Modal>
      )}
    </>
  );
}
