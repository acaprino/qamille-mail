import { useEffect } from 'react';
import MailtoSettings from './MailtoSettings';
import Switch from '../ds/primitives/Switch';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { useReactive } from '../lib/store';

type AutostartStatus = { enabled: boolean; supported: boolean };

/** Settings → General → Windows: what Qamille does as a Windows app (src-tauri/src/background.rs).
 *  Shown only in the desktop app. Each opening of the section mounts it again, so start at sign-in is
 *  read from Windows every time: Task Manager can turn it off behind Qamille's back. */
export default function DesktopSettings() {
  const state = useReactive(() => ({
    closeToTray: true,
    autostart: { enabled: false, supported: false } as AutostartStatus,
    busy: false,
    error: '',
  }));

  useEffect(() => {
    let current = true;
    void Promise.all([call<boolean>('close_to_tray_get'), call<AutostartStatus>('autostart_status')])
      .then(([closeToTray, autostart]) => {
        if (!current) return;
        state.closeToTray = closeToTray;
        state.autostart = autostart;
      })
      .catch((error) => current && (state.error = errorText(error)));
    return () => {
      current = false;
    };
  }, []);

  async function setCloseToTray(on: boolean) {
    state.error = '';
    state.closeToTray = on;
    try {
      await call('close_to_tray_set', { on });
    } catch (error) {
      state.closeToTray = !on;
      state.error = errorText(error);
    }
  }
  async function setAutostart(enabled: boolean) {
    state.error = '';
    state.busy = true;
    try {
      state.autostart = await call<AutostartStatus>('autostart_set', { enabled });
    } catch (error) {
      state.error = errorText(error);
      // What Windows has now, whatever failed.
      state.autostart = await call<AutostartStatus>('autostart_status').catch(() => state.autostart);
    } finally {
      state.busy = false;
    }
  }

  return (
    <>
      <h3>Windows</h3>
      {state.error && (
        <p className="field-help" role="alert">
          {state.error}
        </p>
      )}
      <div className="setting-row">
        <Switch checked={state.closeToTray} onChange={(on) => void setCloseToTray(on)}>
          <strong>{tr('Chiudendo la finestra, Qamille resta nell’area di notifica', 'Closing the window keeps Qamille in the notification area')}</strong>
        </Switch>
        <p>
          {tr(
            'Così invia i messaggi programmati, mostra i promemoria e riceve la posta anche a finestra chiusa. Per uscire usa Esci dal menu dell’icona o dal menu della finestra (Alt+Spazio).',
            'It keeps sending scheduled messages, showing reminders and receiving mail with the window closed. To quit, use Quit in the icon’s menu or in the window menu (Alt+Space).',
          )}
        </p>
      </div>
      {/* phase 3, Task 3: notifications */}
      {state.autostart.supported && (
        <div className="setting-row">
          <Switch checked={state.autostart.enabled} disabled={state.busy} onChange={(on) => void setAutostart(on)}>
            <strong>{tr('Avvia Qamille all’accesso a Windows, nell’area di notifica', 'Start Qamille at Windows sign-in, in the notification area')}</strong>
          </Switch>
          <p>
            {tr(
              'Si può spegnere anche da Gestione attività, scheda App di avvio.',
              'It can also be turned off in Task Manager, Startup apps.',
            )}
          </p>
        </div>
      )}
      <MailtoSettings />
    </>
  );
}
