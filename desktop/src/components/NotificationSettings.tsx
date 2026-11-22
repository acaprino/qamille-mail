import { useEffect } from 'react';
import Switch from '../ds/primitives/Switch';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { useReactive } from '../lib/store';

/** `notifications_get` (src-tauri/src/notifications.rs): the switches, and two facts to show them by. */
type NotificationPrefs = {
  enabled: boolean;
  new_mail: boolean;
  scheduled_send: boolean;
  sign_in: boolean;
  /** A copy without an uninstaller: the switch says it writes in the registry of the user. */
  portable: boolean;
  /** False in the sample workspace, which has no switch. */
  supported: boolean;
};
type Kind = 'new_mail' | 'scheduled_send' | 'sign_in';

/** Settings → General → Windows: the Windows notifications (docs/aspetto-nativo.md, rule 28). A general
 *  switch and one per fact. Shown only in the desktop app, inside DesktopSettings. */
export default function NotificationSettings() {
  const state = useReactive(() => ({ prefs: null as NotificationPrefs | null, busy: false, error: '' }));

  useEffect(() => {
    let current = true;
    void call<NotificationPrefs>('notifications_get')
      .then((prefs) => current && (state.prefs = prefs))
      .catch((error) => current && (state.error = errorText(error)));
    return () => {
      current = false;
    };
  }, []);

  async function save(next: NotificationPrefs) {
    const before = state.prefs;
    state.error = '';
    state.busy = true;
    state.prefs = next;
    try {
      await call('notifications_set', { prefs: next });
    } catch (error) {
      // Not saved (Windows refused the registry entry): the switches show what is in force.
      state.prefs = before;
      state.error = errorText(error);
    } finally {
      state.busy = false;
    }
  }

  const prefs = state.prefs;
  const kinds: [Kind, string][] = [
    ['new_mail', tr('Posta nuova in Posta in arrivo', 'New mail in the Inbox')],
    ['scheduled_send', tr('Invii programmati non riusciti', 'Scheduled messages that were not sent')],
    ['sign_in', tr('Accessi da rinnovare (account di posta e server Qamille)', 'Sign-ins to renew (mail accounts and the Qamille server)')],
  ];
  return (
    <>
      {state.error && (
        <p className="field-help" role="alert">
          {state.error}
        </p>
      )}
      {prefs?.supported && (
        <>
          <div className="setting-row">
            <Switch checked={prefs.enabled} disabled={state.busy} onChange={(on) => void save({ ...prefs, enabled: on })}>
              <strong>{tr('Notifiche di Windows', 'Windows notifications')}</strong>
            </Switch>
            <p>
              {tr(
                'Quando Qamille non è in primo piano: dicono mittente e oggetto, mai il testo del messaggio. Con «Non disturbare» di Windows restano nel centro notifiche.',
                'When Qamille is not in front: they show sender and subject, never the text of the message. With Windows’ Do not disturb they wait in the notification center.',
              )}
            </p>
            {prefs.portable && (
              <p>
                {tr(
                  'Accenderle scrive una voce nel registro di Windows del tuo utente, il nome con cui Windows le mostra; spegnerle la toglie.',
                  'Turning them on writes an entry in your user’s Windows registry, the name Windows shows them under; turning them off removes it.',
                )}
              </p>
            )}
          </div>
          {prefs.enabled &&
            kinds.map(([kind, label]) => (
              <div className="setting-row" key={kind}>
                <Switch checked={prefs[kind]} disabled={state.busy} onChange={(on) => void save({ ...prefs, [kind]: on })}>
                  <strong>{label}</strong>
                </Switch>
              </div>
            ))}
        </>
      )}
    </>
  );
}
