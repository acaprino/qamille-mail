// Credentials that do not open on this Windows user (phase 3, Task 7): the transfer passphrase
// chosen on the other computer opens them, or the person signs in to the accounts again. Starting
// over sets the old files aside in the data folder (never deletes them) and always asks first.
// Shown by the start-up notice and by Settings → Accounts.
import { useState, type ReactNode } from 'react';
import Button from '../ds/primitives/Button';
import Field from '../ds/primitives/Field';
import { call, errorText } from '../lib/api';
import { backendText, tr } from '../lib/i18n';
import type { VaultStatus } from '../lib/vault';

type Props = {
  status: Exclude<VaultStatus, { state: 'ready' }>;
  /** The vault changed: the new status and what to tell the person. */
  done: (next: VaultStatus, message: string) => void;
  error: (message: string) => void;
  /** Extra buttons beside the main ones (the notice's «Later»). */
  children?: ReactNode;
};

export default function VaultUnlock({ status, done, error, children }: Props) {
  const [passphrase, setPassphrase] = useState('');
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const transfer = status.state === 'locked' && status.transfer;

  async function run(work: () => Promise<void>) {
    setBusy(true);
    try {
      await work();
    } catch (e) {
      error(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  const unlock = () =>
    run(async () => {
      const next = await call<VaultStatus>('vault_unlock', { passphrase });
      setPassphrase('');
      done(next, tr('Credenziali aperte: ora le protegge Windows per questo utente.', 'Credentials opened: Windows now protects them for this user.'));
    });
  const startOver = () =>
    run(async () => {
      const folder = await call<string>('vault_start_over');
      const next = await call<VaultStatus>('vault_status');
      setConfirming(false);
      done(
        next,
        tr(
          `Le credenziali di prima sono in data\\${folder}. Accedi di nuovo agli account.`,
          `The previous credentials are in data\\${folder}. Sign in to the accounts again.`,
        ),
      );
    });

  if (confirming)
    return (
      <div>
        <p>
          {tr(
            'Le credenziali di prima vanno in una cartella a parte dentro data, senza essere cancellate, e Qamille non le usa più. Poi accedi di nuovo a Google, agli account IMAP e, nella Professional, al server. La posta e le impostazioni restano.',
            'The previous credentials go to a separate folder inside data, without being deleted, and Qamille no longer uses them. Then sign in again to Google, to the IMAP accounts and, in Professional, to the server. Mail and settings stay.',
          )}
        </p>
        <div className="button-row">
          <Button variant="primary" disabled={busy} onClick={() => void startOver()}>
            {busy ? tr('Attendi…', 'Working…') : tr('Accedi di nuovo agli account', 'Sign in to the accounts again')}
          </Button>
          <Button variant="text" disabled={busy} onClick={() => setConfirming(false)}>
            {tr('Annulla', 'Cancel')}
          </Button>
        </div>
      </div>
    );
  return (
    <div>
      {status.state === 'damaged' ? (
        <>
          <p>{tr('Le credenziali salvate non si aprono.', 'The saved credentials do not open.')}</p>
          <p className="field-help">{backendText(status.message)}</p>
        </>
      ) : transfer ? (
        <p>
          {tr(
            'Questa cartella di Qamille viene da un altro utente di Windows o da un altro computer: le credenziali degli account si aprono con la frase di trasferimento scelta là. La posta e le impostazioni ci sono già.',
            'This Qamille folder comes from another Windows user or another computer: the account credentials open with the transfer passphrase chosen there. Mail and settings are already here.',
          )}
        </p>
      ) : (
        <p>
          {tr(
            'La chiave delle credenziali appartiene a un altro utente di Windows o a un altro computer: accedi di nuovo agli account in Impostazioni. La posta e le impostazioni restano.',
            'The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings. Mail and settings stay.',
          )}
        </p>
      )}
      {transfer ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (passphrase) void unlock();
          }}
        >
          <Field>
            {tr('Frase di trasferimento', 'Transfer passphrase')}
            <input
              type="password"
              value={passphrase}
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => setPassphrase(e.currentTarget.value)}
            />
          </Field>
          <div className="button-row">
            <Button variant="primary" type="submit" disabled={busy || !passphrase}>
              {busy ? tr('Attendi…', 'Working…') : tr('Apri le credenziali', 'Open the credentials')}
            </Button>
            <Button
              variant="secondary"
              type="button"
              disabled={busy}
              onClick={() => {
                // The passphrase is not needed any more: drop it.
                setPassphrase('');
                setConfirming(true);
              }}
            >
              {tr('Non la ricordo: accedi di nuovo', 'Forgotten: sign in again')}
            </Button>
            {children}
          </div>
        </form>
      ) : (
        <div className="button-row">
          <Button variant="secondary" disabled={busy} onClick={() => setConfirming(true)}>
            {tr('Accedi di nuovo agli account', 'Sign in to the accounts again')}
          </Button>
          {children}
        </div>
      )}
    </div>
  );
}
