import { useEffect } from 'react';
import Button from '../ds/primitives/Button';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { useReactive } from '../lib/store';
import './MailtoSettings.css';

type MailHandlerStatus = { registered: boolean; name: string; installed: boolean; supported: boolean; chosen: boolean };

/** Settings → General → Windows: Qamille for the mailto: links of other programs (src-tauri/src/
 *  mail_handler.rs). Windows lets only the person choose the default mail app: the button says to Windows
 *  that this copy can open them and opens Settings > Default apps on its entry. The state is read again
 *  each time the window comes back to the front, since the choice is made there. The installed
 *  Professional's installer writes its entry, and removes it: there is no «Smetti» for it. */
export default function MailtoSettings() {
  const state = useReactive(() => ({ status: null as MailHandlerStatus | null, busy: false, error: '' }));

  useEffect(() => {
    let current = true;
    const read = () =>
      void call<MailHandlerStatus>('mail_handler_status')
        .then((status) => {
          if (!current) return;
          // A first reading that failed is over once one succeeds.
          if (!state.status) state.error = '';
          state.status = status;
        })
        .catch((error) => current && (state.error = errorText(error)));
    read();
    window.addEventListener('focus', read);
    return () => {
      current = false;
      window.removeEventListener('focus', read);
    };
  }, []);

  async function run(command: 'mail_handler_register' | 'mail_handler_unregister') {
    state.error = '';
    state.busy = true;
    try {
      await call(command);
    } catch (error) {
      state.error = errorText(error);
    } finally {
      // What Windows has now, whatever failed.
      state.status = await call<MailHandlerStatus>('mail_handler_status').catch(() => state.status);
      state.busy = false;
    }
  }

  const status = state.status;
  const title = <strong>{tr('Link mailto degli altri programmi', 'Mailto links in other programs')}</strong>;
  const error = state.error && (
    <p className="field-help" role="alert">
      {state.error}
    </p>
  );
  // The first reading failed: what Windows has is unknown, but the reason is said, never a missing row.
  if (!status)
    return error ? (
      <div className="setting-row">
        {title}
        {error}
      </div>
    ) : null;
  if (!status.supported) return null;
  return (
    <div className="setting-row">
      {title}
      <p>
        {status.chosen
          ? tr(`Windows apre i link mailto con ${status.name}: diventano un nuovo messaggio, da rileggere e inviare.`, `Windows opens mailto links with ${status.name}: each becomes a new message, to read and send.`)
          : status.registered
            ? tr(`${status.name} è tra le app che Windows offre per i link mailto: sceglila in Impostazioni → App predefinite.`, `${status.name} is among the apps Windows offers for mailto links: choose it in Settings → Default apps.`)
            : tr('Un link mailto in un altro programma può aprire un nuovo messaggio in Qamille. La scelta la fai in Windows: il pulsante apre la pagina delle app predefinite.', 'A mailto link in another program can open a new message in Qamille. You choose it in Windows: the button opens the Default apps page.')}
        {status.registered && !status.installed &&
          tr(' «Smetti» toglie Qamille da quelle app; se era la scelta, Windows chiederà quale usare.', ' “Stop” removes Qamille from those apps; if it was the one chosen, Windows will ask which to use.')}
      </p>
      {error}
      <div className="mailto-actions">
        <Button variant="secondary" disabled={state.busy} onClick={() => void run('mail_handler_register')}>
          {status.registered ? tr('Apri le app predefinite di Windows', 'Open Windows Default apps') : tr('Usa Qamille per i link mailto', 'Use Qamille for mailto links')}
        </Button>
        {status.registered && !status.installed && (
          <Button variant="text" disabled={state.busy} onClick={() => void run('mail_handler_unregister')}>
            {tr('Smetti', 'Stop')}
          </Button>
        )}
      </div>
    </div>
  );
}
