// The one guided form that connects this computer to a Qamille server (access wave 2, spec A8):
// for connecting, for «Change server» and for the first start of the Professional build. Its
// transitions are in lib/server-connect.ts; this component makes the calls. The password lives in
// this component's state only while the form is open.
import { useEffect, useRef, useState } from 'react';
import { useFocusAfterStep } from '../../lib/focus-after-step';
import Button from '../../ds/primitives/Button';
import Field from '../../ds/primitives/Field';
import { call, errorText } from '../../lib/api';
import ResetPasswordStep from './ResetPasswordStep';
import SignUpStep from './SignUpStep';
import { consentText } from '../../lib/assistant-server';
import { consentLine } from '../../lib/mailbox-sync';
import { tr } from '../../lib/i18n';
import { acceptedTerms, needsTermsBox, termsBlock } from '../../lib/subscription-usage';
import TermsBox from './TermsBox';
import {
  canLeaveServer,
  cancelled,
  consentGiven,
  deviceDisabled,
  leaveServer,
  LOCAL_SERVER_URL,
  localSuggestion,
  probed,
  signedIn,
  start,
  typeUrl,
  type ConnectForm,
  type ConnectOutcome,
  type Probe,
  type ServerInfo,
} from '../../lib/server-connect';

export type ServerConnectProps = {
  /** The Professional address; absent in the self-hosted build. */
  preset?: string | null;
  consented: string[];
  defaultDeviceName: string;
  /** «Change server»: the session in use stays until the new server accepts the sign-in. */
  changeServer?: boolean;
  /** Opens on «Create account» once consent is given (Professional first start). */
  signUp?: boolean;
  /** «Sign in again» (A8, EX-15): the session's address, the form opens on «Sign in». */
  reconnect?: string | null;
  onDone: () => void;
  onCancel: () => void;
};

function probeText(probe: Probe | null): string {
  if (!probe) return tr('Sto provando…', 'Trying…');
  switch (probe.kind) {
    case 'found':
      return tr(`Server Qamille trovato (versione ${probe.info.version})`, `Qamille server found (version ${probe.info.version})`);
    case 'incompatible':
      return tr(
        `Versione del server incompatibile (${probe.info.version}): aggiorna il server o l'app`,
        `Incompatible server version (${probe.info.version}): update the server or the app`,
      );
    case 'unreachable':
      return tr('Il server non risponde', 'The server does not answer');
    case 'not_qamille':
      return tr('Non sembra un server Qamille', 'This does not look like a Qamille server');
    case 'https_required':
      return tr("L'indirizzo deve iniziare con https://", 'The address must start with https://');
    case 'invalid':
      return tr('Scrivi un indirizzo come https://calcolo.example', 'Write an address such as https://compute.example');
  }
}

export default function ServerConnect({ preset = null, consented, defaultDeviceName, changeServer = false, signUp = false, reconnect = null, onDone, onCancel }: ServerConnectProps) {
  const [form, setForm] = useState<ConnectForm>(() => start({ preset, consented, reconnect }));
  const [deviceName, setDeviceName] = useState(defaultDeviceName);
  const [newPassword, setNewPassword] = useState('');
  const [repeat, setRepeat] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [caPem, setCaPem] = useState('');
  // «Create account» and «Forgot password?» replace the sign-in step while open.
  const [side, setSide] = useState<'signup' | 'reset' | null>(signUp && preset ? 'signup' : null);
  const [presetInfo, setPresetInfo] = useState<ServerInfo | null>(null);
  // «Continue with Google» (spec «Vendita Professional», V3): the browser signs in on the server.
  const [browserWaiting, setBrowserWaiting] = useState(false);
  // The server asks the Terms to be accepted before a Google sign-in may create an account (wave 9).
  const [terms, setTerms] = useState(false);
  // The device list came from a browser sign-in: «Disable» uses its code, not a password.
  const [browserLimit, setBrowserLimit] = useState(false);
  // A fixed address (Professional, or «Sign in again») is asked what it offers by itself.
  const fixedUrl = preset ?? reconnect;
  useEffect(() => {
    if (!fixedUrl) return;
    void call<Probe>('server_probe', { url: fixedUrl })
      .then((result) => live.current && result.kind === 'found' && setPresetInfo(result.info))
      .catch(() => {});
  }, [fixedUrl]);
  const info: ServerInfo | null =
    presetInfo ?? (form.probe?.result?.kind === 'found' ? form.probe.result.info : null);
  // One probe of the server of the Docker guide on this PC, when the step opens empty (wave 2).
  const [localProbe, setLocalProbe] = useState<Probe | null>(null);
  useEffect(() => {
    if (fixedUrl) return;
    void call<Probe>('server_probe', { url: LOCAL_SERVER_URL })
      .then((result) => live.current && setLocalProbe(result))
      .catch(() => {});
  }, [fixedUrl]);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const live = useRef(true);
  useEffect(() => () => {
    live.current = false;
    clearTimeout(timer.current);
    // A CA written here and not used for a sign-in is forgotten when the form closes.
    void call('compute_forget_pending_ca').catch(() => {});
    // A browser sign-in still waiting stops with the form.
    void call('server_cancel_browser_sign_in').catch(() => {});
  }, []);

  function probe(url: string) {
    clearTimeout(timer.current);
    if (!url.trim()) return;
    // About 400 ms after the last keystroke (spec A8).
    timer.current = setTimeout(() => {
      void call<Probe>('server_probe', { url })
        .then((result) => live.current && setForm((current) => probed(current, url, result)))
        .catch((e) => live.current && (setForm((current) => probed(current, url, { kind: 'unreachable' })), setError(errorText(e))));
    }, 400);
  }
  function cancel() {
    setForm((current) => cancelled(current));
    onCancel();
  }
  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await work();
    } catch (e) {
      if (live.current) setError(errorText(e));
    } finally {
      if (live.current) setBusy(false);
    }
  }
  const outcome = (result: ConnectOutcome) => {
    const next = signedIn(form, result);
    setForm(next);
    if (next.step === 'done') {
      setNewPassword('');
      setRepeat('');
      onDone();
    }
  };
  const signIn = () =>
    run(async () => {
      setBrowserLimit(false);
      const args = { url: form.url, identifier: form.identifier, password: form.password, deviceName };
      outcome(await call<ConnectOutcome>(changeServer ? 'server_change_server' : 'server_connect', args));
    });
  const signInWithBrowser = (provider: string) =>
    run(async () => {
      setBrowserWaiting(true);
      try {
        const result = await call<ConnectOutcome>('server_connect_browser', { url: form.url, provider, deviceName, changeServer, acceptedTerms: signUpWithGoogle ? acceptedTerms(info?.legal, terms) : null });
        setBrowserLimit(result.kind === 'device_limit');
        outcome(result);
      } finally {
        if (live.current) setBrowserWaiting(false);
      }
    });
  const google = !!info?.login_providers?.includes('google');
  // «Continue with Google» may create the account: the box comes first, like in «Create account».
  const signUpWithGoogle = !!preset && !!info?.google_signup && needsTermsBox(info?.legal);
  const section = useRef<HTMLElement>(null);
  useFocusAfterStep(section, `${form.step}:${side}:${browserWaiting}`);
  const assistantConsent = consentText(info?.llm);
  const mailboxConsent = consentLine(info);

  return (
    <section ref={section} className="server-connect" aria-label={tr('Collegamento al server', 'Server connection')}>
      {form.step === 'server' && (
        <>
          <h3>{tr('Server', 'Server')}</h3>
          <Field>
            {tr('Indirizzo del server', 'Server address')}
            <input
              value={form.url}
              placeholder="https://calcolo.example"
              autoComplete="url"
              onChange={(e) => {
                const url = e.currentTarget.value;
                setForm((current) => typeUrl(current, url));
                probe(url);
              }}
            />
          </Field>
          {form.url.trim() && (
            <p className="field-help" role="status">
              {probeText(form.probe?.url === form.url ? form.probe.result : null)}
            </p>
          )}
          {!form.preset && !form.url.trim() && (
            <>
              {localSuggestion(form, localProbe) && (
                <div className="button-row">
                  <Button
                    variant="secondary"
                    onClick={() => {
                      setForm((current) => typeUrl(current, LOCAL_SERVER_URL));
                      probe(LOCAL_SERVER_URL);
                    }}
                  >
                    {tr('Server su questo PC (Docker)', 'Server on this PC (Docker)')}
                  </Button>
                </div>
              )}
              <p className="field-help">
                {tr(
                  'Non lo hai? La guida spiega come avviarlo con Docker',
                  "Don't have it? The guide explains how to start it with Docker",
                )}
              </p>
            </>
          )}
          <details className="server-ca">
            <summary>{tr('Autorità di certificazione personalizzata', 'Custom certificate authority')}</summary>
            <Field>
              {tr('Certificato PEM della tua organizzazione', "Your organization's PEM certificate")}
              <textarea value={caPem} rows={4} onChange={(e) => setCaPem(e.currentTarget.value)} />
            </Field>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await call('compute_set_ca_pem', { pem: caPem.trim() || null });
                  probe(form.url);
                })
              }
            >
              {tr('Usa questo certificato', 'Use this certificate')}
            </Button>
          </details>
          <div className="button-row">
            <Button variant="secondary" onClick={() => probe(form.url)} disabled={!form.url.trim()}>
              {tr('Riprova', 'Retry')}
            </Button>
            <Button variant="primary" disabled={!canLeaveServer(form)} onClick={() => setForm(leaveServer(form))}>
              {tr('Avanti', 'Next')}
            </Button>
            <Button variant="text" onClick={cancel}>
              {tr('Annulla', 'Cancel')}
            </Button>
          </div>
        </>
      )}
      {form.step === 'consent' && (
        <>
          <h3>{tr('Prima di collegarti', 'Before you connect')}</h3>
          <p>
            {tr(
              'Testo delle mail, allegati e audio delle riunioni verranno inviati a questo server per il calcolo, che non li conserva.',
              'Mail text, attachments and meeting audio will be sent to this server for computation; it does not keep them.',
            )}
          </p>
          {assistantConsent && <p>{tr(assistantConsent.it, assistantConsent.en)}</p>}
          {mailboxConsent && <p>{tr(mailboxConsent.it, mailboxConsent.en)}</p>}
          <div className="button-row">
            <Button
              variant="primary"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await call('server_record_consent', { url: form.url });
                  setForm(consentGiven(form));
                })
              }
            >
              {tr('Acconsento, continua', 'I agree, continue')}
            </Button>
            <Button variant="text" onClick={cancel}>
              {tr('Annulla', 'Cancel')}
            </Button>
          </div>
        </>
      )}
      {form.step === 'sign_in' && side === 'signup' && (
        <SignUpStep url={form.url} defaultDeviceName={defaultDeviceName} legal={info?.legal} onDone={onDone} onBack={() => setSide(null)} />
      )}
      {form.step === 'sign_in' && side === 'reset' && (
        <ResetPasswordStep
          url={form.url}
          onBack={() => setSide(null)}
          onDone={(email) => {
            setSide(null);
            setForm({ ...form, identifier: email, password: '' });
          }}
        />
      )}
      {form.step === 'sign_in' && !side && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void signIn();
          }}
        >
          <h3>{tr('Accedi', 'Sign in')}</h3>
          {google && (
            <div className="server-browser-sign-in">
              {signUpWithGoogle && !browserWaiting && <TermsBox legal={info!.legal!} checked={terms} onChange={setTerms} onError={setError} />}
              {browserWaiting ? (
                <p role="status" className="field-help">
                  {tr(
                    "Completa l'accesso nella finestra del browser che si è aperta, poi torna qui.",
                    'Finish signing in in the browser window that opened, then come back here.',
                  )}{' '}
                  <Button variant="text" type="button" onClick={() => void call('server_cancel_browser_sign_in')}>
                    {tr('Annulla', 'Cancel')}
                  </Button>
                </p>
              ) : (
                <Button variant="secondary" type="button" disabled={busy || (signUpWithGoogle && termsBlock(info?.legal, terms))} onClick={() => void signInWithBrowser('google')}>
                  {preset && info?.google_signup
                    ? tr('Continua con Google', 'Continue with Google')
                    : tr('Accedi con Google', 'Sign in with Google')}
                </Button>
              )}
              <p className="field-help">{tr('oppure con username e password', 'or with username and password')}</p>
            </div>
          )}
          <Field>
            {tr('Username o email', 'Username or email')}
            <input value={form.identifier} autoComplete="username" onChange={(e) => setForm({ ...form, identifier: e.currentTarget.value })} />
          </Field>
          <Field>
            {tr('Password', 'Password')}
            <input type="password" value={form.password} autoComplete="current-password" onChange={(e) => setForm({ ...form, password: e.currentTarget.value })} />
          </Field>
          <Field>
            {tr('Nome di questo dispositivo', 'Name of this device')}
            <input value={deviceName} onChange={(e) => setDeviceName(e.currentTarget.value)} />
          </Field>
          <div className="button-row">
            <Button variant="primary" type="submit" disabled={busy || !form.identifier.trim() || !form.password}>
              {tr('Accedi', 'Sign in')}
            </Button>
            <Button variant="text" type="button" onClick={cancel}>
              {tr('Annulla', 'Cancel')}
            </Button>
          </div>
          <div className="button-row">
            {/* Sign-up only in the Professional build (INV-17), when the server offers it. */}
            {preset && info?.signup && (
              <Button variant="text" type="button" onClick={() => setSide('signup')}>
                {tr('Crea account', 'Create account')}
              </Button>
            )}
            {info?.password_reset && (
              <Button variant="text" type="button" onClick={() => setSide('reset')}>
                {tr('Password dimenticata?', 'Forgot password?')}
              </Button>
            )}
          </div>
        </form>
      )}
      {form.step === 'new_password' && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (newPassword !== repeat) return setError(tr('Le due password non coincidono.', 'The two passwords differ.'));
            void run(async () => {
              outcome(
                await call<ConnectOutcome>('server_change_temporary_password', {
                  url: form.url,
                  identifier: form.identifier,
                  temporaryPassword: form.password,
                  newPassword,
                  deviceName,
                  changeServer,
                }),
              );
            });
          }}
        >
          <h3>{tr('Nuova password', 'New password')}</h3>
          <p className="field-help">
            {tr(
              "Hai usato una password provvisoria: scegline una tua di almeno 10 caratteri.",
              'You used a temporary password: choose your own, at least 10 characters long.',
            )}
          </p>
          <Field>
            {tr('Nuova password', 'New password')}
            <input type="password" value={newPassword} autoComplete="new-password" onChange={(e) => setNewPassword(e.currentTarget.value)} />
          </Field>
          <Field>
            {tr('Ripeti la nuova password', 'Repeat the new password')}
            <input type="password" value={repeat} autoComplete="new-password" onChange={(e) => setRepeat(e.currentTarget.value)} />
          </Field>
          <div className="button-row">
            <Button variant="primary" type="submit" disabled={busy || newPassword.length < 10}>
              {tr('Cambia ed entra', 'Change and sign in')}
            </Button>
            <Button variant="text" type="button" onClick={cancel}>
              {tr('Annulla', 'Cancel')}
            </Button>
          </div>
        </form>
      )}
      {form.step === 'device_limit' && (
        <>
          <h3>{tr('Troppi dispositivi', 'Too many devices')}</h3>
          <p className="field-help">
            {tr(
              "L'account ha già il massimo di dispositivi: disattivane uno per entrare da qui.",
              'The account already has its maximum of devices: disable one to sign in here.',
            )}
          </p>
          <ul className="server-devices">
            {form.devices.map((device) => (
              <li key={device.id}>
                <span>{device.name}</span>
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      if (browserLimit) {
                        // The browser sign-in's code disables the device and finishes the sign-in.
                        const result = await call<ConnectOutcome>('server_browser_disable_device', { deviceId: device.id });
                        setBrowserLimit(result.kind === 'device_limit');
                        outcome(result);
                        return;
                      }
                      await call('server_disable_device_with_password', {
                        url: form.url,
                        identifier: form.identifier,
                        password: form.password,
                        deviceId: device.id,
                      });
                      setForm(deviceDisabled(form, device.id));
                    })
                  }
                >
                  {tr('Disattiva', 'Disable')}
                </Button>
              </li>
            ))}
          </ul>
          <Button variant="text" onClick={cancel}>
            {tr('Annulla', 'Cancel')}
          </Button>
        </>
      )}
      {error && (
        <p role="alert" className="index-notice">
          {error}
        </p>
      )}
    </section>
  );
}
