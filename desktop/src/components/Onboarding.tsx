// The sign-in screen of the Professional build (access wave 2, spec A8): a full-window screen with
// «Sign in to Qamille» and «Create account». The Professional app needs an account (owner,
// 2026-11-19): the screen shows at every start without a session and after signing out, and there
// is no way past it without signing in; an unreachable server offers «Retry». It tries the server
// by itself. The window has no frame, so the screen carries the window controls.
import { useEffect, useState } from 'react';
import Button from '../ui/Button';
import { call, desktop } from '../lib/api';
import { tr } from '../lib/i18n';
import type { Probe } from '../lib/server-connect';
import type { AppEdition } from './ComputeSettings';
import ServerConnect from './server-connect/ServerConnect';
import WindowControls from './WindowControls';
import './Onboarding.css';

export type OnboardingProps = {
  edition: AppEdition;
  consented: string[];
  onDone: () => void;
  onerror: (message: string) => void;
};

export default function Onboarding({ edition, consented, onDone, onerror }: OnboardingProps) {
  const [probe, setProbe] = useState<Probe | null>(null);
  const [signingIn, setSigningIn] = useState<'sign_in' | 'sign_up' | null>(null);
  // The card's title names the step: signing in, or creating the account (also when opened from the form).
  const [creating, setCreating] = useState(false);
  const server = edition.server ?? '';

  function tryServer() {
    setProbe(null);
    void call<Probe>('server_probe', { url: server })
      .then(setProbe)
      .catch(() => setProbe({ kind: 'unreachable' }));
  }
  useEffect(tryServer, [server]);
  async function finish() {
    try {
      await call('server_finish_onboarding');
    } finally {
      onDone();
    }
  }
  const found = probe?.kind === 'found';

  return (
    <div className="onboarding">
      {desktop && (
        <header className="startup-window-bar" data-tauri-drag-region="">
          <span data-tauri-drag-region="">Qamille</span>
          <WindowControls onerror={onerror} />
        </header>
      )}
      <main className="onboarding-main">
        <section className="onboarding-card" aria-labelledby="onboarding-title">
          <span className="brand-mark onboarding-mark" aria-hidden="true">
            <svg width="16" height="20" viewBox="0 0 21 25" fill="none">
              <path d="M12 1 2 14h8l-1 10 10-14h-8l1-9Z" fill="currentColor" />
            </svg>
          </span>
          <h1 id="onboarding-title">
            {signingIn && creating ? tr('Crea il tuo account', 'Create your account') : tr('Accedi a Qamille', 'Sign in to Qamille')}
          </h1>
          {!signingIn && (
            <>
              <p role="status" className="muted">
                {probe === null
                  ? tr('Sto provando il server…', 'Trying the server…')
                  : found
                    ? tr('Il server è pronto.', 'The server is ready.')
                    : probe.kind === 'incompatible'
                      ? tr("Versione del server incompatibile: aggiorna l'app.", 'Incompatible server version: update the app.')
                      : tr('Il server non risponde.', 'The server does not answer.')}
              </p>
              <div className="onboarding-actions">
                <Button variant="primary" disabled={!found} onClick={() => setSigningIn('sign_in')}>
                  {tr('Accedi', 'Sign in')}
                </Button>
                {found && probe.info.signup && (
                  <Button variant="secondary" onClick={() => setSigningIn('sign_up')}>
                    {tr('Crea account', 'Create account')}
                  </Button>
                )}
                {probe !== null && !found && (
                  <Button variant="secondary" onClick={tryServer}>
                    {tr('Riprova', 'Retry')}
                  </Button>
                )}
              </div>
            </>
          )}
          {signingIn && (
            <ServerConnect
              preset={server}
              consented={consented}
              defaultDeviceName={edition.default_device_name}
              signUp={signingIn === 'sign_up'}
              titled
              onCreatingChange={setCreating}
              onDone={() => void finish()}
              onCancel={() => setSigningIn(null)}
            />
          )}
        </section>
      </main>
    </div>
  );
}
