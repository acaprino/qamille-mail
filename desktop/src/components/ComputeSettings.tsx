// Settings → Compute (access wave 2, spec A7, A8): no server, or a Qamille server. Nothing runs on this computer.
// Self-hosted builds write the address; the Professional build has its own server and no
// «Change server». Connected: server, user, devices, change password, sign out, delete account.
import { llmModelName } from '../lib/assistant-server';
import { useEffect, useRef, useState } from 'react';
import Button from '../ds/primitives/Button';
import Checkbox from '../ds/primitives/Checkbox';
import Field from '../ds/primitives/Field';
import Modal from '../ds/primitives/Modal';
import Select from '../ds/primitives/Select';
import { call, errorText } from '../lib/api';
import { backendText, tr } from '../lib/i18n';
import type { AccountInfo, DeviceInfo } from '../lib/server-connect';
import { revokeChoice, type MailboxRevoke, type SyncStatus } from '../lib/mailbox-sync';
import { RevokeReport } from './MailboxSync';
import { notify } from '../ds/primitives/Toaster';
import { deleteWarning } from '../lib/subscription-usage';
import { SERVER_GUIDE_URL } from '../lib/types';
import ServerConnect from './server-connect/ServerConnect';
import SubscriptionPanel from './SubscriptionPanel';
import './ComputeSettings.css';

export type AppEdition = { kind: 'self-hosted' | 'professional'; server?: string; default_device_name: string };
type Status = {
  connected: boolean;
  url: string | null;
  username: string | null;
  device_id: string | null;
  compute_mode: 'server' | 'off';
  consented_urls: string[];
};

type ServerModel = { id: string; state: string; percent: number | null };
type Identity = { model: string; dimensions: number; quantization: string };
/** `compute_status`: what the remote server says to this device (D7). Never a key. */
type RemoteStatus = {
  route: 'server' | 'off' | 'not_connected';
  url: string | null;
  reachable: boolean;
  unlinked: boolean;
  version: string | null;
  models: ServerModel[];
  model_mismatch: { server: Identity; index: Identity } | null;
};

function modelName(id: string): string {
  if (id === 'embeddinggemma-2') return 'EmbeddingGemma 2';
  if (id.startsWith('whisper-')) return `Whisper ${id.slice('whisper-'.length)}`;
  const assistant = llmModelName(id);
  if (assistant) return tr(assistant.it, assistant.en);
  return id;
}
function modelState(model: ServerModel): string {
  switch (model.state) {
    case 'ready':
    case 'loading':
      return tr('pronto', 'ready');
    case 'downloading':
      return model.percent === null ? tr('in download', 'downloading') : tr(`in download ${model.percent}%`, `downloading ${model.percent}%`);
    default:
      return tr('mancante', 'missing');
  }
}

export default function ComputeSettings() {
  const [edition, setEdition] = useState<AppEdition | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [connecting, setConnecting] = useState<'connect' | 'change' | 'reconnect' | null>(null);
  const [remote, setRemote] = useState<RemoteStatus | null>(null);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [signOutOthers, setSignOutOthers] = useState(true);
  const [deleting, setDeleting] = useState(false);
  const keepAccount = useRef<HTMLButtonElement>(null);
  const [deletePassword, setDeletePassword] = useState('');
  // «Disattiva» of another device: the choice to revoke the mailboxes' access, and what it did (S5).
  const [disabling, setDisabling] = useState<DeviceInfo | null>(null);
  const [revokeMailboxes, setRevokeMailboxes] = useState(true);
  const [revoked, setRevoked] = useState<MailboxRevoke[]>([]);
  const [mailboxSync, setMailboxSync] = useState<SyncStatus | null>(null);
  // `GET /v1/account`: subscription, password or Google only. Absent on older servers.
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const professional = edition?.kind === 'professional';
  const passwordless = account !== null && !account.has_password;

  async function load() {
    try {
      const [e, s] = await Promise.all([call<AppEdition>('app_edition'), call<Status>('server_status')]);
      setEdition(e);
      setStatus(s);
      setError('');
      setRemote(s.connected && s.compute_mode === 'server' ? await call<RemoteStatus>('compute_status').catch(() => null) : null);
      if (s.connected) setDevices(await call<DeviceInfo[]>('server_devices').catch(() => []));
      setAccount(s.connected ? await call<AccountInfo>('server_account_info').catch(() => null) : null);
      setMailboxSync(s.connected ? await call<SyncStatus>('sync_status').catch(() => null) : null);
    } catch (e) {
      setError(errorText(e));
    }
  }
  useEffect(() => {
    void load();
  }, []);
  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  if (!edition || !status) return <h2>{tr('Calcolo', 'Compute')}</h2>;
  const serverLabel = tr('Un server Qamille', 'A Qamille server');

  return (
    <>
      <h2>{tr('Calcolo', 'Compute')}</h2>
      <p className="muted">
        {tr(
          "Dove girano indicizzazione, ricerca per significato e trascrizione con il server. Senza server restano la posta e la ricerca per parole.",
          'Where indexing, semantic search and transcription run, with the server. Without a server the mail and the search by words remain.',
        )}{' '}
        <a
          href={SERVER_GUIDE_URL}
          onClick={(e) => {
            e.preventDefault();
            call('open_link', { url: SERVER_GUIDE_URL }).catch((error) => setError(errorText(error)));
          }}
        >
          {tr('Server sul tuo PC con Docker', 'Server on your PC with Docker')}
        </a>
      </p>
      <Select
        label={tr('Calcolo', 'Compute')}
        value={status.compute_mode}
        items={[
          { value: 'off', label: tr('Nessun server', 'No server') },
          { value: 'server', label: serverLabel },
        ]}
        disabled={busy}
        onChange={(mode) =>
          void run(async () => {
            if (mode === 'server' && !status.connected) return setConnecting('connect');
            await call('compute_set_mode', { mode });
            await load();
          })
        }
      />
      {connecting && (
        <ServerConnect
          preset={professional ? edition.server : null}
          consented={status.consented_urls}
          defaultDeviceName={edition.default_device_name}
          changeServer={connecting === 'change'}
          reconnect={connecting === 'reconnect' ? status.url : null}
          onDone={() => {
            setConnecting(null);
            void load();
          }}
          onCancel={() => setConnecting(null)}
        />
      )}
      {!connecting && !status.connected && (
        <div className="button-row">
          <Button variant="secondary" onClick={() => setConnecting('connect')}>
            {professional ? tr('Accedi a Qamille', 'Sign in to Qamille') : tr('Collegati a un server', 'Connect to a server')}
          </Button>
        </div>
      )}
      {!connecting && status.connected && (
        <section className="compute-account" aria-label={tr('Account del server', 'Server account')}>
          {!professional && (
            <p>
              {tr('Server', 'Server')}: <strong>{status.url}</strong>
            </p>
          )}
          <p>
            {tr('Utente', 'User')}: <strong>{status.username}</strong>
            {account?.identities.includes('google') && <span className="muted"> · {tr('accesso con Google', 'signs in with Google')}</span>}
          </p>
          {account?.billing_enabled && !account.subscription?.entitled && status.compute_mode === 'server' && (
            <div className="compute-alert" role="alert">
              <p>
                {tr(
                  "L'abbonamento non è attivo: il calcolo sul server è in pausa.",
                  'The subscription is not active: compute on the server is paused.',
                )}
              </p>
              <div className="button-row">
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await call('compute_set_mode', { mode: 'off' });
                      await load();
                    })
                  }
                >
                  {tr('Continua senza server', 'Continue without a server')}
                </Button>
              </div>
            </div>
          )}
          {account && status.url && (
            <SubscriptionPanel url={status.url} account={account} computeMode={status.compute_mode} onChanged={load} />
          )}
          {remote?.unlinked && (
            <div className="compute-alert" role="alert">
              <p>{backendText('This device is no longer connected to the server')}</p>
              <div className="button-row">
                <Button variant="primary" disabled={busy} onClick={() => setConnecting('reconnect')}>
                  {tr('Accedi di nuovo', 'Sign in again')}
                </Button>
              </div>
            </div>
          )}
          {remote?.model_mismatch && (
            <div className="compute-alert" role="alert">
              <p>
                {tr(
                  `Il server usa un modello diverso dall'indice (server: ${modelName(remote.model_mismatch.server.model)}, ${remote.model_mismatch.server.dimensions} dimensioni; indice: ${modelName(remote.model_mismatch.index.model)}, ${remote.model_mismatch.index.dimensions} dimensioni). Finché non reindicizzi, la ricerca trova solo le parole.`,
                  `The server uses a different model from the index (server: ${modelName(remote.model_mismatch.server.model)}, ${remote.model_mismatch.server.dimensions} dimensions; index: ${modelName(remote.model_mismatch.index.model)}, ${remote.model_mismatch.index.dimensions} dimensions). Until you reindex, search finds words only.`,
                )}
              </p>
              <div className="button-row">
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await call('search_reindex');
                      notify(tr('Reindicizzazione avviata', 'Reindexing started'));
                      await load();
                    })
                  }
                >
                  {tr('Reindicizza', 'Reindex')}
                </Button>
              </div>
            </div>
          )}
          {remote && !remote.unlinked && (
            <>
              <h3>{tr('Modelli del server', 'Server models')}</h3>
              {remote.reachable ? (
                <ul className="server-models" aria-label={tr('Modelli del server', 'Server models')}>
                  {remote.models.map((model) => (
                    <li key={model.id}>
                      <span>{modelName(model.id)}</span>
                      <span className="muted">{modelState(model)}</span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted">{tr('Il server non risponde: riprovo dal prossimo passo.', 'The server does not answer: trying again from the next step.')}</p>
              )}
              <p className="field-help">
                {tr(
                  "I modelli del server li installa il suo amministratore.",
                  'Models on the server are installed by its administrator.',
                )}
              </p>
            </>
          )}
          <h3>{tr('Dispositivi', 'Devices')}</h3>
          <ul className="server-devices">
            {devices.map((device) => (
              <li key={device.id}>
                <span>
                  {device.name}
                  {device.current && <em className="muted"> · {tr('questo dispositivo', 'this device')}</em>}
                </span>
                {!device.current && (
                  <Button
                    variant="secondary"
                    disabled={busy}
                    onClick={() => {
                      // The mailboxes' access can be revoked only where the synchronization shares them.
                      if (mailboxSync?.consented) {
                        setRevokeMailboxes(revokeChoice(device).checked);
                        setDisabling(device);
                        return;
                      }
                      void run(async () => {
                        await call('server_revoke_device', { deviceId: device.id });
                        setDevices(await call<DeviceInfo[]>('server_devices'));
                      });
                    }}
                  >
                    {tr('Disattiva', 'Disable')}
                  </Button>
                )}
              </li>
            ))}
          </ul>
          {disabling && (
            <div className="server-disable" role="group" aria-label={tr('Disattiva il dispositivo', 'Disable the device')}>
              <p>
                {tr(
                  `Disattivare «${disabling.name}»? Da ora non riceve più nulla dal server.`,
                  `Disable "${disabling.name}"? From now on it receives nothing from the server.`,
                )}
              </p>
              {revokeChoice(disabling).shown && (
                <Checkbox checked={revokeMailboxes} onChange={() => setRevokeMailboxes(!revokeMailboxes)}>
                  {tr('Revoca gli accessi alle caselle', "Revoke the mailboxes' access")}
                </Checkbox>
              )}
              <p className="muted">
                {tr(
                  'Le credenziali che aveva già restano su quel computer. Con la revoca Google invalida i token e dovrai ricollegare le caselle qui; le password per app le revochi presso il fornitore.',
                  'The credentials it already had stay on that computer. Revoking makes Google invalidate the tokens and you connect the mailboxes again here; app passwords are revoked with their provider.',
                )}
              </p>
              <div className="button-row">
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      const device = disabling;
                      // The device goes first: a failed revocation never undoes it.
                      await call('server_revoke_device', { deviceId: device.id });
                      setDisabling(null);
                      setDevices(await call<DeviceInfo[]>('server_devices'));
                      if (revokeMailboxes) setRevoked(await call<MailboxRevoke[]>('sync_revoke_all'));
                    })
                  }
                >
                  {tr('Disattiva', 'Disable')}
                </Button>
                <Button variant="text" onClick={() => setDisabling(null)}>
                  {tr('Annulla', 'Cancel')}
                </Button>
              </div>
            </div>
          )}
          <RevokeReport
            results={revoked}
            busy={busy}
            onRetry={() =>
              void run(async () => {
                const again = await Promise.all(
                  revoked
                    .filter((result) => result.outcome === 'failed')
                    .map((result) => call<MailboxRevoke>('sync_revoke_mailbox', { accountId: result.account_id })),
                );
                setRevoked((old) => old.map((result) => again.find((a) => a.account_id === result.account_id) ?? result));
              })
            }
          />
          {!passwordless && (
          <form
            className="compute-password"
            onSubmit={(e) => {
              e.preventDefault();
              void run(async () => {
                await call('server_change_password', { currentPassword: current, newPassword: next, signOutOtherDevices: signOutOthers });
                setCurrent('');
                setNext('');
                notify(tr('Password cambiata', 'Password changed'));
                setDevices(await call<DeviceInfo[]>('server_devices').catch(() => devices));
              });
            }}
          >
            <h3>{tr('Cambia password', 'Change password')}</h3>
            <Field>
              {tr('Password attuale', 'Current password')}
              <input type="password" value={current} autoComplete="current-password" onChange={(e) => setCurrent(e.currentTarget.value)} />
            </Field>
            <Field>
              {tr('Nuova password', 'New password')}
              <input type="password" value={next} autoComplete="new-password" onChange={(e) => setNext(e.currentTarget.value)} />
            </Field>
            <Checkbox checked={signOutOthers} onChange={() => setSignOutOthers(!signOutOthers)}>
              {tr('Scollega gli altri dispositivi', 'Sign out the other devices')}
            </Checkbox>
            <div className="button-row">
              <Button variant="secondary" type="submit" disabled={busy || !current || next.length < 10}>
                {tr('Cambia password', 'Change password')}
              </Button>
            </div>
          </form>
          )}
          <div className="button-row">
            {!professional && (
              <Button variant="secondary" disabled={busy} onClick={() => setConnecting('change')}>
                {tr('Cambia server', 'Change server')}
              </Button>
            )}
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  const outcome = await call<'signed_out' | 'signed_out_offline'>('server_sign_out');
                  notify(
                    outcome === 'signed_out_offline'
                      ? tr(
                          "Sei uscito, ma il server non ha risposto: il dispositivo resta nel suo elenco finché non lo disattivi da un altro dispositivo o dall'amministratore.",
                          'Signed out, but the server did not answer: the device stays in its list until you disable it from another device or the administrator does.',
                        )
                      : tr('Sei uscito da questo dispositivo', 'Signed out of this device'),
                  );
                  // Professional needs an account: back to its sign-in screen.
                  if (professional) return window.location.reload();
                  await load();
                })
              }
            >
              {tr('Esci da questo dispositivo', 'Sign out of this device')}
            </Button>
          </div>
          <div className="compute-danger">
            <Button variant="text" className="danger" disabled={busy} onClick={() => setDeleting(true)}>
              {tr('Elimina account', 'Delete account')}
            </Button>
          </div>
        </section>
      )}
      {deleting && (
        <Modal
          size="sm"
          aria-labelledby="delete-account-title"
          onClose={() => (setDeleting(false), setDeletePassword(''))}
          initialFocus={passwordless ? () => keepAccount.current : undefined}
        >
          <h2 id="delete-account-title">{tr("Eliminare l'account?", 'Delete the account?')}</h2>
          <p>
            {tr(
              "Utente, dispositivi e chiavi vengono cancellati dal server. La posta e l'indice su questo computer restano.",
              'User, devices and keys are deleted from the server. Mail and the index on this computer stay.',
            )}
          </p>
          {(() => {
            // A subscription that pays is ended by the server at the processor: say it before the button.
            const warning = deleteWarning(account?.subscription);
            return (
              warning && (
                <div className="compute-alert" data-delete-warning>
                  <p>{tr(warning.it, warning.en)}</p>
                  {account?.subscription?.manageable && (
                    <div className="button-row">
                      <Button variant="secondary" disabled={busy} onClick={() => void run(async () => void (await call('server_open_billing_portal')))}>
                        {tr('Gestisci abbonamento', 'Manage subscription')}
                      </Button>
                    </div>
                  )}
                </div>
              )
            );
          })()}
          {passwordless ? (
            <p className="field-help">
              {tr(
                "L'account non ha una password: per confermare accedi di nuovo con Google nel browser.",
                'The account has no password: to confirm, sign in with Google again in the browser.',
              )}
            </p>
          ) : (
            <Field>
              {tr('Password', 'Password')}
              <input type="password" value={deletePassword} autoComplete="current-password" onChange={(e) => setDeletePassword(e.currentTarget.value)} />
            </Field>
          )}
          {/* The error belongs here while the dialog is open: outside, it is under the inert veil. */}
          {error && (
            <p role="alert" className="index-notice">
              {error}
            </p>
          )}
          <div className="button-row">
            {passwordless && (
              <Button
                variant="primary"
                className="danger"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await call('server_delete_account_browser', { provider: 'google' });
                    setDeleting(false);
                    notify(tr('Account eliminato', 'Account deleted'));
                    if (professional) return window.location.reload();
                    await load();
                  })
                }
              >
                {tr('Conferma con Google ed elimina', 'Confirm with Google and delete')}
              </Button>
            )}
            {!passwordless && (
            <Button
              variant="primary"
              className="danger"
              disabled={busy || !deletePassword}
              onClick={() =>
                void run(async () => {
                  await call('server_delete_account', { password: deletePassword });
                  setDeleting(false);
                  setDeletePassword('');
                  notify(tr('Account eliminato', 'Account deleted'));
                  if (professional) return window.location.reload();
                  await load();
                })
              }
            >
              {tr('Elimina account', 'Delete account')}
            </Button>
            )}
            <Button ref={keepAccount} variant="secondary" onClick={() => (setDeleting(false), setDeletePassword(''))}>
              {tr('Annulla', 'Cancel')}
            </Button>
          </div>
        </Modal>
      )}
      {error && !deleting && (
        <p role="alert" className="index-notice">
          {error}
        </p>
      )}
    </>
  );
}
